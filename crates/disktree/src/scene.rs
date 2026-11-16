//! The 3D treemap scene: nested squarified layout, per-tile spring animation (morphs between
//! layouts, hover lift, ripple, finish wave), the tilted camera, picking and folder labels.
//! Produces instance data for `gpu::Gpu`. Coordinates are DIPs of the map panel.

use std::collections::HashMap;

use disktree_core::{squarify, Rect, Tree};

use crate::gpu::{Instance, Label};

/// Per-node facts the scene needs, looked up once when a node first appears.
#[derive(Clone, Copy)]
pub struct Info {
    pub colour: [f32; 3],
    /// Identifies the file type, for highlighting one type.
    pub ext_key: u32,
}

#[derive(Clone, Copy)]
struct Anim {
    node: u32,
    parent: u32,
    is_dir: bool,
    depth: u8,
    label: bool,
    present: bool,
    // current and target rectangles
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    tx: f32,
    ty: f32,
    tw: f32,
    th: f32,
    lift: f32,
    vel: f32,
    glow: f32,
    dim: f32,
    info: Info,
    under_hover_folder: bool,
}

pub struct Scene {
    pub w: f32,
    pub h: f32,
    anims: Vec<Anim>,
    slot: HashMap<u32, usize>,
    pub instances: Vec<Instance>,
    // interaction
    pub hover_leaf: Option<usize>,
    hover_parent: Option<u32>,
    pub hover_folder: Option<usize>,
    pub highlight_ext: Option<u32>,
    pub selected: Option<usize>,
    mouse: Option<(f32, f32)>,
    mouse_rel: (f32, f32),
    wave: Option<f32>,
    time: f32,
    // camera
    pub tilt_on: bool,
    pub cushion: f32,
    pub lift_scale: f32,
    pub animate: bool,
    tilt: f32,
    yaw: f32,
    pub vp: [f32; 16],
    ivp: [f32; 16],
    pub eye: [f32; 3],
    settled_frames: u32,
}

impl Default for Scene {
    fn default() -> Self {
        Scene {
            w: 1.0,
            h: 1.0,
            anims: Vec::new(),
            slot: HashMap::new(),
            instances: Vec::new(),
            hover_leaf: None,
            hover_parent: None,
            hover_folder: None,
            highlight_ext: None,
            selected: None,
            mouse: None,
            mouse_rel: (0.5, 0.5),
            wave: None,
            time: 0.0,
            tilt_on: true,
            cushion: 1.0,
            lift_scale: 1.0,
            animate: true,
            tilt: 0.44,
            yaw: 0.0,
            vp: IDENTITY,
            ivp: IDENTITY,
            eye: [0.0, 0.0, 1000.0],
            settled_frames: 0,
        }
    }
}

const IDENTITY: [f32; 16] = [1., 0., 0., 0., 0., 1., 0., 0., 0., 0., 1., 0., 0., 0., 0., 1.];
const HEADER: f32 = 15.0;

impl Scene {
    pub fn clear(&mut self) {
        self.anims.clear();
        self.slot.clear();
        self.instances.clear();
        self.hover_leaf = None;
        self.hover_parent = None;
        self.hover_folder = None;
    }

