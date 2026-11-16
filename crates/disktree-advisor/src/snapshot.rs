//! A compact copy of a scan for history and diffs.
//!
//! Layout: `DTSN`, a little-endian u32 version, then one zstd frame holding:
//!
//! * a header: node count, root path, scan method, scan time, unreadable count;
//! * the name table: every distinct name once, in first-seen order;
//! * columns, each one value per node, as LEB128 varints:
//!   name index, parent distance (`i - parent`; children always come after their parent),
//!   flags (`attributes << 1 | is_dir`), and for files (and empty folders) size,
//!   `allocated - size` (zig-zag) and the modification time in seconds since 1601 as a
//!   zig-zag delta from the previous node's.
//!
//! Folder totals aren't stored: [`read`] adds them up again. Times keep one-second precision.
//! Columns compress much better than rows because neighbouring values look alike.

use std::collections::HashMap;
use std::io::{self, Read, Write};
use std::path::PathBuf;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use disktree_core::{Node, ScanMethod, Tree};

const MAGIC: &[u8; 4] = b"DTSN";
const VERSION: u32 = 1;
/// Seconds from 1601-01-01 to 1970-01-01.
const EPOCH_1601: i64 = 11_644_473_600;

pub struct Meta {
    pub root_path: PathBuf,
    pub method: ScanMethod,
    /// When the scan finished.
    pub taken: SystemTime,
    pub nodes: usize,
}

fn put(out: &mut Vec<u8>, mut v: u64) {
    loop {
        let b = (v & 0x7f) as u8;
        v >>= 7;
        if v == 0 {
            out.push(b);
            return;
        }
        out.push(b | 0x80);
    }
}

fn zig(v: i64) -> u64 {
    ((v << 1) ^ (v >> 63)) as u64
}

fn unzig(v: u64) -> i64 {
    ((v >> 1) as i64) ^ -((v & 1) as i64)
}

fn secs(t: Option<SystemTime>) -> i64 {
    match t {
        None => 0,
        Some(t) => match t.duration_since(UNIX_EPOCH) {
            Ok(d) => d.as_secs() as i64 + EPOCH_1601,
            Err(e) => (EPOCH_1601 - e.duration().as_secs() as i64).max(1),
        },
    }
}

fn from_secs(s: i64) -> Option<SystemTime> {
    if s <= 0 {
        return None;
    }
    let u = s - EPOCH_1601;
    if u >= 0 {
        Some(UNIX_EPOCH + Duration::from_secs(u as u64))
    } else {
        UNIX_EPOCH.checked_sub(Duration::from_secs((-u) as u64))
    }
}

fn method_code(m: ScanMethod) -> u8 {
    match m {
        ScanMethod::Mft => 1,
        ScanMethod::Csv => 2,
        ScanMethod::Walk => 0,
    }
}

/// Does this node carry its own size (files, and folders with nothing below them)?
fn leaf(n: &Node) -> bool {
    !n.is_dir || n.children.is_empty()
}

/// The uncompressed body (exposed for measuring).
pub fn encode(tree: &Tree, taken: SystemTime) -> Vec<u8> {
    let mut out = Vec::with_capacity(tree.nodes.len() * 12);
    put(&mut out, tree.nodes.len() as u64);
    let root = tree.root_path.to_string_lossy();
    put(&mut out, root.len() as u64);
    out.extend_from_slice(root.as_bytes());
    out.push(method_code(tree.method));
    put(&mut out, secs(Some(taken)) as u64);
    put(&mut out, tree.errors);

    let mut index: HashMap<&str, u64> = HashMap::new();
    let mut names: Vec<&str> = Vec::new();
    let ids: Vec<u64> = tree
        .nodes
        .iter()
        .map(|n| {
            *index.entry(n.name.as_str()).or_insert_with(|| {
                names.push(n.name.as_str());
                names.len() as u64 - 1
            })
        })
        .collect();
    put(&mut out, names.len() as u64);
    for s in &names {
        put(&mut out, s.len() as u64);
        out.extend_from_slice(s.as_bytes());
    }
    for &id in &ids {
        put(&mut out, id);
    }
    for (i, n) in tree.nodes.iter().enumerate() {
        put(&mut out, n.parent.map_or(0, |p| (i - p) as u64));
    }
    for n in &tree.nodes {
        put(&mut out, (u64::from(n.attributes) << 1) | u64::from(n.is_dir));
    }
    for n in tree.nodes.iter().filter(|n| leaf(n)) {
        put(&mut out, n.size);
    }
    for n in tree.nodes.iter().filter(|n| leaf(n)) {
        put(&mut out, zig(n.allocated as i64 - n.size as i64));
    }
    let mut prev = 0i64;
    for n in &tree.nodes {
        let s = secs(n.modified);
        put(&mut out, zig(s - prev));
        prev = s;
    }
    out
}

