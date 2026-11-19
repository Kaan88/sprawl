//! Reading the NTFS master file table directly - WizTree's trick.
//!
//! 1. Open the volume (`\\?\Volume{...}`, needs admin) and read the NTFS boot sector: bytes per
//!    sector, sectors per cluster, where the MFT starts, how big a FILE record is.
//! 2. Read FILE record 0 (`$MFT` itself) for the runlist of the MFT's `$DATA` (plus extension
//!    records named in its `$ATTRIBUTE_LIST`, for a very fragmented MFT).
//! 3. Read the whole MFT front to back in 8 MB chunks on a reader thread (unbuffered, so the
//!    file cache isn't flooded) while a few parser threads decode the records:
//!    update-sequence fixups, in-use flag, `$STANDARD_INFORMATION` (modified time, attributes),
//!    `$FILE_NAME` (parent, name - a Win32 name is preferred over the DOS 8.3 one; with hard
//!    links the file counts once, under its first name), the unnamed `$DATA` (resident length,
//!    or real + allocated size when non-resident). Records that are extensions of another
//!    record (a file with many attributes has an `$ATTRIBUTE_LIST` pointing at them) carry the
//!    base record's number in their header, so their names and sizes go to the base file.
//! 4. Link records to their parents (the root folder is record 5), drop the NTFS metafiles below
//!    record 16 except the root and `$Extend`, and lay the tree out breadth-first from the
//!    scanned folder (any folder on the volume, not just its root).

use std::collections::{HashMap, VecDeque};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::Mutex;

use windows_sys::Win32::Foundation::{CloseHandle, GENERIC_READ, HANDLE, INVALID_HANDLE_VALUE};
use windows_sys::Win32::Storage::FileSystem::{
    CreateFileW, GetFileInformationByHandle, GetVolumeInformationW, GetVolumeNameForVolumeMountPointW, ReadFile, SetFilePointerEx,
    BY_HANDLE_FILE_INFORMATION, FILE_BEGIN, FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_NO_BUFFERING, FILE_FLAG_OPEN_REPARSE_POINT,
    FILE_READ_ATTRIBUTES, FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
};

use crate::progress::{LiveBatch, Progress, ScanMethod};
use crate::walk::{volume_path_name, wide};
use crate::{attr, filetime_to_system, top_of, Node, Tree, ROOT};

/// FRN of the volume's root folder.
pub const ROOT_FRN: u64 = 5;
/// FRN of `$Extend` (kept, like WizTree).
pub const EXTEND_FRN: u64 = 11;
/// Records below this are NTFS metafiles (`$MFT`, `$LogFile`, `$Bitmap`, ...).
pub const FIRST_USER_FRN: u64 = 16;

const CHUNK: usize = 8 << 20;
const BUFFERS: usize = 6;
const FRN_MASK: u64 = 0x0000_FFFF_FFFF_FFFF;

const AT_STANDARD_INFORMATION: u32 = 0x10;
const AT_ATTRIBUTE_LIST: u32 = 0x20;
const AT_FILE_NAME: u32 = 0x30;
const AT_DATA: u32 = 0x80;
const AT_END: u32 = 0xFFFF_FFFF;

const REC_IN_USE: u16 = 0x1;
const REC_DIRECTORY: u16 = 0x2;

// -------------------------------------------------------------------- little-endian helpers

fn u16le(b: &[u8], o: usize) -> Option<u16> {
    b.get(o..o + 2).map(|s| u16::from_le_bytes([s[0], s[1]]))
}
fn u32le(b: &[u8], o: usize) -> Option<u32> {
    b.get(o..o + 4).map(|s| u32::from_le_bytes([s[0], s[1], s[2], s[3]]))
}
fn u64le(b: &[u8], o: usize) -> Option<u64> {
    b.get(o..o + 8).map(|s| u64::from_le_bytes([s[0], s[1], s[2], s[3], s[4], s[5], s[6], s[7]]))
}

// -------------------------------------------------------------------- boot sector

/// The parts of the NTFS boot sector the reader needs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BootSector {
    pub bytes_per_sector: u32,
    pub cluster_size: u64,
    /// First cluster of the MFT.
    pub mft_lcn: u64,
    /// Bytes per FILE record (normally 1024, 4096 on some 4K-native disks).
    pub record_size: u32,
    pub total_sectors: u64,
}

pub fn parse_boot(b: &[u8]) -> Option<BootSector> {
    if b.get(3..11)? != b"NTFS    " {
        return None;
    }
    let bps = u32::from(u16le(b, 0x0B)?);
    if !bps.is_power_of_two() || !(256..=4096).contains(&bps) {
        return None;
    }
    let spc_raw = *b.get(0x0D)?;
    // Values above 0x80 mean 2^(256 - v) sectors (for clusters of 128 KB and more).
    let spc: u64 = if spc_raw <= 0x80 { u64::from(spc_raw) } else { 1u64 << (256 - u32::from(spc_raw)).min(31) };
    if spc == 0 {
        return None;
    }
    let cluster_size = u64::from(bps) * spc;
    let mft_lcn = u64le(b, 0x30)?;
    let cpr = *b.get(0x40)? as i8;
    let record_size: u64 = if cpr > 0 { cpr as u64 * cluster_size } else { 1u64 << u32::from(cpr.unsigned_abs()).min(31) };
    if !record_size.is_power_of_two() || !(256..=65536).contains(&record_size) {
        return None;
    }
    Some(BootSector { bytes_per_sector: bps, cluster_size, mft_lcn, record_size: record_size as u32, total_sectors: u64le(b, 0x28)? })
}

// -------------------------------------------------------------------- runlists

/// One extent of a non-resident attribute: `len` clusters at virtual cluster `vcn` live at
/// logical cluster `lcn` (None: a sparse hole).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Run {
    pub vcn: u64,
    pub lcn: Option<u64>,
    pub len: u64,
}

/// Decode a mapping-pairs array. `start_vcn` is the attribute's lowest VCN.
pub fn decode_runlist(b: &[u8], start_vcn: u64) -> Option<Vec<Run>> {
    let mut runs = Vec::new();
    let mut i = 0usize;
    let mut lcn: i64 = 0;
    let mut vcn = start_vcn;
    while i < b.len() {
        let h = b[i];
        if h == 0 {
            break;
        }
        let ls = usize::from(h & 0x0F);
        let os = usize::from(h >> 4);
        i += 1;
        if ls == 0 || ls > 8 || os > 8 || i + ls + os > b.len() {
            return None;
        }
        let mut len: u64 = 0;
        for k in 0..ls {
            len |= u64::from(b[i + k]) << (8 * k);
        }
        i += ls;
        if os == 0 {
            runs.push(Run { vcn, lcn: None, len });
        } else {
            let mut delta: i64 = 0;
            for k in 0..os {
                delta |= i64::from(b[i + k]) << (8 * k);
            }
            if os < 8 && b[i + os - 1] & 0x80 != 0 {
                delta -= 1i64 << (8 * os); // sign-extend
            }
            lcn = lcn.checked_add(delta)?;
            if lcn < 0 {
                return None;
            }
            runs.push(Run { vcn, lcn: Some(lcn as u64), len });
        }
        i += os;
        vcn = vcn.checked_add(len)?;
    }
    Some(runs)
}

/// Where virtual cluster `vcn` is: (lcn or None for sparse, clusters left in that run).
fn map_vcn(runs: &[Run], vcn: u64) -> Option<(Option<u64>, u64)> {
    let i = runs.partition_point(|r| r.vcn + r.len <= vcn);
    let r = runs.get(i)?;
    if vcn < r.vcn {
        return None;
    }
    let skip = vcn - r.vcn;
    Some((r.lcn.map(|l| l + skip), r.len - skip))
}

// -------------------------------------------------------------------- FILE records

/// Undo the update-sequence protection: the last two bytes of every 512-byte block were
/// replaced by the update sequence number when written; check and restore them.
pub fn apply_fixups(rec: &mut [u8]) -> bool {
    let (Some(off), Some(cnt)) = (u16le(rec, 4), u16le(rec, 6)) else { return false };
    let (off, cnt) = (usize::from(off), usize::from(cnt));
    if cnt < 2 || off + cnt * 2 > rec.len() || (cnt - 1) * 512 > rec.len() {
        return false;
    }
    let usn = [rec[off], rec[off + 1]];
    for i in 1..cnt {
        let end = i * 512;
        if rec[end - 2..end] != usn {
            return false;
        }
        rec[end - 2] = rec[off + 2 * i];
        rec[end - 1] = rec[off + 2 * i + 1];
    }
    true
}

/// Iterator over a FILE record's attributes: (type, the attribute's bytes).
struct Attrs<'a> {
    rec: &'a [u8],
    off: usize,
    end: usize,
}

fn attributes(rec: &[u8]) -> Attrs<'_> {
    let off = u16le(rec, 0x14).map_or(usize::MAX, usize::from);
    let end = u32le(rec, 0x18).map_or(0, |u| (u as usize).min(rec.len()));
    Attrs { rec, off, end }
}