    /// Lay out `root` of `tree` into the panel, weighting each node with `weight` (0 = hidden).
    /// Existing tiles glide to their new places; new ones grow from their centre.
    pub fn relayout(&mut self, tree: &Tree, root: usize, weight: &dyn Fn(usize) -> u64, info: &dyn Fn(usize) -> Info) {
        for a in &mut self.anims {
            a.present = false;
        }
        let (w, h) = (self.w, self.h);
        let mut stack: Vec<(usize, Rect, u8)> = vec![(root, Rect::new(0.0, 0.0, w as f64, h as f64), 0)];
        let mut count = 0usize;
        while let Some((n, r, d)) = stack.pop() {
            count += 1;
            if count > 600_000 {
                break;
            }
            let node = &tree.nodes[n];
            let mut label = false;
            let (mut ix, mut iy, mut iw, mut ih) = (r.x, r.y, r.w, r.h);
            if node.is_dir && d > 0 {
                let pad = if r.w > 10.0 && r.h > 10.0 { 1.6 } else { 0.0 };
                ix += pad;
                iy += pad;
                iw -= pad * 2.0;
                ih -= pad * 2.0;
                if d <= 2 && iw > 64.0 && ih > 44.0 {
                    label = true;
                    iy += HEADER as f64;
                    ih -= HEADER as f64;
                }
            }
            self.place(n, node.parent.unwrap_or(n), node.is_dir, d, label, r, info);
            if !node.is_dir || iw <= 0.0 || ih <= 0.0 {
                continue;
            }
            let mut kids: Vec<usize> = node.children.iter().copied().filter(|&c| weight(c) > 0).collect();
            kids.sort_by_key(|&c| std::cmp::Reverse(weight(c)));
            let sizes: Vec<f64> = kids.iter().map(|&c| weight(c) as f64).collect();
            let rects = squarify(&sizes, Rect::new(ix, iy, iw, ih));
            for (&c, cr) in kids.iter().zip(rects).rev() {
                if cr.w >= 0.7 && cr.h >= 0.7 {
                    stack.push((c, cr, d.saturating_add(1)));
                }
            }
        }
        self.settled_frames = 0;
        self.refresh_flags(tree);
    }

    fn place(&mut self, n: usize, parent: usize, is_dir: bool, d: u8, label: bool, r: Rect, info: &dyn Fn(usize) -> Info) {
        let (x, y, w, h) = (r.x as f32, r.y as f32, r.w as f32, r.h as f32);
        if let Some(&i) = self.slot.get(&(n as u32)) {
            let a = &mut self.anims[i];
            a.present = true;
            a.depth = d;
            a.label = label;
            (a.tx, a.ty, a.tw, a.th) = (x, y, w, h);
        } else {
            let start = if self.animate { (x + w / 2.0, y + h / 2.0, 0.0, 0.0) } else { (x, y, w, h) };
            self.slot.insert(n as u32, self.anims.len());
            self.anims.push(Anim {
                node: n as u32,
                parent: parent as u32,
                is_dir,
                depth: d,
                label,
                present: true,
                x: start.0,
                y: start.1,
                w: start.2,
                h: start.3,
                tx: x,
                ty: y,
                tw: w,
                th: h,
                lift: 0.0,
                vel: 0.0,
                glow: 0.0,
                dim: 0.0,
                info: info(n),
                under_hover_folder: false,
            });
        }
    }

    /// Move animation state from old node ids to new ones (after a rescan replaced the tree).
    pub fn rekey(&mut self, map: &dyn Fn(usize) -> Option<usize>) {
        let mut keep = Vec::new();
        for a in &self.anims {
            if let Some(n) = map(a.node as usize) {
                let mut a = *a;
                a.node = n as u32;
                keep.push(a);
            }
        }
        self.anims = keep;
        self.slot = self.anims.iter().enumerate().map(|(i, a)| (a.node, i)).collect();
    }

    /// Start the ripple that runs across the map when a scan finishes.
    pub fn start_wave(&mut self) {
        self.wave = Some(self.time + 0.25);
    }

    pub fn set_hover_folder(&mut self, tree: &Tree, folder: Option<usize>) {
        if self.hover_folder != folder {
            self.hover_folder = folder;
            self.refresh_flags(tree);
        }
    }

    fn refresh_flags(&mut self, tree: &Tree) {
        let f = self.hover_folder;
        for a in &mut self.anims {
            a.under_hover_folder = match f {
                None => false,
                Some(f) => {
                    let mut x = Some(a.node as usize);
                    let mut hit = false;
                    while let Some(i) = x {
                        if i == f {
                            hit = true;
                            break;
                        }
                        x = tree.nodes.get(i).and_then(|n| n.parent);
                    }
                    hit
                }
            };
        }
    }