/// Write a snapshot. `level` is the zstd level (3 is fast, 19 is small).
pub fn write(tree: &Tree, taken: SystemTime, level: i32, w: &mut impl Write) -> io::Result<()> {
    write_body(&encode(tree, taken), level, w)
}

/// Compress and write a body made by [`encode`] (so encoding can happen on one thread and the
/// slower compression on another).
pub fn write_body(body: &[u8], level: i32, w: &mut impl Write) -> io::Result<()> {
    w.write_all(MAGIC)?;
    w.write_all(&VERSION.to_le_bytes())?;
    let mut z = zstd::stream::Encoder::new(w, level)?;
    z.multithread(std::thread::available_parallelism().map_or(1, |n| n.get() as u32))?;
    z.write_all(body)?;
    z.finish()?;
    Ok(())
}

struct Cursor<'a> {
    b: &'a [u8],
    at: usize,
}

impl Cursor<'_> {
    fn bad() -> io::Error {
        io::Error::new(io::ErrorKind::InvalidData, "snapshot is damaged")
    }

    fn get(&mut self) -> io::Result<u64> {
        let mut v = 0u64;
        let mut shift = 0;
        loop {
            let b = *self.b.get(self.at).ok_or_else(Self::bad)?;
            self.at += 1;
            if shift > 63 {
                return Err(Self::bad());
            }
            v |= u64::from(b & 0x7f) << shift;
            if b & 0x80 == 0 {
                return Ok(v);
            }
            shift += 7;
        }
    }

    fn bytes(&mut self, n: usize) -> io::Result<&[u8]> {
        let end = self.at.checked_add(n).filter(|&e| e <= self.b.len()).ok_or_else(Self::bad)?;
        let s = &self.b[self.at..end];
        self.at = end;
        Ok(s)
    }

    fn string(&mut self) -> io::Result<String> {
        let n = self.get()? as usize;
        Ok(String::from_utf8_lossy(self.bytes(n)?).into_owned())
    }
}

/// Read a snapshot back into a tree (with folder totals) and its metadata.
pub fn read(r: &mut impl Read) -> io::Result<(Tree, Meta)> {
    let mut head = [0u8; 8];
    r.read_exact(&mut head)?;
    if &head[..4] != MAGIC {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "not a DiskTree snapshot"));
    }
    let version = u32::from_le_bytes([head[4], head[5], head[6], head[7]]);
    if version != VERSION {
        return Err(io::Error::new(io::ErrorKind::InvalidData, format!("snapshot version {version} isn't supported")));
    }
    let mut body = Vec::new();
    zstd::stream::Decoder::new(r)?.read_to_end(&mut body)?;
    decode(&body)
}

pub fn decode(body: &[u8]) -> io::Result<(Tree, Meta)> {
    let mut c = Cursor { b: body, at: 0 };
    let count = c.get()? as usize;
    // every node takes at least five bytes in the columns
    if count == 0 || count > body.len() {
        return Err(Cursor::bad());
    }
    let root_path = PathBuf::from(c.string()?);
    let method = match c.bytes(1)?[0] {
        1 => ScanMethod::Mft,
        2 => ScanMethod::Csv,
        _ => ScanMethod::Walk,
    };
    let taken = from_secs(c.get()? as i64).unwrap_or(UNIX_EPOCH);
    let errors = c.get()?;
    let n_names = c.get()? as usize;
    if n_names > body.len() {
        return Err(Cursor::bad());
    }
    let mut names = Vec::with_capacity(n_names);
    for _ in 0..n_names {
        names.push(c.string()?);
    }
    let mut nodes: Vec<Node> = Vec::with_capacity(count);
    for _ in 0..count {
        let id = c.get()? as usize;
        let name = names.get(id).ok_or_else(Cursor::bad)?.clone();
        nodes.push(Node { name, ..Default::default() });
    }
    for i in 0..count {
        let d = c.get()? as usize;
        if i == 0 {
            continue;
        }
        if d == 0 || d > i {
            return Err(Cursor::bad());
        }
        nodes[i].parent = Some(i - d);
        nodes[i - d].children.push(i);
    }
    for n in nodes.iter_mut() {
        let f = c.get()?;
        n.is_dir = f & 1 == 1;
        n.attributes = (f >> 1) as u32;
        n.files = u64::from(!n.is_dir);
    }
    let leaves: Vec<usize> = (0..count).filter(|&i| leaf(&nodes[i])).collect();
    for &i in &leaves {
        nodes[i].size = c.get()?;
    }
    for &i in &leaves {
        nodes[i].allocated = (nodes[i].size as i64 + unzig(c.get()?)).max(0) as u64;
    }
    let mut prev = 0i64;
    for n in nodes.iter_mut() {
        prev += unzig(c.get()?);
        n.modified = from_secs(prev);
    }
    let mut tree = Tree { root_path: root_path.clone(), nodes, errors, method };
    tree.compute_totals();
    Ok((tree, Meta { root_path, method, taken, nodes: count }))
}