impl<'a> Iterator for Attrs<'a> {
    type Item = (u32, &'a [u8]);
    fn next(&mut self) -> Option<Self::Item> {
        if self.off.checked_add(16)? > self.end {
            return None;
        }
        let t = u32le(self.rec, self.off)?;
        if t == AT_END {
            return None;
        }
        let len = u32le(self.rec, self.off + 4)? as usize;
        if len < 16 || self.off + len > self.end {
            return None;
        }
        let a = &self.rec[self.off..self.off + len];
        self.off += len;
        Some((t, a))
    }
}

fn attr_name_len(a: &[u8]) -> u8 {
    a.get(9).copied().unwrap_or(0)
}
fn is_non_resident(a: &[u8]) -> bool {
    a.get(8).copied().unwrap_or(0) != 0
}
fn resident_value(a: &[u8]) -> Option<&[u8]> {
    let len = u32le(a, 0x10)? as usize;
    let off = usize::from(u16le(a, 0x14)?);
    a.get(off..off.checked_add(len)?)
}
fn runlist_of(a: &[u8]) -> Option<Vec<Run>> {
    let start = u64le(a, 0x10)?;
    let off = usize::from(u16le(a, 0x20)?);
    decode_runlist(a.get(off..)?, start)
}

/// A `$FILE_NAME` picked for a record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileName {
    /// Parent folder's record number.
    pub parent: u64,
    /// Parent's sequence number, to spot names pointing at a reused record.
    pub parent_seq: u16,
    /// 0 = Win32 (or Win32+DOS), 1 = POSIX, 2 = DOS 8.3 only.
    pub rank: u8,
    pub name: String,
}

/// What one in-use FILE record says.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Parsed {
    pub frn: u64,
    pub seq: u16,
    /// Base record this one extends; 0 when it is a base record itself.
    pub base: u64,
    pub is_dir: bool,
    /// `$STANDARD_INFORMATION` was present.
    pub has_std: bool,
    /// FILETIME of the last write.
    pub modified: u64,
    /// `FILE_ATTRIBUTE_*` from `$STANDARD_INFORMATION`.
    pub attrs: u32,
    pub name: Option<FileName>,
    /// Unnamed `$DATA`: (size, allocated). Only from the part that starts at VCN 0.
    pub data: Option<(u64, u64)>,
}

fn name_rank(namespace: u8) -> u8 {
    match namespace {
        1 | 3 => 0, // Win32, Win32 & DOS
        0 => 1,     // POSIX
        _ => 2,     // DOS 8.3
    }
}

/// Decode one FILE record (fixups are applied in place). None for free, damaged or non-FILE
/// records.
pub fn parse_record(rec: &mut [u8], frn: u64) -> Option<Parsed> {
    if rec.get(0..4)? != b"FILE" {
        return None;
    }
    let flags = u16le(rec, 0x16)?;
    if flags & REC_IN_USE == 0 || !apply_fixups(rec) {
        return None;
    }
    let rec = &*rec;
    let mut p = Parsed {
        frn,
        seq: u16le(rec, 0x10)?,
        base: u64le(rec, 0x20)? & FRN_MASK,
        is_dir: flags & REC_DIRECTORY != 0,
        ..Default::default()
    };
    let mut best: Option<(u8, usize, usize, u64)> = None; // rank, name offset, chars, parent ref
    for (t, a) in attributes(rec) {
        match t {
            AT_STANDARD_INFORMATION if !is_non_resident(a) => {
                if let Some(v) = resident_value(a) {
                    p.modified = u64le(v, 0x08).unwrap_or(0);
                    p.attrs = u32le(v, 0x20).unwrap_or(0) | (p.attrs & attr::WOF);
                    p.has_std = true;
                }
            }
            AT_FILE_NAME if !is_non_resident(a) => {
                let Some(v) = resident_value(a) else { continue };
                let (Some(parent), Some(&chars), Some(&ns)) = (u64le(v, 0), v.get(0x40), v.get(0x41)) else { continue };
                let chars = usize::from(chars);
                if v.len() < 0x42 + chars * 2 {
                    continue;
                }
                let rank = name_rank(ns);
                if best.is_none_or(|b| rank < b.0) {
                    let base_off = v.as_ptr() as usize - rec.as_ptr() as usize;
                    best = Some((rank, base_off + 0x42, chars, parent));
                }
            }
            AT_DATA if attr_name_len(a) == 0 => {
                if !is_non_resident(a) {
                    p.data = Some((u64::from(u32le(a, 0x10).unwrap_or(0)), 0));
                } else if u64le(a, 0x10) == Some(0) {
                    let size = u64le(a, 0x30).unwrap_or(0);
                    let mut alloc = u64le(a, 0x28).unwrap_or(0);
                    // Compressed or sparse: what is really allocated is the "compressed size".
                    let aflags = u16le(a, 0x0C).unwrap_or(0);
                    if aflags & 0x8001 != 0 && a.len() >= 0x48 {
                        if let Some(c) = u64le(a, 0x40) {
                            alloc = c;
                        }
                    }
                    p.data = Some((size, alloc));
                }
            }
            0xC0 if !is_non_resident(a) => {
                if resident_value(a).and_then(|v| u32le(v, 0)) == Some(0x8000_0017) { p.attrs |= attr::WOF; }
            }
            _ => {}
        }
    }
    if let Some((rank, off, chars, parent)) = best {
        let units: Vec<u16> = rec[off..off + chars * 2].as_chunks::<2>().0.iter().map(|c| u16::from_le_bytes(*c)).collect();
        p.name = Some(FileName {
            parent: parent & FRN_MASK,
            parent_seq: (parent >> 48) as u16,
            rank,
            name: String::from_utf16_lossy(&units),
        });
    }
    Some(p)
}

// -------------------------------------------------------------------- the index

const NONE32: u32 = u32::MAX;
const F_IN_USE: u8 = 1;
const F_DIR: u8 = 2;

#[derive(Debug, Clone, Copy)]
struct Rec {
    size: u64,
    alloc: u64,
    modified: u64,
    parent: u32,
    attrs: u32,
    seq: u16,
    parent_seq: u16,
    flags: u8,
    /// Name rank * 2 + 1 when the name came from an extension record; 255 = no name yet.
    rank: u8,
}

impl Default for Rec {
    fn default() -> Self {
        Rec { size: 0, alloc: 0, modified: 0, parent: NONE32, attrs: 0, seq: 0, parent_seq: 0, flags: 0, rank: 255 }
    }
}

/// Every record of a volume, indexed by FRN, plus the CSR child lists once linked.
#[derive(Debug, Default)]
pub struct Index {
    recs: Vec<Rec>,
    names: Vec<String>,
}

impl Index {
    pub fn with_capacity(records: usize) -> Self {
        Index { recs: vec![Rec::default(); records], names: vec![String::new(); records] }
    }

    /// Fold one parsed record in. Order doesn't matter.
    pub fn merge(&mut self, p: Parsed) {
        let target = if p.base == 0 { p.frn } else { p.base };
        if target >= u64::from(NONE32) {
            return;
        }
        let t = target as usize;
        if t >= self.recs.len() {
            // The index is sized from the MFT's own record count; a reference past it is a damaged
            // record. Grow a little for safety, never by billions of entries.
            if t > self.recs.len().saturating_mul(2).max(1 << 16) {
                return;
            }
            self.recs.resize(t + 1, Rec::default());
            self.names.resize(t + 1, String::new());
        }
        let r = &mut self.recs[t];
        if p.base == 0 {
            r.flags |= F_IN_USE;
            if p.is_dir {
                r.flags |= F_DIR;
            }
            r.seq = p.seq;
            if p.has_std {
                r.modified = p.modified;
                r.attrs = p.attrs | (r.attrs & attr::WOF);
            }
        }
        r.attrs |= p.attrs & attr::WOF;
        if let Some(n) = p.name {
            let rank = n.rank * 2 + u8::from(p.base != 0);
            if rank < r.rank && n.parent < u64::from(NONE32) {
                r.rank = rank;
                r.parent = n.parent as u32;
                r.parent_seq = n.parent_seq;
                self.names[t] = n.name;
            }
        }
        if let Some((size, alloc)) = p.data {
            r.size = size;
            r.alloc = alloc;
        }
    }

    /// Whether `frn` shows up in trees: in use, named, not a metafile, and its parent link is
    /// current.
    fn listed(&self, frn: usize) -> bool {
        let Some(r) = self.recs.get(frn) else { return false };
        if r.flags & F_IN_USE == 0 || r.rank == 255 || r.parent == NONE32 || r.parent as usize == frn {
            return false;
        }
        if (frn as u64) < FIRST_USER_FRN && frn as u64 != EXTEND_FRN {
            return false;
        }
        match self.recs.get(r.parent as usize) {
            Some(p) => p.flags & F_IN_USE != 0 && p.flags & F_DIR != 0 && (r.parent_seq == 0 || p.seq == r.parent_seq),
            None => false,
        }
    }

