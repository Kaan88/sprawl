//! Treemap painting into a pixel buffer (cushion-shaded tiles) and a small PNG writer.

use std::collections::HashMap;
use disktree_core::{squarify, Rect, Tree};

/// Vivid colours for the biggest file types; everything else is grey (as in the original).
pub const PALETTE: [(u8, u8, u8); 20] = [
    (235, 125, 20),
    (45, 175, 65),
    (195, 205, 25),
    (205, 45, 150),
    (25, 145, 235),
    (150, 65, 215),
    (125, 115, 245),
    (225, 45, 205),
    (235, 25, 35),
    (135, 165, 45),
    (25, 205, 175),
    (160, 95, 35),
    (60, 70, 225),
    (245, 185, 45),
    (35, 130, 130),
    (205, 105, 155),
    (105, 205, 245),
    (175, 175, 65),
    (245, 95, 95),
    (95, 145, 65),
];
pub const GREY: (u8, u8, u8) = (105, 105, 112);

pub struct Layout {
    /// Every laid-out node (folders and files) with its rectangle, parents before children.
    pub rects: Vec<(usize, Rect)>,
}

impl Layout {
    /// The deepest node under a point.
    pub fn hit(&self, x: f64, y: f64) -> Option<usize> {
        self.rects.iter().rev().find(|(_, r)| r.contains(x, y)).map(|(n, _)| *n)
    }
    pub fn rect_of(&self, node: usize) -> Option<Rect> {
        self.rects.iter().find(|(n, _)| *n == node).map(|(_, r)| *r)
    }
}

pub fn layout(tree: &Tree, start: usize, w: f64, h: f64) -> Layout {
    let mut rects = Vec::new();
    let mut stack = vec![(start, Rect::new(0.0, 0.0, w, h))];
    while let Some((index, rect)) = stack.pop() {
        if rect.w < 1.0 || rect.h < 1.0 || rects.len() > 400_000 {
            continue;
        }
        rects.push((index, rect));
        let node = &tree.nodes[index];
        if !node.is_dir || node.children.is_empty() {
            continue;
        }
        let sizes: Vec<f64> = node.children.iter().map(|&c| tree.nodes[c].size as f64).collect();
        for (&child, r) in node.children.iter().zip(squarify(&sizes, rect)).rev() {
            stack.push((child, r));
        }
    }
    Layout { rects }
}

/// Paint tiles as BGRA (premultiplied, opaque) into `px`.
pub fn paint(
    tree: &Tree,
    lay: &Layout,
    colours: &HashMap<String, usize>,
    highlight_ext: Option<&str>,
    selected: Option<usize>,
    w: usize,
    h: usize,
    px: &mut [u8],
) {
    for p in px.chunks_exact_mut(4) {
        p.copy_from_slice(&[22, 20, 20, 255]);
    }
    for (node, r) in &lay.rects {
        let n = &tree.nodes[*node];
        if n.is_dir {
            continue;
        }
        let ext = n.extension();
        let mut c = colours.get(&ext).map(|&i| PALETTE[i]).unwrap_or(GREY);
        if let Some(hx) = highlight_ext {
            if hx != ext {
                c = (c.0 / 4 + 10, c.1 / 4 + 10, c.2 / 4 + 10);
            }
        }
        fill_cushion(px, w, h, r, c);
    }
    if let Some(sel) = selected {
        if let Some(r) = lay.rect_of(sel) {
            outline(px, w, h, &r, (255, 255, 255), 2);
        }
    }
}