/// Name of the pseudo-file that stands for a folder's own files in a [`folders_only`] tree.
pub const FILES_HERE: &str = "<files>";

/// A smaller tree for long-range history: every folder, plus one pseudo-file per folder (named
/// [`FILES_HERE`]) holding the total of the files directly inside it. Folder totals still add
/// up exactly; only individual files are gone.
pub fn folders_only(t: &Tree) -> Tree {
    let mut map = vec![usize::MAX; t.nodes.len()];
    let mut nodes: Vec<Node> = Vec::new();
    for (i, n) in t.nodes.iter().enumerate() {
        if !n.is_dir {
            continue;
        }
        let parent = n.parent.map(|p| map[p]);
        map[i] = nodes.len();
        nodes.push(Node {
            name: n.name.clone(),
            is_dir: true,
            attributes: n.attributes,
            modified: n.modified,
            parent,
            ..Default::default()
        });
        if let Some(p) = parent {
            let me = nodes.len() - 1;
            nodes[p].children.push(me);
        }
        let (mut size, mut alloc, mut files, mut newest) = (0u64, 0u64, 0u64, None::<SystemTime>);
        for &c in &n.children {
            let f = &t.nodes[c];
            if !f.is_dir {
                size += f.size;
                alloc += f.allocated;
                files += 1;
                newest = newest.max(f.modified);
            }
        }
        if files > 0 {
            let me = nodes.len() - 1;
            nodes.push(Node { name: FILES_HERE.into(), size, allocated: alloc, files, modified: newest, parent: Some(me), ..Default::default() });
            let pf = nodes.len() - 1;
            nodes[me].children.push(pf);
        }
    }
    let mut out = Tree { root_path: t.root_path.clone(), nodes, errors: t.errors, method: t.method };
    out.compute_totals();
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Tree {
        let t0 = UNIX_EPOCH + Duration::from_secs(1_700_000_000);
        let mut nodes = vec![
            Node { name: r"C:\data".into(), is_dir: true, ..Default::default() },
            Node { name: "a".into(), is_dir: true, parent: Some(0), ..Default::default() },
            Node { name: "x.bin".into(), size: 5000, allocated: 8192, files: 1, parent: Some(1), modified: Some(t0), ..Default::default() },
            Node { name: "x.bin".into(), size: 10, allocated: 0, files: 1, parent: Some(0), modified: Some(t0 - Duration::from_secs(90)), attributes: 0x2, ..Default::default() },
            Node { name: "empty".into(), is_dir: true, parent: Some(0), ..Default::default() },
            Node { name: "old".into(), size: 1, allocated: 4096, files: 1, parent: Some(1), modified: None, ..Default::default() },
        ];
        nodes[0].children = vec![1, 3, 4];
        nodes[1].children = vec![2, 5];
        let mut t = Tree { root_path: r"C:\data".into(), nodes, errors: 3, method: ScanMethod::Walk };
        t.compute_totals();
        t
    }

    #[test]
    fn round_trip() {
        let t = sample();
        let mut buf = Vec::new();
        let taken = UNIX_EPOCH + Duration::from_secs(1_750_000_000);
        write(&t, taken, 3, &mut buf).unwrap();
        let (back, meta) = read(&mut buf.as_slice()).unwrap();
        assert_eq!(meta.nodes, t.nodes.len());
        assert_eq!(meta.taken, taken);
        assert_eq!(back.errors, 3);
        assert_eq!(back.nodes[0].size, t.nodes[0].size);
        assert_eq!(back.nodes[0].allocated, t.nodes[0].allocated);
        assert_eq!(back.nodes[0].files, 3);
        assert_eq!(back.nodes[0].folders, 2);
        for i in 0..t.nodes.len() {
            assert_eq!(back.path_of(i), t.path_of(i));
            assert_eq!(back.nodes[i].size, t.nodes[i].size, "{i}");
            assert_eq!(back.nodes[i].allocated, t.nodes[i].allocated, "{i}");
            assert_eq!(back.nodes[i].modified, t.nodes[i].modified, "{i}");
            assert_eq!(back.nodes[i].attributes, t.nodes[i].attributes);
            assert_eq!(back.nodes[i].is_dir, t.nodes[i].is_dir);
        }
    }

    #[test]
    fn damaged_input_is_an_error_not_a_panic() {
        let t = sample();
        let body = encode(&t, UNIX_EPOCH);
        for cut in 0..body.len() {
            assert!(decode(&body[..cut]).is_err());
        }
        let mut bad = body.clone();
        bad[0] = 0xff; // absurd node count
        assert!(decode(&bad).is_err());
    }
}