    /// Child lists in compressed form: children of `f` are `kids[off[f]..off[f + 1]]`.
    fn link(&self) -> (Vec<u32>, Vec<u32>) {
        let n = self.recs.len();
        let mut off = vec![0u32; n + 1];
        for f in 0..n {
            if self.listed(f) {
                off[self.recs[f].parent as usize + 1] += 1;
            }
        }
        for i in 0..n {
            off[i + 1] += off[i];
        }
        let mut fill = off.clone();
        let mut kids = vec![0u32; off[n] as usize];
        for f in 0..n {
            if self.listed(f) {
                let p = self.recs[f].parent as usize;
                kids[fill[p] as usize] = f as u32;
                fill[p] += 1;
            }
        }
        (off, kids)
    }

    /// Trees for each `(root FRN, path)`; names are moved out when there is one root.
    pub fn build(&mut self, roots: &[(u64, PathBuf)]) -> Vec<Tree> {
        self.build_excluding(roots, &Progress::new())
    }

    /// Build the trees, leaving out every scope `progress` excludes (and all below it). Paths
    /// are made only for folders holding an excluded scope, so this costs nothing without one.
    pub fn build_excluding(&mut self, roots: &[(u64, PathBuf)], progress: &Progress) -> Vec<Tree> {
        let (off, kids) = self.link();
        let take = roots.len() == 1;
        let mut out = Vec::with_capacity(roots.len());
        for (root_frn, path) in roots {
            out.push(self.build_one(*root_frn as usize, path, &off, &kids, take, progress));
        }
        out
    }

    fn node_of(&mut self, frn: usize, parent: Option<usize>, take: bool) -> Node {
        let r = self.recs[frn];
        let is_dir = r.flags & F_DIR != 0;
        let name = if take { std::mem::take(&mut self.names[frn]) } else { self.names[frn].clone() };
        Node {
            name,
            is_dir,
            size: if is_dir { 0 } else { r.size },
            allocated: if is_dir { 0 } else { r.alloc },
            files: u64::from(!is_dir),
            folders: 0,
            modified: filetime_to_system(r.modified),
            attributes: if is_dir { r.attrs | attr::DIRECTORY } else { r.attrs },
            parent,
            children: Vec::new(),
        }
    }

    fn build_one(&mut self, root: usize, path: &Path, off: &[u32], kids: &[u32], take: bool, progress: &Progress) -> Tree {
        let mut nodes: Vec<Node> = Vec::new();
        if root < self.recs.len() {
            let mut r = self.node_of(root, None, take);
            r.name = path.display().to_string();
            nodes.push(r);
        } else {
            nodes.push(Node { name: path.display().to_string(), is_dir: true, attributes: attr::DIRECTORY, ..Default::default() });
        }
        let inside = |p: &Path| progress.exclusion_inside(p).then(|| p.to_path_buf());
        let mut queue: VecDeque<(usize, usize, Option<PathBuf>)> = VecDeque::from([(ROOT, root, inside(path))]);
        while let Some((ni, frn, here)) = queue.pop_front() {
            if frn + 1 >= off.len() {
                continue;
            }
            let list = &kids[off[frn] as usize..off[frn + 1] as usize];
            let mut children = Vec::with_capacity(list.len());
            for &k in list {
                let k = k as usize;
                // Only below a folder that holds an excluded scope can a child be one.
                let child = here.as_ref().map(|h| h.join(&self.names[k]));
                if child.as_deref().is_some_and(|c| progress.excludes(c)) { continue; }
                let idx = nodes.len();
                children.push(idx);
                let node = self.node_of(k, Some(ni), take);
                let dir = node.is_dir;
                nodes.push(node);
                if dir {
                    queue.push_back((idx, k, child.as_deref().and_then(inside)));
                }
            }
            nodes[ni].children = children;
        }
        let mut tree = Tree { root_path: path.to_path_buf(), nodes, errors: 0, method: ScanMethod::Mft };
        tree.compute_totals();
        tree
    }

    /// Display path of `frn` (best effort, for the progress line).
    fn path_hint(&self, frn: usize) -> String {
        let mut parts: Vec<&str> = Vec::new();
        let mut cur = frn;
        for _ in 0..64 {
            let Some(r) = self.recs.get(cur) else { break };
            if cur as u64 == ROOT_FRN || r.parent == NONE32 {
                break;
            }
            parts.push(&self.names[cur]);
            cur = r.parent as usize;
        }
        parts.reverse();
        parts.join("\\")
    }
}

/// Finds which direct child of the scan root a file is under, while the MFT is still being read
/// (for the live "top" bars). Folders on the way are memoised.
struct TopTracker {
    root: usize,
    memo: Vec<u32>,
    slots: HashMap<u32, usize>,
    path: Vec<u32>,
}

const T_UNKNOWN: u32 = u32::MAX;
const T_OUTSIDE: u32 = u32::MAX - 1;

impl TopTracker {
    fn new(root: u64, records: usize) -> Self {
        TopTracker { root: root as usize, memo: vec![T_UNKNOWN; records], slots: HashMap::new(), path: Vec::new() }
    }

    fn child_for(&mut self, idx: &Index, file: usize) -> Option<u32> {
        self.path.clear();
        let mut cur = file;
        let result;
        loop {
            if cur != file {
                if cur >= self.memo.len() {
                    return None;
                }
                if self.memo[cur] != T_UNKNOWN {
                    result = self.memo[cur];
                    break;
                }
                self.path.push(cur as u32);
            }
            let r = idx.recs.get(cur)?;
            if r.flags & F_IN_USE == 0 || r.parent == NONE32 || self.path.len() > 512 {
                return None;
            }
            let p = r.parent as usize;
            if p == self.root {
                result = cur as u32;
                break;
            }
            if cur as u64 == ROOT_FRN || p == cur {
                result = T_OUTSIDE;
                break;
            }
            cur = p;
        }
        for &d in &self.path {
            self.memo[d as usize] = result;
        }
        if result == T_OUTSIDE {
            return None;
        }
        Some(result)
    }

    fn slot_for(&mut self, idx: &Index, file: usize, progress: &Progress) -> Option<usize> {
        let result = self.child_for(idx, file)?;
        if let Some(&s) = self.slots.get(&result) {
            return Some(s);
        }
        let name = idx.names.get(result as usize).cloned().unwrap_or_default();
        let s = progress.top_push(name, 0);
        self.slots.insert(result, s);
        Some(s)
    }

    /// Parser chunks arrive in any order; unresolved live contributions are not retried on
    /// every chunk. Reconcile from merged records once all ancestors/extensions are present.
    fn final_totals(&mut self, idx: &Index) -> Vec<(String, u64)> {
        self.memo.resize(idx.recs.len(), T_UNKNOWN);
        self.memo.fill(T_UNKNOWN);
        let mut totals: HashMap<u32, u64> = HashMap::new();
        for (file, rec) in idx.recs.iter().enumerate() {
            if rec.flags & F_DIR != 0 || !idx.listed(file) { continue; }
            if let Some(top) = self.child_for(idx, file) { *totals.entry(top).or_default() += rec.size; }
        }
        totals.into_iter().map(|(file, bytes)| (idx.names[file as usize].clone(), bytes)).collect()
    }
}

// -------------------------------------------------------------------- volume access

/// The volume root (`C:\`, or a mount folder) `path` lives on.
pub fn volume_of(path: &Path) -> Option<PathBuf> {
    let v = volume_path_name(path)?;
    Some(PathBuf::from(String::from_utf16_lossy(&v[..v.len() - 1])))
}

fn file_system_name(volume: &Path) -> Option<String> {
    let mut root = wide(volume.as_os_str());
    if root.len() >= 2 && root[root.len() - 2] != u16::from(b'\\') {
        root.insert(root.len() - 1, u16::from(b'\\'));
    }
    let mut fs = [0u16; 64];
    // SAFETY: NUL-terminated input; output buffer with its size.
    let ok = unsafe {
        GetVolumeInformationW(
            root.as_ptr(),
            std::ptr::null_mut(),
            0,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            fs.as_mut_ptr(),
            fs.len() as u32,
        )
    };
    if ok == 0 {
        return None;
    }
    let n = fs.iter().position(|&c| c == 0).unwrap_or(fs.len());
    Some(String::from_utf16_lossy(&fs[..n]))
}

/// True when `path` is on an NTFS volume.
pub fn is_ntfs(path: &Path) -> bool {
    volume_of(path).and_then(|v| file_system_name(&v)).is_some_and(|f| f.eq_ignore_ascii_case("NTFS"))
}

