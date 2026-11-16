//! The file-type colour palette and a small dependency-free PNG writer.

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