    // ------------------------------------------------------------ camera

    fn camera(&mut self, dt: f32) {
        let (mx, my) = self.mouse_rel;
        let tt = if self.tilt_on { 0.44 + (my - 0.5) * 0.08 } else { 0.0 };
        let ty = if self.tilt_on { (mx - 0.5) * 0.16 } else { 0.0 };
        let k = if self.animate { (dt * 4.0).min(1.0) } else { 1.0 };
        self.tilt += (tt - self.tilt) * k;
        self.yaw += (ty - self.yaw) * k;
        let (w, h) = (self.w.max(1.0), self.h.max(1.0));
        let p = perspective(30f32.to_radians(), w / h, 10.0, 40_000.0);
        let make = |d: f32| {
            let e = [d * self.yaw.sin() * self.tilt.cos(), -d * self.tilt.sin(), d * self.tilt.cos() * self.yaw.cos()];
            (mul(&p, &look_at(e, [0.0, 0.0, 0.0], [0.0, 1.0, 0.0])), e)
        };
        let (mut lo, mut hi) = (w * 0.2, w * 8.0);
        for _ in 0..20 {
            let d = (lo + hi) / 2.0;
            let (m, _) = make(d);
            let ok = [[-w / 2., -h / 2.], [w / 2., -h / 2.], [w / 2., h / 2.], [-w / 2., h / 2.]].iter().all(|c| {
                let q = xform(&m, [c[0], c[1], 0.0, 1.0]);
                (q[0] / q[3]).abs() <= 0.985 && (q[1] / q[3]).abs() <= 0.965
            });
            if ok {
                hi = d;
            } else {
                lo = d;
            }
        }
        let (m, e) = make(hi);
        self.vp = m;
        self.ivp = invert(&m).unwrap_or(IDENTITY);
        self.eye = e;
    }

    /// Screen DIPs of a layout point at height `z`.
    pub fn project(&self, x: f32, y: f32, z: f32) -> (f32, f32) {
        let q = xform(&self.vp, [x - self.w / 2.0, self.h / 2.0 - y, z, 1.0]);
        ((q[0] / q[3] * 0.5 + 0.5) * self.w, (0.5 - q[1] / q[3] * 0.5) * self.h)
    }

    /// Layout point under a screen point (on the floor plane).
    pub fn unproject(&self, px: f32, py: f32) -> (f32, f32) {
        let nx = px / self.w * 2.0 - 1.0;
        let ny = 1.0 - py / self.h * 2.0;
        let a = xform(&self.ivp, [nx, ny, 0.0, 1.0]);
        let b = xform(&self.ivp, [nx, ny, 1.0, 1.0]);
        let p0 = [a[0] / a[3], a[1] / a[3], a[2] / a[3]];
        let p1 = [b[0] / b[3], b[1] / b[3], b[2] / b[3]];
        let t = p0[2] / (p0[2] - p1[2]);
        let x = p0[0] + (p1[0] - p0[0]) * t;
        let y = p0[1] + (p1[1] - p0[1]) * t;
        (x + self.w / 2.0, self.h / 2.0 - y)
    }

    // ------------------------------------------------------------ pointer

    /// Pointer moved to screen DIPs; returns the file under it.
    pub fn pointer(&mut self, px: f32, py: f32) -> Option<usize> {
        self.mouse_rel = (px / self.w.max(1.0), py / self.h.max(1.0));
        let (x, y) = self.unproject(px, py);
        self.mouse = Some((x, y));
        let hit = self.hit_layout(x, y, false);
        if hit != self.hover_leaf {
            self.hover_leaf = hit;
            self.hover_parent = hit.and_then(|n| self.slot.get(&(n as u32)).map(|&i| self.anims[i].parent));
        }
        self.settled_frames = 0;
        hit
    }