/// The device path to open for a volume root: `\\?\Volume{guid}` (no trailing backslash).
fn device_of(volume: &Path) -> Vec<u16> {
    let mut root = wide(volume.as_os_str());
    if root.len() >= 2 && root[root.len() - 2] != u16::from(b'\\') {
        root.insert(root.len() - 1, u16::from(b'\\'));
    }
    let mut buf = [0u16; 128];
    // SAFETY: NUL-terminated input; output buffer with its size.
    if unsafe { GetVolumeNameForVolumeMountPointW(root.as_ptr(), buf.as_mut_ptr(), buf.len() as u32) } != 0 {
        let mut n = buf.iter().position(|&c| c == 0).unwrap_or(0);
        if n > 0 && buf[n - 1] == u16::from(b'\\') {
            n -= 1;
        }
        let mut v = buf[..n].to_vec();
        v.push(0);
        return v;
    }
    // Fall back to \\.\C:
    let s = volume.display().to_string();
    wide(format!(r"\\.\{}", s.trim_end_matches('\\')).as_ref())
}

struct Handle(HANDLE);
// SAFETY: a file handle can be used from any thread; reads are issued by one thread at a time.
unsafe impl Send for Handle {}
// SAFETY: as above; `read_at` takes &self but the reader thread is the only caller.
unsafe impl Sync for Handle {}
impl Drop for Handle {
    fn drop(&mut self) {
        // SAFETY: a valid handle, closed once.
        unsafe { CloseHandle(self.0) };
    }
}

fn open(path: &[u16], access: u32, flags: u32) -> io::Result<Handle> {
    // SAFETY: NUL-terminated path; the handle is owned by `Handle`.
    let h = unsafe {
        CreateFileW(
            path.as_ptr(),
            access,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            std::ptr::null(),
            OPEN_EXISTING,
            flags,
            std::ptr::null_mut(),
        )
    };
    if h == INVALID_HANDLE_VALUE {
        Err(io::Error::last_os_error())
    } else {
        Ok(Handle(h))
    }
}

impl Handle {
    /// Read exactly `buf.len()` bytes at `offset` (sector-aligned for an unbuffered volume).
    fn read_at(&self, offset: u64, buf: &mut [u8]) -> io::Result<()> {
        // SAFETY: valid handle and buffer.
        if unsafe { SetFilePointerEx(self.0, offset as i64, std::ptr::null_mut(), FILE_BEGIN) } == 0 {
            return Err(io::Error::last_os_error());
        }
        let mut done = 0usize;
        while done < buf.len() {
            let want = (buf.len() - done).min(1 << 30) as u32;
            let mut got = 0u32;
            // SAFETY: valid handle; the destination range is inside `buf`.
            if unsafe { ReadFile(self.0, buf[done..].as_mut_ptr(), want, &mut got, std::ptr::null_mut()) } == 0 {
                return Err(io::Error::last_os_error());
            }
            if got == 0 {
                return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "short read from volume"));
            }
            done += got as usize;
        }
        Ok(())
    }
}

/// FRN, sequence number and attributes of a file or folder.
fn file_id(path: &Path) -> io::Result<(u64, u16, u32)> {
    let h = open(&wide(path.as_os_str()), FILE_READ_ATTRIBUTES, FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT)?;
    // SAFETY: zeroed POD out-struct; valid handle.
    let mut info: BY_HANDLE_FILE_INFORMATION = unsafe { std::mem::zeroed() };
    if unsafe { GetFileInformationByHandle(h.0, &mut info) } == 0 {
        return Err(io::Error::last_os_error());
    }
    let id = (u64::from(info.nFileIndexHigh) << 32) | u64::from(info.nFileIndexLow);
    Ok((id & FRN_MASK, (id >> 48) as u16, info.dwFileAttributes))
}

/// Sector-aligned heap buffer (unbuffered I/O needs aligned memory).
struct AlignedBuf {
    ptr: std::ptr::NonNull<u8>,
    len: usize,
}
// SAFETY: owns its allocation exclusively.
unsafe impl Send for AlignedBuf {}
impl AlignedBuf {
    fn new(len: usize) -> Self {
        let layout = std::alloc::Layout::from_size_align(len.max(4096), 4096).expect("layout");
        // SAFETY: non-zero size.
        let p = unsafe { std::alloc::alloc_zeroed(layout) };
        AlignedBuf { ptr: std::ptr::NonNull::new(p).unwrap_or_else(|| std::alloc::handle_alloc_error(layout)), len: len.max(4096) }
    }
}
impl std::ops::Deref for AlignedBuf {
    type Target = [u8];
    fn deref(&self) -> &[u8] {
        // SAFETY: valid for `len` bytes, initialised (zeroed at allocation).
        unsafe { std::slice::from_raw_parts(self.ptr.as_ptr(), self.len) }
    }
}
impl std::ops::DerefMut for AlignedBuf {
    fn deref_mut(&mut self) -> &mut [u8] {
        // SAFETY: as above, uniquely borrowed.
        unsafe { std::slice::from_raw_parts_mut(self.ptr.as_ptr(), self.len) }
    }
}
impl Drop for AlignedBuf {
    fn drop(&mut self) {
        // SAFETY: same layout as allocated.
        unsafe { std::alloc::dealloc(self.ptr.as_ptr(), std::alloc::Layout::from_size_align_unchecked(self.len, 4096)) };
    }
}

/// The MFT's own extents and size, from record 0.
fn mft_layout(vol: &Handle, boot: &BootSector) -> io::Result<(Vec<Run>, u64)> {
    let bad = |m: &str| io::Error::new(io::ErrorKind::InvalidData, m.to_string());
    let rs = boot.record_size as usize;
    let read_len = rs.div_ceil(4096) * 4096;
    let mut buf = AlignedBuf::new(read_len);
    vol.read_at(boot.mft_lcn * boot.cluster_size, &mut buf[..read_len])?;
    let rec = &mut buf[..rs];
    if rec.get(0..4) != Some(b"FILE") || !apply_fixups(rec) {
        return Err(bad("MFT record 0 is damaged"));
    }
    let mut runs: Vec<Run> = Vec::new();
    let mut size = None;
    let mut ext: Vec<u64> = Vec::new();
    for (t, a) in attributes(rec) {
        if t == AT_DATA && attr_name_len(a) == 0 && is_non_resident(a) {
            if u64le(a, 0x10) == Some(0) {
                size = u64le(a, 0x30);
            }
            runs.extend(runlist_of(a).ok_or_else(|| bad("bad MFT runlist"))?);
        } else if t == AT_ATTRIBUTE_LIST {
            if is_non_resident(a) {
                return Err(bad("non-resident $ATTRIBUTE_LIST on $MFT"));
            }
            let v = resident_value(a).ok_or_else(|| bad("bad $ATTRIBUTE_LIST"))?;
            let mut o = 0usize;
            while o + 0x1A <= v.len() {
                let (ty, len) = (u32le(v, o).unwrap_or(0), usize::from(u16le(v, o + 4).unwrap_or(0)));
                if len == 0 {
                    break;
                }
                let r = u64le(v, o + 0x10).unwrap_or(0) & FRN_MASK;
                if ty == AT_DATA && v.get(o + 6) == Some(&0) && r != 0 && !ext.contains(&r) {
                    ext.push(r);
                }
                o += len;
            }
        }
    }
    // Extension records of $MFT hold the rest of its runlist.
    for r in ext {
        runs.sort_by_key(|r| r.vcn);
        let byte = r * rs as u64;
        let (Some(lcn), _) = map_vcn(&runs, byte / boot.cluster_size).ok_or_else(|| bad("MFT extension record out of range"))? else {
            return Err(bad("MFT extension record in a hole"));
        };
        let disk = lcn * boot.cluster_size + byte % boot.cluster_size;
        let aligned = disk / 4096 * 4096;
        let mut b = AlignedBuf::new(read_len + 4096);
        vol.read_at(aligned, &mut b[..read_len + 4096])?;
        let s = (disk - aligned) as usize;
        let rec = &mut b[s..s + rs];
        if rec.get(0..4) != Some(b"FILE") || !apply_fixups(rec) {
            return Err(bad("MFT extension record is damaged"));
        }
        for (t, a) in attributes(rec) {
            if t == AT_DATA && attr_name_len(a) == 0 && is_non_resident(a) {
                if u64le(a, 0x10) == Some(0) {
                    size = u64le(a, 0x30);
                }
                runs.extend(runlist_of(a).ok_or_else(|| bad("bad MFT runlist"))?);
            }
        }
    }
    runs.sort_by_key(|r| r.vcn);
    let size = size.ok_or_else(|| bad("no $DATA in MFT record 0"))?;
    Ok((runs, size))
}

/// What a parser thread hands back for one chunk.
struct ChunkResult {
    parsed: Vec<Parsed>,
    batch: LiveBatch,
    bytes: u64,
}