fn fill_cushion(px: &mut [u8], w: usize, h: usize, r: &Rect, c: (u8, u8, u8)) {
    let x0 = r.x.round().max(0.0) as usize;
    let y0 = r.y.round().max(0.0) as usize;
    let x1 = (r.x + r.w).round().min(w as f64) as usize;
    let y1 = (r.y + r.h).round().min(h as f64) as usize;
    if x1 <= x0 || y1 <= y0 {
        return;
    }
    let (cx, cy) = (r.x + r.w / 2.0, r.y + r.h / 2.0);
    let (hw, hh) = ((r.w / 2.0).max(0.5), (r.h / 2.0).max(0.5));
    for y in y0..y1 {
        let ny = (y as f64 + 0.5 - cy) / hh;
        for x in x0..x1 {
            let nx = (x as f64 + 0.5 - cx) / hw;
            // A soft pillow lit from the top left.
            let mut l = 1.0 - 0.32 * (nx * nx + ny * ny) - 0.14 * (nx + ny);
            if x + 1 == x1 || y + 1 == y1 {
                l *= 0.55;
            }
            let l = l.clamp(0.2, 1.25);
            let i = (y * w + x) * 4;
            px[i] = (c.2 as f64 * l).min(255.0) as u8;
            px[i + 1] = (c.1 as f64 * l).min(255.0) as u8;
            px[i + 2] = (c.0 as f64 * l).min(255.0) as u8;
            px[i + 3] = 255;
        }
    }
}

fn outline(px: &mut [u8], w: usize, h: usize, r: &Rect, c: (u8, u8, u8), t: usize) {
    let x0 = r.x.max(0.0) as usize;
    let y0 = r.y.max(0.0) as usize;
    let x1 = ((r.x + r.w) as usize).min(w);
    let y1 = ((r.y + r.h) as usize).min(h);
    let mut put = |x: usize, y: usize| {
        if x < w && y < h {
            let i = (y * w + x) * 4;
            px[i] = c.2;
            px[i + 1] = c.1;
            px[i + 2] = c.0;
        }
    };
    for k in 0..t {
        for x in x0..x1 {
            put(x, y0 + k);
            put(x, y1.saturating_sub(1 + k));
        }
        for y in y0..y1 {
            put(x0 + k, y);
            put(x1.saturating_sub(1 + k), y);
        }
    }
}

// ---- PNG (stored deflate blocks; simple and dependency free) ----

fn crc32(data: &[u8], mut c: u32) -> u32 {
    c = !c;
    for &b in data {
        c ^= b as u32;
        for _ in 0..8 {
            c = if c & 1 != 0 { 0xEDB8_8320 ^ (c >> 1) } else { c >> 1 };
        }
    }
    !c
}

fn chunk(out: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
    out.extend((data.len() as u32).to_be_bytes());
    let start = out.len();
    out.extend(kind);
    out.extend(data);
    let crc = crc32(&out[start..], 0);
    out.extend(crc.to_be_bytes());
}

/// Encode RGBA rows as a PNG file.
pub fn png(w: u32, h: u32, rgba: &[u8]) -> Vec<u8> {
    let mut raw = Vec::with_capacity((w as usize * 4 + 1) * h as usize);
    for row in rgba.chunks_exact(w as usize * 4) {
        raw.push(0);
        raw.extend(row);
    }
    let mut z = vec![0x78, 0x01];
    let blocks: Vec<&[u8]> = raw.chunks(65535).collect();
    for (i, b) in blocks.iter().enumerate() {
        z.push(u8::from(i + 1 == blocks.len()));
        let len = b.len() as u16;
        z.extend(len.to_le_bytes());
        z.extend((!len).to_le_bytes());
        z.extend(*b);
    }
    let (mut a, mut b) = (1u32, 0u32);
    for &x in &raw {
        a = (a + x as u32) % 65521;
        b = (b + a) % 65521;
    }
    z.extend(((b << 16) | a).to_be_bytes());

    let mut out = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
    let mut ihdr = Vec::new();
    ihdr.extend(w.to_be_bytes());
    ihdr.extend(h.to_be_bytes());
    ihdr.extend([8, 6, 0, 0, 0]);
    chunk(&mut out, b"IHDR", &ihdr);
    chunk(&mut out, b"IDAT", &z);
    chunk(&mut out, b"IEND", &[]);
    out
}