    pub fn leave(&mut self) {
        self.mouse = None;
        self.mouse_rel = (0.5, 0.5);
        self.hover_leaf = None;
        self.hover_parent = None;
    }

    /// Deepest node at a layout point: a file, or with `folders` the innermost folder.
    pub fn hit_layout(&self, x: f32, y: f32, folders: bool) -> Option<usize> {
        let mut best: Option<(u8, usize)> = None;
        for a in &self.anims {
            if !a.present || (a.is_dir && !folders) || (!a.is_dir && folders) {
                continue;
            }
            if x >= a.tx && y >= a.ty && x < a.tx + a.tw && y < a.ty + a.th && best.is_none_or(|(d, _)| a.depth > d) {
                best = Some((a.depth, a.node as usize));
                if !folders {
                    break;
                }
            }
        }
        best.map(|b| b.1)
    }

    /// Screen point to the node there (file, else folder).
    pub fn hit(&self, px: f32, py: f32) -> Option<usize> {
        let (x, y) = self.unproject(px, py);
        self.hit_layout(x, y, false).or_else(|| self.hit_layout(x, y, true))
    }

    // ------------------------------------------------------------ per frame

    /// Advance animations by `dt` seconds and rebuild the instance list. Returns false once
    /// everything has come to rest (nothing needs drawing until something changes).
    pub fn update(&mut self, dt: f32) -> bool {
        self.time += dt;
        self.camera(dt);
        let k_pos = if self.animate { (dt * 9.0).min(1.0) } else { 1.0 };
        let wave_t = self.wave.map(|t0| self.time - t0);
        if wave_t.is_some_and(|t| t > 3.0) {
            self.wave = None;
        }
        let (w, h) = (self.w, self.h);
        let mut moving = false;
        self.instances.clear();
        let mut i = 0;
        while i < self.anims.len() {
            let a = &mut self.anims[i];
            if !a.present {
                let (cx, cy) = (a.tx + a.tw / 2.0, a.ty + a.th / 2.0);
                (a.tx, a.ty, a.tw, a.th) = (cx, cy, 0.0, 0.0);
            }
            let before = (a.x, a.y, a.w, a.h);
            a.x += (a.tx - a.x) * k_pos;
            a.y += (a.ty - a.y) * k_pos;
            a.w += (a.tw - a.w) * k_pos;
            a.h += (a.th - a.h) * k_pos;
            if (a.x - before.0).abs() + (a.y - before.1).abs() + (a.w - before.2).abs() + (a.h - before.3).abs() > 0.01 {
                moving = true;
            }
            if !a.present && a.w < 0.4 {
                self.anims.swap_remove(i);
                if let Some(moved) = self.anims.get(i) {
                    self.slot.insert(moved.node, i);
                }
                continue;
            }
            i += 1;
        }
        // swap_remove may have left stale slots for removed nodes
        if self.slot.len() != self.anims.len() {
            self.slot = self.anims.iter().enumerate().map(|(i, a)| (a.node, i)).collect();
        }

        let hover_leaf = self.hover_leaf.map(|n| n as u32);
        let hover_parent = self.hover_parent;
        let hover_folder = self.hover_folder.map(|n| n as u32);
        let selected = self.selected.map(|n| n as u32);
        let mouse = if self.animate { self.mouse } else { None };
        let hl = self.highlight_ext;
        let ls = self.lift_scale;
        for idx in 0..self.anims.len() {
            let a = self.anims[idx];
            if a.is_dir && a.depth == 0 {
                continue;
            }
            let mut target = 0.0f32;
            let mut glow_t = 0.0f32;
            if !a.is_dir {
                if Some(a.node) == hover_leaf {
                    target = 26.0;
                    glow_t = 1.0;
                } else if hover_parent == Some(a.parent) && hover_leaf.is_some() {
                    target = 8.0;
                    glow_t = 0.22;
                }
                if a.under_hover_folder {
                    target = target.max(12.0);
                    glow_t = glow_t.max(0.3);
                }
                if let Some((mx, my)) = mouse {
                    let dx = a.x + a.w / 2.0 - mx;
                    let dy = a.y + a.h / 2.0 - my;
                    target += 7.0 * (-(dx * dx + dy * dy) / (2.0 * 70.0 * 70.0)).exp();
                }
                if let Some(t) = wave_t {
                    let d = (a.x + a.y * 1.6) / (w + h * 1.6);
                    let ph = t * 1.4 - d;
                    if ph > 0.0 && ph < 0.35 {
                        target += 18.0 * (ph / 0.35 * std::f32::consts::PI).sin();
                    }
                }
            } else if hover_folder == Some(a.node) {
                glow_t = 0.35;
            }
            if Some(a.node) == selected {
                glow_t = glow_t.max(0.6);
                target = target.max(10.0);
            }
            target *= ls;
            let pr = self.slot.get(&a.parent).map(|&p| &self.anims[p]).map(|p| [p.x, p.y, p.w, p.h]).unwrap_or([a.x, a.y, a.w, a.h]);
            let a = &mut self.anims[idx];
            if self.animate {
                a.vel += ((target - a.lift) * 170.0 - a.vel * 15.0) * dt;
                a.lift += a.vel * dt;
                a.glow += (glow_t - a.glow) * (dt * 10.0).min(1.0);
            } else {
                a.lift = target;
                a.vel = 0.0;
                a.glow = glow_t;
            }
            let dim_t = if hl.is_some() && (a.is_dir || Some(a.info.ext_key) != hl) { 1.0 } else { 0.0 };
            a.dim += (dim_t - a.dim) * if self.animate { (dt * 8.0).min(1.0) } else { 1.0 };
            if a.vel.abs() > 0.05 || (a.lift - target).abs() > 0.05 || (a.glow - glow_t).abs() > 0.01 || (a.dim - dim_t).abs() > 0.01 {
                moving = true;
            }

            let gap = if !a.is_dir && a.w > 3.0 && a.h > 3.0 { 0.45 } else { 0.0 };
            let (colour, kind) = if a.is_dir {
                let v = if a.depth == 1 { 0.15 } else { 0.12 };
                ([v, v * 1.04, v * 1.25], 1.0)
            } else {
                (a.info.colour, 0.0)
            };
            let base_h = if a.is_dir { 0.6 + (3.0 - a.depth.min(3) as f32) * 0.4 } else { 3.2 + ((a.w * a.h).sqrt() * 0.03).min(4.0) };
            self.instances.push(Instance {
                rect: [a.x + gap, a.y + gap, (a.w - gap * 2.0).max(0.0), (a.h - gap * 2.0).max(0.0)],
                parent: pr,
                color: [colour[0], colour[1], colour[2], kind],
                fx: [base_h, a.lift.max(0.0), a.glow, a.dim],
            });
        }
        if self.wave.is_some() || self.mouse.is_some() && self.animate {
            moving = true;
        }
        if moving {
            self.settled_frames = 0;
        } else {
            self.settled_frames += 1;
        }
        self.settled_frames < 3
    }