/// Read and parse the whole MFT of the volume at `volume` (e.g. `C:\`).
/// `live_root`: FRN whose direct children get live sizes in the progress.
pub fn read_mft(volume: &Path, progress: &Progress, live_root: Option<u64>) -> io::Result<Index> {
    let label = volume.display().to_string();
    read_mft_device(&device_of(volume), label.trim_end_matches('\\'), progress, live_root, CHUNK)
}

/// [`read_mft`] from a device path (or, in tests, an NTFS image file). `label` prefixes the
/// paths shown as progress; `chunk_size` must be a multiple of the cluster and record size.
fn read_mft_device(device: &[u16], label: &str, progress: &Progress, live_root: Option<u64>, chunk_size: usize) -> io::Result<Index> {
    let vol = open(device, GENERIC_READ, FILE_FLAG_NO_BUFFERING)?;
    let mut boot_buf = AlignedBuf::new(4096);
    vol.read_at(0, &mut boot_buf[..4096])?;
    let boot = parse_boot(&boot_buf).ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "not an NTFS boot sector"))?;
    let (runs, mft_bytes) = mft_layout(&vol, &boot)?;
    let rs = boot.record_size as u64;
    let cluster = boot.cluster_size;
    let records = (mft_bytes / rs) as usize;
    progress.set_units(0, mft_bytes);
    let chunk = (chunk_size as u64).max(cluster).max(rs);
    let chunks = mft_bytes.div_ceil(chunk);

    let mut index = Index::with_capacity(records);
    let mut top = live_root.map(|r| TopTracker::new(r, records));
    let stop = AtomicBool::new(false);
    let parsers = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4).clamp(2, 8);

    let (free_tx, free_rx) = mpsc::sync_channel::<AlignedBuf>(BUFFERS);
    for _ in 0..BUFFERS {
        let _ = free_tx.send(AlignedBuf::new(chunk as usize));
    }
    let (work_tx, work_rx) = mpsc::sync_channel::<(AlignedBuf, u64, usize)>(BUFFERS);
    let work_rx = Mutex::new(work_rx);
    let (res_tx, res_rx) = mpsc::channel::<io::Result<ChunkResult>>();

    let outcome: io::Result<()> = std::thread::scope(|s| {
        // Reader: one big sequential read per chunk, following the MFT's extents.
        let reader_res = res_tx.clone();
        let (stop_r, runs_r, vol_r) = (&stop, &runs, &vol);
        s.spawn(move || {
            let work_tx = work_tx;
            let mut off = 0u64;
            while off < mft_bytes && !stop_r.load(Ordering::Relaxed) {
                let Ok(mut buf) = free_rx.recv() else { break };
                let len = (mft_bytes - off).min(chunk);
                let read_len = len.div_ceil(cluster) * cluster;
                let mut pos = 0u64;
                while pos < read_len {
                    let vcn = (off + pos) / cluster;
                    let Some((lcn, left)) = map_vcn(runs_r, vcn) else {
                        let _ = reader_res.send(Err(io::Error::new(io::ErrorKind::InvalidData, "MFT runlist too short")));
                        return;
                    };
                    let n = (left * cluster).min(read_len - pos);
                    let dst = &mut buf[pos as usize..(pos + n) as usize];
                    match lcn {
                        Some(l) => {
                            if let Err(e) = vol_r.read_at(l * cluster, dst) {
                                let _ = reader_res.send(Err(e));
                                return;
                            }
                        }
                        None => dst.fill(0),
                    }
                    pos += n;
                }
                if work_tx.send((buf, off / rs, len as usize)).is_err() {
                    break;
                }
                off += len;
            }
        });
        // Parsers.
        for _ in 0..parsers {
            let (work_rx, res_tx, free_tx) = (&work_rx, res_tx.clone(), free_tx.clone());
            s.spawn(move || loop {
                let job = work_rx.lock().unwrap_or_else(|e| e.into_inner()).recv();
                let Ok((mut buf, first, len)) = job else { break };
                let mut parsed = Vec::with_capacity(len / rs as usize);
                let mut batch = LiveBatch::default();
                for (i, rec) in buf[..len].chunks_exact_mut(rs as usize).enumerate() {
                    if let Some(p) = parse_record(rec, first + i as u64) {
                        if p.base == 0 && !p.is_dir && p.frn >= FIRST_USER_FRN {
                            if let (Some(n), Some((size, _))) = (&p.name, p.data) {
                                batch.add_file(&n.name, size);
                            }
                        }
                        parsed.push(p);
                    }
                }
                let _ = free_tx.send(buf);
                if res_tx.send(Ok(ChunkResult { parsed, batch, bytes: len as u64 })).is_err() {
                    break;
                }
            });
        }
        drop(res_tx);
        // Merge on this thread.
        let mut received = 0u64;
        let mut sizes: Vec<(usize, u64)> = Vec::new();
        while received < chunks {
            if progress.is_cancelled() {
                stop.store(true, Ordering::Relaxed);
                return Err(io::Error::new(io::ErrorKind::Interrupted, "scan cancelled"));
            }
            let res = match res_rx.recv_timeout(std::time::Duration::from_millis(50)) {
                Ok(r) => r,
                Err(mpsc::RecvTimeoutError::Timeout) => continue,
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    stop.store(true, Ordering::Relaxed);
                    return Err(io::Error::other("MFT reader stopped early"));
                }
            };
            let mut chunk_res = match res {
                Ok(c) => c,
                Err(e) => {
                    stop.store(true, Ordering::Relaxed);
                    return Err(e);
                }
            };
            received += 1;
            let (mut files, mut folders, mut bytes) = (0u64, 0u64, 0u64);
            sizes.clear();
            let mut last_file = None;
            for p in chunk_res.parsed.drain(..) {
                let target = if p.base == 0 { p.frn } else { p.base } as usize;
                if p.base == 0 && p.frn >= FIRST_USER_FRN {
                    if p.is_dir {
                        folders += 1;
                    } else {
                        files += 1;
                        last_file = Some(target);
                    }
                }
                if let Some((size, _)) = p.data {
                    bytes += size;
                    if size > 0 {
                        sizes.push((target, size));
                    }
                }
                index.merge(p);
            }
            if let Some(t) = top.as_mut() {
                for &(f, size) in &sizes {
                    if let Some(slot) = t.slot_for(&index, f, progress) {
                        chunk_res.batch.add_top(slot, size);
                    }
                }
            }
            if let Some(f) = last_file.filter(|_| !progress.has_exclusions()) {
                chunk_res.batch.current = Some(format!("{label}\\{}", index.path_hint(f)));
            }
            progress.add_counts(files, folders, bytes);
            progress.merge(&mut chunk_res.batch);
            progress.add_done_units(chunk_res.bytes);
        }
        Ok(())
    });
    outcome?;
    if let Some(top) = &mut top { progress.top_reset(top.final_totals(&index)); }
    Ok(index)
}

/// MFT scan of one folder (see [`scan_multi`]).
pub fn scan(path: &Path, progress: &Progress) -> io::Result<Tree> {
    let mut v = scan_multi(&[path.to_path_buf()], progress)?;
    v.pop().ok_or_else(|| io::Error::other("no tree"))
}

/// MFT scan of several folders on the same NTFS volume (the MFT is read once). Fails when the
/// volume isn't NTFS, can't be opened (not elevated), or a path isn't a plain folder - callers
/// fall back to [`crate::walk::walk`].
pub fn scan_multi(paths: &[PathBuf], progress: &Progress) -> io::Result<Vec<Tree>> {
    let unsupported = |m: &str| io::Error::new(io::ErrorKind::Unsupported, m.to_string());
    let first = paths.first().ok_or_else(|| unsupported("no paths"))?;
    let volume = volume_of(first).ok_or_else(|| unsupported("no volume"))?;
    if !file_system_name(&volume).is_some_and(|f| f.eq_ignore_ascii_case("NTFS")) {
        return Err(unsupported("not NTFS"));
    }
    let mut roots = Vec::with_capacity(paths.len());
    for p in paths {
        if volume_of(p).as_deref() != Some(volume.as_path()) {
            return Err(unsupported("paths on different volumes"));
        }
        let (frn, _, attrs) = file_id(p)?;
        if attrs & attr::DIRECTORY == 0 || attrs & attr::REPARSE_POINT != 0 {
            return Err(unsupported("not a plain folder"));
        }
        if progress.excludes(p) {
            return Err(io::Error::new(io::ErrorKind::PermissionDenied, "This folder is excluded in Settings > Scanning"));
        }
        roots.push((frn, p.clone()));
    }
    progress.set_method(ScanMethod::Mft);
    progress.set_phase("Reading MFT");
    // With skipped folders, the live list of top folders and the current-file line would show
    // what is skipped while the whole MFT is read: leave both out until the tree is built.
    let live = if roots.len() == 1 && !progress.has_exclusions() { Some(roots[0].0) } else { None };
    let mut index = read_mft(&volume, progress, live)?;
    progress.set_phase("Building tree");
    let trees = index.build_excluding(&roots, progress);
    drop(index);
    if let [t] = trees.as_slice() {
        let r = &t.nodes[ROOT];
        progress.set_counts(r.files, r.folders, r.size);
        progress.top_reset(top_of(t));
    }
    progress.set_phase("Done");
    Ok(trees)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Builds FILE records the way NTFS writes them (attributes, then the update sequence).
    struct RecBuilder {
        buf: Vec<u8>,
        off: usize,
        next_id: u16,
    }

    impl RecBuilder {
        fn new(size: usize, seq: u16, flags: u16, base: u64) -> Self {
            let mut buf = vec![0u8; size];
            buf[0..4].copy_from_slice(b"FILE");
            let cnt = (size / 512 + 1) as u16;
            buf[4..6].copy_from_slice(&0x30u16.to_le_bytes());
            buf[6..8].copy_from_slice(&cnt.to_le_bytes());
            buf[0x10..0x12].copy_from_slice(&seq.to_le_bytes());
            buf[0x12..0x14].copy_from_slice(&1u16.to_le_bytes());
            let first = (0x30 + cnt as usize * 2).div_ceil(8) * 8;
            buf[0x14..0x16].copy_from_slice(&(first as u16).to_le_bytes());
            buf[0x16..0x18].copy_from_slice(&flags.to_le_bytes());
            buf[0x1C..0x20].copy_from_slice(&(size as u32).to_le_bytes());
            buf[0x20..0x28].copy_from_slice(&base.to_le_bytes());
            RecBuilder { buf, off: first, next_id: 0 }
        }

        fn attr(&mut self, ty: u32, non_resident: bool, name: &str, body: &[u8]) {
            // body = everything after the common 16-byte header
            let name16: Vec<u8> = name.encode_utf16().flat_map(|c| c.to_le_bytes()).collect();
            let name_off = 16 + body.len();
            let len = (name_off + name16.len()).div_ceil(8) * 8;
            let a = &mut self.buf[self.off..self.off + len];
            a[0..4].copy_from_slice(&ty.to_le_bytes());
            a[4..8].copy_from_slice(&(len as u32).to_le_bytes());
            a[8] = u8::from(non_resident);
            a[9] = (name16.len() / 2) as u8;
            a[0x0A..0x0C].copy_from_slice(&(name_off as u16).to_le_bytes());
            a[0x0E..0x10].copy_from_slice(&self.next_id.to_le_bytes());
            a[16..16 + body.len()].copy_from_slice(body);
            a[name_off..name_off + name16.len()].copy_from_slice(&name16);
            self.off += len;
            self.next_id += 1;
        }

        fn resident(&mut self, ty: u32, value: &[u8]) {
            let mut body = vec![0u8; 8];
            body[0..4].copy_from_slice(&(value.len() as u32).to_le_bytes());
            body[4..6].copy_from_slice(&24u16.to_le_bytes());
            body.extend_from_slice(value);
            self.attr(ty, false, "", &body);
        }

        fn std_info(&mut self, modified: u64, attrs: u32) {
            let mut v = vec![0u8; 0x48];
            v[8..16].copy_from_slice(&modified.to_le_bytes());
            v[0x20..0x24].copy_from_slice(&attrs.to_le_bytes());
            self.resident(AT_STANDARD_INFORMATION, &v);
        }

        fn file_name(&mut self, parent: u64, parent_seq: u16, ns: u8, name: &str) {
            let units: Vec<u16> = name.encode_utf16().collect();
            let mut v = vec![0u8; 0x42];
            v[0..8].copy_from_slice(&(parent | (u64::from(parent_seq) << 48)).to_le_bytes());
            v[0x40] = units.len() as u8;
            v[0x41] = ns;
            for u in units {
                v.extend_from_slice(&u.to_le_bytes());
            }
            self.resident(AT_FILE_NAME, &v);
        }

        fn data_nonresident(&mut self, name: &str, start_vcn: u64, size: u64, alloc: u64, flags: u16, runlist: &[u8]) {
            // non-resident header from 0x10 to 0x40 (or 0x48 with compressed size)
            let hdr = if flags != 0 { 0x48 } else { 0x40 };
            let mut body = vec![0u8; hdr - 16];
            let w = |b: &mut Vec<u8>, at: usize, v: &[u8]| b[at - 16..at - 16 + v.len()].copy_from_slice(v);
            w(&mut body, 0x10, &start_vcn.to_le_bytes());
            w(&mut body, 0x20, &(hdr as u16).to_le_bytes()); // runlist right after the header... adjusted below
            w(&mut body, 0x28, &alloc.to_le_bytes());
            w(&mut body, 0x30, &size.to_le_bytes());
            w(&mut body, 0x38, &size.to_le_bytes());
            if flags != 0 {
                w(&mut body, 0x40, &(alloc / 2).to_le_bytes());
            }
            body.extend_from_slice(runlist);
            body.push(0);
            let start = self.off;
            self.attr(AT_DATA, true, name, &body);
            let a = &mut self.buf[start..];
            a[0x0C..0x0E].copy_from_slice(&flags.to_le_bytes());
        }

        fn finish(mut self, usn: u16) -> Vec<u8> {
            let off = self.off;
            self.buf[off..off + 4].copy_from_slice(&AT_END.to_le_bytes());
            let used = (off + 8) as u32;
            self.buf[0x18..0x1C].copy_from_slice(&used.to_le_bytes());
            let cnt = self.buf.len() / 512 + 1;
            self.buf[0x30..0x32].copy_from_slice(&usn.to_le_bytes());
            for i in 1..cnt {
                let end = i * 512;
                let (a, b) = (self.buf[end - 2], self.buf[end - 1]);
                self.buf[0x30 + 2 * i] = a;
                self.buf[0x30 + 2 * i + 1] = b;
                self.buf[end - 2..end].copy_from_slice(&usn.to_le_bytes());
            }
            self.buf
        }
    }

    #[test]
    fn boot_sector() {
        let mut b = vec![0u8; 512];
        b[3..11].copy_from_slice(b"NTFS    ");
        b[0x0B..0x0D].copy_from_slice(&512u16.to_le_bytes());
        b[0x0D] = 8;
        b[0x28..0x30].copy_from_slice(&1_000_000u64.to_le_bytes());
        b[0x30..0x38].copy_from_slice(&786_432u64.to_le_bytes());
        b[0x40] = 0xF6; // -10 -> 1024-byte records
        let bs = parse_boot(&b).unwrap();
        assert_eq!(bs, BootSector { bytes_per_sector: 512, cluster_size: 4096, mft_lcn: 786_432, record_size: 1024, total_sectors: 1_000_000 });
        b[0x0D] = 0xF4; // 2^12 sectors per cluster: 2 MB clusters
        assert_eq!(parse_boot(&b).unwrap().cluster_size, 512 * 4096);
        b[0x0D] = 8;
        b[0x40] = 1; // one cluster per record
        assert_eq!(parse_boot(&b).unwrap().record_size, 4096);
        b[0x0D] = 0xF4;
        assert!(parse_boot(&b).is_none()); // a 2 MB record is nonsense
        b[3] = b'X';
        assert!(parse_boot(&b).is_none());
    }

    #[test]
    fn runlists() {
        // 0x21: 1 length byte, 2 offset bytes. Runs: 0x18 clusters at 0x5634, then 0x10 at
        // 0x5634 - 0x10 (negative delta), then a sparse run of 4, then 1 at +0x100.
        let rl = [0x21, 0x18, 0x34, 0x56, 0x21, 0x10, 0xF0, 0xFF, 0x01, 0x04, 0x21, 0x01, 0x00, 0x01, 0x00];
        let runs = decode_runlist(&rl, 0).unwrap();
        assert_eq!(
            runs,
            vec![
                Run { vcn: 0, lcn: Some(0x5634), len: 0x18 },
                Run { vcn: 0x18, lcn: Some(0x5624), len: 0x10 },
                Run { vcn: 0x28, lcn: None, len: 4 },
                Run { vcn: 0x2C, lcn: Some(0x5724), len: 1 },
            ]
        );
        assert_eq!(map_vcn(&runs, 0x19), Some((Some(0x5625), 0x0F)));
        assert_eq!(map_vcn(&runs, 0x29), Some((None, 3)));
        assert_eq!(map_vcn(&runs, 0x2D), None);
        // Truncated and negative-LCN lists are rejected.
        assert!(decode_runlist(&[0x21, 0x18, 0x34], 0).is_none());
        assert!(decode_runlist(&[0x11, 0x01, 0xFF], 0).is_none());
        // Starting VCN carries through.
        assert_eq!(decode_runlist(&[0x11, 0x02, 0x10, 0x00], 100).unwrap()[0].vcn, 100);
    }

    #[test]
    fn wof_tag_is_preserved_in_either_attribute_order() {
        for first in [true, false] {
            let mut b = RecBuilder::new(1024, 9, REC_IN_USE, 0);
            if first { b.resident(0xC0, &0x8000_0017u32.to_le_bytes()); }
            b.std_info(0, attr::REPARSE_POINT);
            if !first { b.resident(0xC0, &0x8000_0017u32.to_le_bytes()); }
            let p = parse_record(&mut b.finish(0xABCD), 42).unwrap();
            assert_eq!(p.attrs & attr::WOF, attr::WOF);
        }
        let mut b = RecBuilder::new(1024, 9, REC_IN_USE, 0);
        b.std_info(0, attr::REPARSE_POINT);
        b.resident(0xC0, &0xA000_000Cu32.to_le_bytes()); // a symlink is not compression
        assert_eq!(parse_record(&mut b.finish(0xABCD), 42).unwrap().attrs & attr::WOF, 0);
    }

    #[test]
    fn record_parsing_and_fixups() {
        let mut b = RecBuilder::new(1024, 9, REC_IN_USE, 0);
        b.std_info(133_000_000_000_000_000, attr::HIDDEN | attr::ARCHIVE);
        b.file_name(40, 3, 2, "LONGNA~1.TXT"); // DOS name first
        b.file_name(40, 3, 1, "Long name.txt"); // Win32 wins
        b.file_name(77, 1, 1, "hardlink.txt"); // second hard link: ignored
        b.attr(AT_DATA, false, "Zone.Identifier", &{
            let mut v = vec![0u8; 8];
            v[0..4].copy_from_slice(&26u32.to_le_bytes());
            v[4..6].copy_from_slice(&40u16.to_le_bytes());
            v.extend_from_slice(&[0u8; 26]);
            v
        });
        b.data_nonresident("", 0, 10_000, 12_288, 0, &[0x21, 0x03, 0x00, 0x10]);
        let mut rec = b.finish(0xABCD);
        // The sector ends now hold the USN.
        assert_eq!(&rec[510..512], &0xABCDu16.to_le_bytes());
        let p = parse_record(&mut rec, 1234).unwrap();
        assert_eq!(p.frn, 1234);
        assert_eq!(p.seq, 9);
        assert_eq!(p.base, 0);
        assert!(!p.is_dir);
        assert_eq!(p.modified, 133_000_000_000_000_000);
        assert_eq!(p.attrs, attr::HIDDEN | attr::ARCHIVE);
        assert_eq!(p.name, Some(FileName { parent: 40, parent_seq: 3, rank: 0, name: "Long name.txt".into() }));
        assert_eq!(p.data, Some((10_000, 12_288)));

        // Resident data, directory flag, DOS-only name.
        let mut b = RecBuilder::new(1024, 1, REC_IN_USE | REC_DIRECTORY, 0);
        b.file_name(5, 5, 2, "PROGRA~1");
        b.resident(AT_DATA, b"hello");
        let mut rec = b.finish(7);
        let p = parse_record(&mut rec, 50).unwrap();
        assert!(p.is_dir);
        assert!(!p.has_std);
        assert_eq!(p.name.unwrap().rank, 2);
        assert_eq!(p.data, Some((5, 0)));

        // Compressed: allocated is the compressed size.
        let mut b = RecBuilder::new(1024, 1, REC_IN_USE, 0);
        b.data_nonresident("", 0, 100_000, 102_400, 0x0001, &[0x11, 0x19, 0x22]);
        let p = parse_record(&mut b.finish(3), 60).unwrap();
        assert_eq!(p.data, Some((100_000, 51_200)));

        // Extension record: base reference, data that doesn't start at VCN 0 is ignored.
        let mut b = RecBuilder::new(1024, 2, REC_IN_USE, 1234 | (9 << 48));
        b.data_nonresident("", 3, 0, 0, 0, &[0x11, 0x01, 0x05]);
        let p = parse_record(&mut b.finish(3), 99).unwrap();
        assert_eq!(p.base, 1234);
        assert_eq!(p.data, None);

        // Torn write: a sector end doesn't match the USN.
        let mut b = RecBuilder::new(1024, 1, REC_IN_USE, 0);
        b.file_name(5, 5, 1, "x");
        let mut rec = b.finish(0x1111);
        rec[1022] = 0;
        assert!(parse_record(&mut rec, 70).is_none());

        // Free record.
        let mut rec = RecBuilder::new(1024, 1, 0, 0).finish(1);
        assert!(parse_record(&mut rec, 71).is_none());

        // 4K records (8 fixup slots).
        let mut b = RecBuilder::new(4096, 1, REC_IN_USE, 0);
        b.file_name(5, 5, 3, "big");
        let mut rec = b.finish(0x0102);
        assert_eq!(parse_record(&mut rec, 72).unwrap().name.unwrap().name, "big");
    }

    fn rec(frn: u64, seq: u16, parent: u64, pseq: u16, name: &str, dir: bool, data: Option<(u64, u64)>) -> Parsed {
        Parsed {
            frn,
            seq,
            base: 0,
            is_dir: dir,
            has_std: true,
            modified: 0,
            attrs: if name.starts_with('$') { attr::HIDDEN | attr::SYSTEM } else { 0 },
            name: Some(FileName { parent, parent_seq: pseq, rank: 0, name: name.into() }),
            data,
        }
    }

    #[test]
    fn index_builds_trees() {
        let mut idx = Index::with_capacity(64);
        let recs = vec![
            rec(0, 1, 5, 5, "$MFT", false, Some((1 << 20, 1 << 20))),
            rec(5, 5, 5, 5, ".", true, None),
            rec(11, 11, 5, 5, "$Extend", true, None),
            rec(30, 1, 11, 11, "$UsnJrnl", false, Some((0, 0))),
            rec(40, 2, 5, 5, "Program Files", true, None),
            rec(41, 1, 40, 2, "app.exe", false, None), // data comes from an extension record
            rec(43, 1, 40, 2, "linked.dll", false, Some((300, 4096))),
            rec(44, 1, 40, 1, "stale.txt", false, Some((999, 4096))), // parent seq mismatch
            rec(45, 4, 46, 1, "orphan.txt", false, Some((7, 0))), // parent not in use
            rec(47, 1, 5, 5, "pagefile.sys", false, Some((5000, 8192))),
            rec(48, 1, 40, 2, "Sub", true, None),
            rec(49, 1, 48, 1, "tiny.ini", false, Some((20, 0))),
        ];
        for r in recs {
            idx.merge(r);
        }
        // Extension record of 41 with its $DATA and a second (DOS) name.
        idx.merge(Parsed { frn: 42, seq: 1, base: 41, data: Some((123_456, 126_976)), ..Default::default() });
        idx.merge(Parsed {
            frn: 42,
            seq: 1,
            base: 41,
            name: Some(FileName { parent: 40, parent_seq: 2, rank: 2, name: "APP~1.EXE".into() }),
            ..Default::default()
        });

        let trees = idx.build(&[(5, PathBuf::from("C:\\")), (40, PathBuf::from("C:\\Program Files"))]);
        let t = &trees[0];
        let root = &t.nodes[ROOT];
        let names: Vec<&str> = root.children.iter().map(|&c| t.nodes[c].name.as_str()).collect();
        assert_eq!(names, vec!["Program Files", "pagefile.sys", "$Extend"]);
        assert_eq!(root.size, 123_456 + 300 + 20 + 5000);
        assert_eq!(root.allocated, 126_976 + 4096 + 8192);
        assert_eq!(root.files, 5);
        assert_eq!(root.folders, 3);
        assert_eq!(t.method, ScanMethod::Mft);
        let pf = &t.nodes[root.children[0]];
        assert!(pf.attributes & attr::DIRECTORY != 0);
        let app = pf.children.iter().map(|&c| &t.nodes[c]).find(|n| n.name == "app.exe").unwrap();
        assert_eq!((app.size, app.allocated), (123_456, 126_976));
        assert_eq!(t.path_of(t.nodes.iter().position(|n| n.name == "tiny.ini").unwrap()), PathBuf::from(r"C:\Program Files\Sub\tiny.ini"));
        for (i, n) in t.nodes.iter().enumerate() {
            for &c in &n.children {
                assert!(c > i);
            }
        }

        let sub = &trees[1];
        assert_eq!(sub.nodes[ROOT].name, "C:\\Program Files");
        assert_eq!(sub.nodes[ROOT].size, 123_456 + 300 + 20);
        assert_eq!(sub.nodes[ROOT].files, 3);
        assert_eq!(sub.nodes.len(), 5);

        // Skipped folders leave the tree whole, with every total below them, at any depth.
        let progress = Progress::with_exclusions(&[PathBuf::from(r"c:\program files\SUB"), PathBuf::from(r"C:\pagefile.sys")]);
        let trees = idx.build_excluding(&[(5, PathBuf::from("C:\\"))], &progress);
        let t = &trees[0];
        assert!(!t.nodes.iter().any(|n| n.name == "Sub" || n.name == "tiny.ini" || n.name == "pagefile.sys"));
        assert_eq!(t.nodes[ROOT].size, 123_456 + 300);
        assert_eq!(t.nodes[ROOT].files, 3);
        assert!(t.nodes.iter().any(|n| n.name == "app.exe"));
        let whole = Progress::with_exclusions(&[PathBuf::from(r"D:\Elsewhere")]);
        assert_eq!(idx.build_excluding(&[(5, PathBuf::from("C:\\"))], &whole)[0].nodes.len(), idx.build(&[(5, PathBuf::from("C:\\"))])[0].nodes.len(), "other volumes' scopes change nothing");
    }

    #[test]
    fn top_tracker_resolves_ancestors() {
        let mut idx = Index::with_capacity(64);
        idx.merge(rec(5, 5, 5, 5, ".", true, None));
        idx.merge(rec(20, 1, 5, 5, "Windows", true, None));
        idx.merge(rec(21, 1, 20, 1, "System32", true, None));
        idx.merge(rec(22, 1, 21, 1, "a.dll", false, Some((10, 0))));
        idx.merge(rec(23, 1, 5, 5, "file.bin", false, Some((5, 0))));
        idx.merge(rec(24, 1, 99, 1, "unknown-parent", false, Some((5, 0))));
        let p = Progress::new();
        let mut t = TopTracker::new(5, 64);
        let a = t.slot_for(&idx, 22, &p).unwrap();
        let b = t.slot_for(&idx, 23, &p).unwrap();
        assert_ne!(a, b);
        assert_eq!(t.slot_for(&idx, 22, &p), Some(a));
        assert_eq!(t.slot_for(&idx, 24, &p), None);
        // Scanning a subfolder: files outside it resolve to nothing.
        let mut t = TopTracker::new(20, 64);
        assert!(t.slot_for(&idx, 22, &p).is_some());
        assert_eq!(t.slot_for(&idx, 23, &p), None);
        let names: Vec<String> = p.snapshot().top.into_iter().map(|t| t.0).collect();
        assert!(names.contains(&"Windows".to_string()) && names.contains(&"System32".to_string()));
    }

    #[test]
    fn final_top_totals_include_files_merged_before_their_parents() {
        let mut idx = Index::with_capacity(64);
        idx.merge(rec(22, 1, 21, 1, "early.dll", false, Some((10, 0))));
        let progress = Progress::new();
        let mut tracker = TopTracker::new(5, 64);
        assert_eq!(tracker.slot_for(&idx, 22, &progress), None);
        idx.merge(rec(5, 5, 5, 5, ".", true, None));
        idx.merge(rec(20, 1, 5, 5, "Windows", true, None));
        idx.merge(rec(21, 1, 20, 1, "System32", true, None));
        idx.merge(rec(23, 1, 5, 5, "other.bin", false, Some((5, 0))));
        let mut totals = tracker.final_totals(&idx);
        totals.sort();
        assert_eq!(totals, vec![("Windows".into(), 10), ("other.bin".into(), 5)]);
    }

    /// The whole reader pipeline (boot sector, record 0, fragmented MFT, chunked unbuffered
    /// reads, parser threads, merge, live progress) on a small NTFS image file.
    #[test]
    fn reads_a_synthetic_volume_image() {
        const CL: usize = 4096;
        const RS: usize = 1024;
        const RECORDS: usize = 64; // 16 clusters: VCN 0..8 at LCN 4, VCN 8..16 at LCN 40
        let mut img = vec![0u8; 48 * CL];
        img[3..11].copy_from_slice(b"NTFS    ");
        img[0x0B..0x0D].copy_from_slice(&512u16.to_le_bytes());
        img[0x0D] = 8;
        img[0x28..0x30].copy_from_slice(&((48 * CL / 512) as u64).to_le_bytes());
        img[0x30..0x38].copy_from_slice(&4u64.to_le_bytes());
        img[0x40] = 0xF6;
        let place = |img: &mut Vec<u8>, frn: usize, rec: Vec<u8>| {
            let byte = frn * RS;
            let vcn = byte / CL;
            let lcn = if vcn < 8 { 4 + vcn } else { 40 + vcn - 8 };
            let off = lcn * CL + byte % CL;
            img[off..off + RS].copy_from_slice(&rec);
        };
        let sys = attr::HIDDEN | attr::SYSTEM;
        let mut b = RecBuilder::new(RS, 1, REC_IN_USE, 0);
        b.std_info(0, sys);
        b.file_name(5, 5, 3, "$MFT");
        b.data_nonresident("", 0, (RECORDS * RS) as u64, (RECORDS * RS) as u64, 0, &[0x11, 0x08, 0x04, 0x11, 0x08, 0x24]);
        place(&mut img, 0, b.finish(1));
        let simple = |frn: usize, seq: u16, flags: u16, parent: u64, pseq: u16, name: &str, attrs: u32| {
            let mut b = RecBuilder::new(RS, seq, flags, 0);
            b.std_info(133_000_000_000_000_000 + frn as u64, attrs);
            b.file_name(parent, pseq, 3, name);
            b
        };
        let dir = REC_IN_USE | REC_DIRECTORY;
        place(&mut img, 3, simple(3, 3, REC_IN_USE, 5, 5, "$LogFile", sys).finish(2));
        place(&mut img, 5, simple(5, 5, dir, 5, 5, ".", sys).finish(3));
        place(&mut img, 11, simple(11, 11, dir, 5, 5, "$Extend", sys).finish(4));
        place(&mut img, 20, simple(20, 1, dir, 5, 5, "Users", 0).finish(5));
        let mut b = simple(21, 1, REC_IN_USE, 20, 1, "a.txt", attr::ARCHIVE);
        b.resident(AT_DATA, b"0123456789");
        place(&mut img, 21, b.finish(6));
        let mut b = simple(33, 2, REC_IN_USE, 20, 1, "big.bin", attr::ARCHIVE);
        b.data_nonresident("", 0, 1_000_000, 1_003_520, 0, &[0x21, 0xF5, 0x10, 0x00]);
        place(&mut img, 33, b.finish(7));
        place(&mut img, 34, simple(34, 1, REC_IN_USE, 20, 1, "many-attrs.dat", 0).finish(8));
        let mut b = RecBuilder::new(RS, 1, REC_IN_USE, 34 | (1 << 48));
        b.data_nonresident("", 0, 5000, 8192, 0, &[0x11, 0x02, 0x30]);
        place(&mut img, 35, b.finish(9));
        place(&mut img, 50, simple(50, 1, dir, 5, 5, "Program Files", 0).finish(10));
        let mut b = RecBuilder::new(RS, 1, REC_IN_USE, 0);
        b.file_name(50, 1, 2, "TOOL~1.EXE");
        b.file_name(50, 1, 1, "tool.exe");
        b.data_nonresident("", 0, 777, 4096, 0, &[0x11, 0x01, 0x31]);
        place(&mut img, 51, b.finish(11));
        place(&mut img, 60, simple(60, 1, 0, 20, 1, "deleted.txt", 0).finish(12));

        let path = std::env::temp_dir().join(format!("sprawl-mft-image-{}.img", std::process::id()));
        std::fs::write(&path, &img).unwrap();
        let p = Progress::new();
        let res = read_mft_device(&wide(path.as_os_str()), "X:", &p, Some(ROOT_FRN), 4 * CL);
        let _ = std::fs::remove_file(&path);
        let mut index = res.unwrap();

        let s = p.snapshot();
        assert_eq!(s.fraction, Some(1.0));
        assert_eq!(s.top.first(), Some(&("Users".to_string(), 1_005_010)));
        assert!(s.top.contains(&("Program Files".to_string(), 777)));
        assert!(s.by_ext.iter().any(|e| e.0 == "bin" && e.1 == 1_000_000));
        assert!(s.current.starts_with("X:\\"));

        let trees = index.build(&[(ROOT_FRN, PathBuf::from("X:\\"))]);
        let t = &trees[0];
        let r = &t.nodes[ROOT];
        let names: Vec<&str> = r.children.iter().map(|&c| t.nodes[c].name.as_str()).collect();
        assert_eq!(names, vec!["Users", "Program Files", "$Extend"]);
        assert_eq!((r.size, r.files, r.folders), (1_005_010 + 777, 4, 3));
        assert_eq!(r.allocated, 1_003_520 + 8192 + 4096);
        let ext = &t.nodes[r.children[2]];
        assert!(ext.is_hidden() && ext.is_system() && ext.is_dir);
        let tool = t.nodes.iter().find(|n| n.name == "tool.exe").unwrap();
        assert_eq!((tool.size, tool.allocated), (777, 4096));
        let a = t.nodes.iter().find(|n| n.name == "a.txt").unwrap();
        assert_eq!((a.size, a.allocated, a.attributes), (10, 0, attr::ARCHIVE));
        assert_eq!(a.modified, filetime_to_system(133_000_000_000_000_021));
        assert!(!t.nodes.iter().any(|n| n.name == "deleted.txt" || n.name == "$LogFile" || n.name == "$MFT"));
    }
}