    /// Folder names for the label pass: "Users\ (629.9 GB)".
    pub fn labels(&self, tree: &Tree, text: &dyn Fn(usize) -> String) -> Vec<Label> {
        let mut out = Vec::new();
        for a in &self.anims {
            if !a.present || !a.label || a.w < 60.0 {
                continue;
            }
            let (sx, sy) = self.project(a.x + 4.0, a.y + 1.0, 2.0);
            let (ex, _) = self.project(a.x + a.w - 4.0, a.y + 1.0, 2.0);
            let max_w = ex - sx;
            if max_w < 36.0 || sx > self.w || sy > self.h {
                continue;
            }
            let n = a.node as usize;
            if n >= tree.nodes.len() {
                continue;
            }
            out.push(Label { x: sx, y: sy, max_w, text: text(n), strong: a.depth == 1 });
        }
        out
    }

}

// ---------------------------------------------------------------- column-major 4x4 math

fn perspective(fy: f32, aspect: f32, n: f32, f: f32) -> [f32; 16] {
    let t = 1.0 / (fy / 2.0).tan();
    // right-handed, depth 0..1 (Direct3D)
    [t / aspect, 0., 0., 0., 0., t, 0., 0., 0., 0., f / (n - f), -1., 0., 0., n * f / (n - f), 0.]
}

fn sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}
fn norm(a: [f32; 3]) -> [f32; 3] {
    let l = (a[0] * a[0] + a[1] * a[1] + a[2] * a[2]).sqrt().max(1e-9);
    [a[0] / l, a[1] / l, a[2] / l]
}
fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
fn look_at(e: [f32; 3], t: [f32; 3], up: [f32; 3]) -> [f32; 16] {
    let z = norm(sub(e, t));
    let x = norm(cross(up, z));
    let y = cross(z, x);
    [x[0], y[0], z[0], 0., x[1], y[1], z[1], 0., x[2], y[2], z[2], 0., -dot(x, e), -dot(y, e), -dot(z, e), 1.]
}
fn mul(a: &[f32; 16], b: &[f32; 16]) -> [f32; 16] {
    let mut o = [0.0; 16];
    for c in 0..4 {
        for r in 0..4 {
            o[c * 4 + r] = (0..4).map(|k| a[k * 4 + r] * b[c * 4 + k]).sum();
        }
    }
    o
}
fn xform(m: &[f32; 16], p: [f32; 4]) -> [f32; 4] {
    let mut o = [0.0; 4];
    for (r, v) in o.iter_mut().enumerate() {
        *v = m[r] * p[0] + m[4 + r] * p[1] + m[8 + r] * p[2] + m[12 + r] * p[3];
    }
    o
}
fn invert(m: &[f32; 16]) -> Option<[f32; 16]> {
    // Gauss-Jordan on a row-major copy.
    let mut a = [[0f64; 8]; 4];
    for r in 0..4 {
        for c in 0..4 {
            a[r][c] = m[c * 4 + r] as f64;
        }
        a[r][4 + r] = 1.0;
    }
    for col in 0..4 {
        let piv = (col..4).max_by(|&i, &j| a[i][col].abs().partial_cmp(&a[j][col].abs()).unwrap())?;
        if a[piv][col].abs() < 1e-12 {
            return None;
        }
        a.swap(col, piv);
        let d = a[col][col];
        for v in a[col].iter_mut() {
            *v /= d;
        }
        for r in 0..4 {
            if r != col {
                let f = a[r][col];
                let row = a[col];
                for (v, rv) in a[r].iter_mut().zip(row) {
                    *v -= f * rv;
                }
            }
        }
    }
    let mut o = [0f32; 16];
    for r in 0..4 {
        for c in 0..4 {
            o[c * 4 + r] = a[r][4 + c] as f32;
        }
    }
    Some(o)
}

/// Map a hashed extension to a stable key.
pub fn ext_key(ext: &str) -> u32 {
    let mut h: u32 = 0x811c_9dc5;
    for b in ext.bytes() {
        h ^= u32::from(b);
        h = h.wrapping_mul(0x0100_0193);
    }
    h
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unproject_inverts_project() {
        let mut s = Scene { w: 1200.0, h: 400.0, ..Default::default() };
        s.camera(1.0);
        for &(x, y) in &[(10.0, 10.0), (600.0, 200.0), (1100.0, 390.0)] {
            let (px, py) = s.project(x, y, 0.0);
            let (bx, by) = s.unproject(px, py);
            assert!((bx - x).abs() < 0.5 && (by - y).abs() < 0.5, "{x},{y} -> {bx},{by}");
        }
    }
}
