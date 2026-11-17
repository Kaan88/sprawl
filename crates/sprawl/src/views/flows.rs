//! Flow (a Sankey from file type to folder to the Advisor's tag), Strata (folders as rock
//! layers, the safe ones on top), Grove (the tree as a tree) and Lens (a hyperbolic tree).

use std::collections::HashMap;
use std::f32::consts::{PI, TAU};

use super::{hash01, memo, mix, rgba, scale, Ctx, Out, Shape};

const RISK_NAMES: [&str; 4] = ["No finding", "Comes back by itself", "Costs something", "Your own files"];

fn risk_colour(r: usize) -> [f32; 3] {
    match r {
        1 => crate::advisor::lens_colour(1),
        2 => crate::advisor::lens_colour(2),
        3 => crate::advisor::lens_colour(3),
        _ => [0.34, 0.37, 0.45],
    }
}

/// Bytes by (file type, top folder, risk) below the root: the eight biggest types and folders,
/// the rest in "other".
struct Flows {
    types: Vec<String>,
    folders: Vec<usize>,
    /// [type][folder] and [folder][risk]
    tf: Vec<Vec<u64>>,
    fr: Vec<[u64; 4]>,
}

fn flows(c: &Ctx) -> std::rc::Rc<Flows> {
    memo("flows", c, || {
        let folders: Vec<usize> = c.kids(c.root).into_iter().take(8).collect();
        let fi: HashMap<usize, usize> = folders.iter().enumerate().map(|(i, &f)| (f, i)).collect();
        let mut by_type: HashMap<String, u64> = HashMap::new();
        let mut files: Vec<(String, usize, u8, u64)> = Vec::new();
        let mut stack: Vec<(usize, usize)> = c.tree.nodes[c.root].children.iter().map(|&k| (k, fi.get(&k).copied().unwrap_or(folders.len()))).collect();
        while let Some((n, f)) = stack.pop() {
            let node = &c.tree.nodes[n];
            if node.is_dir {
                stack.extend(node.children.iter().map(|&k| (k, f)));
            } else {
                let e = node.extension();
                let b = (c.size)(n);
                *by_type.entry(e.clone()).or_default() += b;
                files.push((e, f, c.risk_of(n).min(3), b));
            }
        }
        let mut types: Vec<(String, u64)> = by_type.into_iter().collect();
        types.sort_by_key(|t| std::cmp::Reverse(t.1));
        let types: Vec<String> = types.into_iter().take(8).map(|t| t.0).collect();
        let ti: HashMap<&str, usize> = types.iter().enumerate().map(|(i, t)| (t.as_str(), i)).collect();
        let mut tf = vec![vec![0u64; folders.len() + 1]; types.len() + 1];
        let mut fr = vec![[0u64; 4]; folders.len() + 1];
        for (e, f, r, b) in files {
            let t = ti.get(e.as_str()).copied().unwrap_or(types.len());
            tf[t][f] += b;
            fr[f][r as usize] += b;
        }
        Flows { types, folders, tf, fr }
    })
}

/// A band from (x0, y0..y0+h0) to (x1, y1..y1+h1), eased in the middle.
fn ribbon(o: &mut Out, x0: f32, y0: f32, x1: f32, y1: f32, h: f32, col: [f32; 4]) {
    let steps = 28;
    let at = |t: f32| {
        let s = t * t * (3.0 - 2.0 * t);
        (x0 + (x1 - x0) * t, y0 + (y1 - y0) * s)
    };
    for k in 0..steps {
        let (a, b) = (at(k as f32 / steps as f32), at((k + 1) as f32 / steps as f32));
        o.quad([a.0, a.1], [b.0, b.1], [b.0, b.1 + h], [a.0, a.1 + h], col);
    }
}

pub(crate) fn flow(c: &Ctx, o: &mut Out) {
    let f = flows(c);
    let nt = f.types.len() + 1;
    let nf = f.folders.len() + 1;
    let total: u64 = f.fr.iter().map(|r| r.iter().sum::<u64>()).sum();
    if total == 0 {
        return;
    }
    let (top, bottom) = (24.0, c.h - 14.0);
    let gap = 8.0;
    let colw = 14.0;
    let xs = [c.w * 0.14, c.w * 0.5 - colw * 0.5, c.w * 0.86 - colw];
    // each column's blocks: (bytes, y, height)
    let lay = |sizes: &[u64]| -> Vec<(f32, f32)> {
        let n = sizes.iter().filter(|&&s| s > 0).count().max(1);
        let k = (bottom - top - gap * (n as f32 - 1.0)) / total as f32;
        let mut y = top;
        sizes.iter().map(|&s| {
            let h = s as f32 * k;
            let r = (y, h);
            if s > 0 {
                y += h + gap;
            }
            r
        }).collect()
    };
    let tsize: Vec<u64> = (0..nt).map(|t| f.tf[t].iter().sum()).collect();
    let fsize: Vec<u64> = (0..nf).map(|i| f.fr[i].iter().sum()).collect();
    let rsize: Vec<u64> = (0..4).map(|r| f.fr.iter().map(|x| x[r]).sum()).collect();
    let (tl, fl, rl) = (lay(&tsize), lay(&fsize), lay(&rsize));
    let k = (bottom - top - gap * (nf.max(nt) as f32 - 1.0)).max(10.0) / total as f32;
    let k = k.min((tl.iter().map(|x| x.1).sum::<f32>()) / total as f32);
    let tcol = |t: usize| if t < f.types.len() { (c.ext_colour)(&f.types[t]) } else { [0.45, 0.46, 0.5] };
    let fcol = |i: usize| if i < f.folders.len() { c.col(f.folders[i]) } else { [0.45, 0.46, 0.5] };
    let hover_folder = c.hover.and_then(|h| f.folders.iter().position(|&x| x == h));
    let grow = c.intro(1.0);
    // type -> folder ribbons
    let mut tout = vec![0.0f32; nt];
    let mut fin = vec![0.0f32; nf];
    for t in 0..nt {
        for i in 0..nf {
            let b = f.tf[t][i];
            if b == 0 {
                continue;
            }
            let h = b as f32 * k;
            let (y0, y1) = (tl[t].0 + tout[t], fl[i].0 + fin[i]);
            tout[t] += h;
            fin[i] += h;
            let lit = hover_folder.is_none_or(|hf| hf == i);
            let a = if lit { 0.38 } else { 0.08 };
            let x1 = xs[0] + colw + (xs[1] - xs[0] - colw) * grow;
            ribbon(o, xs[0] + colw, y0, x1, y1, h, rgba(tcol(t), a));
            // beads running along it
            if h > 3.0 && lit {
                let s = (c.time * 0.18 + hash01((t * 31 + i) as u64)).fract() * grow;
                let e = s * s * (3.0 - 2.0 * s);
                o.disc([xs[0] + colw + (xs[1] - xs[0] - colw) * s, y0 + (y1 - y0) * e + h * 0.5], 1.8, [1.0, 1.0, 1.0, 0.55]);
            }
        }
    }
    // folder -> risk ribbons
    let mut fout = vec![0.0f32; nf];
    let mut rin = [0.0f32; 4];
    for i in 0..nf {
        for r in 0..4 {
            let b = f.fr[i][r];
            if b == 0 {
                continue;
            }
            let h = b as f32 * k;
            let (y0, y1) = (fl[i].0 + fout[i], rl[r].0 + rin[r]);
            fout[i] += h;
            rin[r] += h;
            let lit = hover_folder.is_none_or(|hf| hf == i);
            let a = if lit { 0.38 } else { 0.08 };
            let x1 = xs[1] + colw + (xs[2] - xs[1] - colw) * grow;
            ribbon(o, xs[1] + colw, y0, x1, y1, h, rgba(mix(fcol(i), risk_colour(r), 0.5), a));
            if h > 3.0 && lit {
                let s = (c.time * 0.18 + hash01((i * 17 + r + 99) as u64)).fract() * grow;
                let e = s * s * (3.0 - 2.0 * s);
                o.disc([xs[1] + colw + (xs[2] - xs[1] - colw) * s, y0 + (y1 - y0) * e + h * 0.5], 1.8, [1.0, 1.0, 1.0, 0.55]);
            }
        }
    }
    // the blocks and their names
    o.label(xs[0] - 4.0, 2.0, 140.0, "File type".into(), true);
    o.label(xs[1] - 30.0, 2.0, 140.0, "Folder".into(), true);
    o.label(xs[2] - 60.0, 2.0, 160.0, "What the Advisor says".into(), true);
    for t in 0..nt {
        let (y, h) = (tl[t].0, tsize[t] as f32 * k);
        if tsize[t] == 0 {
            continue;
        }
        o.rect(xs[0], y, colw, h, rgba(tcol(t), 1.0));
        let name = if t < f.types.len() { if f.types[t].is_empty() { "(none)".to_string() } else { format!(".{}", f.types[t]) } } else { "other".into() };
        let tip = format!("{name}   {}", (c.fmt)(tsize[t]));
        o.tip(Shape::Rect([xs[0], y, colw, h.max(4.0)]), tip.clone());
        if h > 12.0 {
            o.label(xs[0] - 104.0, y + h * 0.5 - 8.0, 100.0, tip, false);
        }
    }
    for i in 0..nf {
        let (y, h) = (fl[i].0, fsize[i] as f32 * k);
        if fsize[i] == 0 {
            continue;
        }
        o.rect(xs[1], y, colw, h, rgba(scale(fcol(i), if hover_folder == Some(i) { 1.3 } else { 1.0 }), 1.0));
        if i < f.folders.len() {
            o.hit(Shape::Rect([xs[1] - 4.0, y, colw + 8.0, h.max(4.0)]), f.folders[i]);
        } else {
            o.tip(Shape::Rect([xs[1], y, colw, h.max(4.0)]), format!("Everything else   {}", (c.fmt)(fsize[i])));
        }
        if h > 12.0 {
            let name = if i < f.folders.len() { c.tree.nodes[f.folders[i]].name.clone() } else { "other".into() };
            o.label(xs[1] + colw + 6.0, y + h * 0.5 - 8.0, 150.0, name, false);
        }
    }
    for r in 0..4 {
        let (y, h) = (rl[r].0, rsize[r] as f32 * k);
        if rsize[r] == 0 {
            continue;
        }
        o.rect(xs[2], y, colw, h, rgba(risk_colour(r), 1.0));
        let tip = format!("{}   {}", RISK_NAMES[r], (c.fmt)(rsize[r]));
        o.tip(Shape::Rect([xs[2], y, colw, h.max(4.0)]), tip.clone());
        if h > 12.0 {
            o.label(xs[2] + colw + 6.0, y + h * 0.5 - 8.0, c.w - xs[2] - colw - 10.0, tip, false);
        }
    }
}

/// Folders as rock layers: what can go on top, what to keep at the bottom.
pub(crate) fn strata(c: &Ctx, o: &mut Out) {
    let mut kids: Vec<usize> = c.kids(c.root).into_iter().take(14).collect();
    let share = |k: usize| c.reclaim_of(k) as f32 / (c.size)(k).max(1) as f32;
    // the most that can go on top, then by size
    kids.sort_by(|&a, &b| share(b).partial_cmp(&share(a)).unwrap_or(std::cmp::Ordering::Equal).then((c.size)(b).cmp(&(c.size)(a))));
    let total: f32 = kids.iter().map(|&k| (c.size)(k) as f32).sum::<f32>().max(1.0);
    let (left, right) = (16.0, c.w - 220.0);
    let (top, bottom) = (30.0, c.h - 10.0);
    let min_h = 7.0;
    let k = ((bottom - top) - min_h * kids.len() as f32).max(10.0) / total;
    let edge = |i: usize, x: f32| -> f32 {
        // a gently warped boundary, different for every layer
        let s = hash01(i as u64 * 7 + 3);
        3.0 * (x * (0.008 + 0.006 * s) + s * 9.0).sin() + 1.5 * (x * 0.031 + s * 3.0).sin()
    };
    let drill = left + (right - left) * (0.5 + 0.35 * (c.time * 0.12).sin());
    let grow = c.intro(1.0);
    let mut y = top;
    for (i, &n) in kids.iter().enumerate() {
        let h = ((c.size)(n) as f32 * k + min_h) * grow;
        let col = scale(c.col(n), c.lit(n));
        let cap = share(n).clamp(0.0, 1.0) * h;
        let step = 6.0;
        let mut x = left;
        while x < right {
            let x2 = (x + step).min(right);
            let (t0, t1) = (y + edge(i, x), y + edge(i, x2));
            let (b0, b1) = (y + h + edge(i + 1, x), y + h + edge(i + 1, x2));
            let (b0, b1) = (if i + 1 == kids.len() { y + h } else { b0 }, if i + 1 == kids.len() { y + h } else { b1 });
            o.quad([x, t0], [x2, t1], [x2, b1], [x, b0], rgba(scale(col, 0.85), 1.0));
            if cap > 0.5 {
                o.quad([x, t0], [x2, t1], [x2, t1 + cap], [x, t0 + cap], rgba(mix(col, [0.92, 0.95, 0.88], 0.55), 1.0));
            }
            x = x2;
        }
        // speckles in the rock
        let dots = ((right - left) * h / 900.0) as usize;
        for d in 0..dots.min(160) {
            let px = left + (right - left) * hash01((i * 1000 + d) as u64);
            let py = y + 2.0 + (h - 4.0).max(0.0) * hash01((i * 1000 + d + 500) as u64);
            o.rect(px, py, 1.6, 1.6, rgba(scale(col, 0.55), 0.8));
        }
        o.hit(Shape::Rect([left, y, right - left, h]), n);
        let note = if c.reclaim_of(n) > 0 { format!("   {} can go", (c.fmt)(c.reclaim_of(n))) } else { String::new() };
        if h > 13.0 {
            o.label(right + 10.0, y + h * 0.5 - 8.0, c.w - right - 14.0, format!("{}  {}{note}", c.tree.nodes[n].name, (c.fmt)((c.size)(n))), false);
        }
        y += h;
    }
    // the drill: a rig on the surface and a pipe down to where it has reached
    let depth = top + (y - top) * (0.5 + 0.5 * (c.time * 0.2).sin()).powf(0.7);
    o.rect(drill - 1.5, top - 18.0, 3.0, depth - top + 18.0, [0.85, 0.87, 0.92, 0.9]);
    o.tri([drill - 12.0, top - 2.0], [drill + 12.0, top - 2.0], [drill, top - 22.0], [0.85, 0.6, 0.25, 0.95]);
    o.rect(drill - 4.0, depth - 4.0, 8.0, 8.0, [1.0, 0.85, 0.4, 0.95]);
    o.label(left, 6.0, 260.0, "Surface: what can go first".into(), false);
}

/// The folder tree as a tree: trunk at the bottom, branch thickness by size, leaves are files.
pub(crate) fn grove(c: &Ctx, o: &mut Out) {
    let base = [c.w * 0.5, c.h - 8.0];
    let total = (c.size)(c.root).max(1) as f32;
    let trunk_w = (c.w.min(c.h) * 0.06).max(6.0);
    let len0 = c.h * 0.26;
    let grow = c.intro(1.4);
    // (node, start point, angle (0 = up), length, width, depth)
    let mut stack: Vec<(usize, [f32; 2], f32, f32, f32, usize)> = vec![(c.root, base, 0.0, len0, trunk_w, 0)];
    let mut placed: Vec<[f32; 2]> = Vec::new();
    while let Some((n, p, ang, len, w, d)) = stack.pop() {
        // sways in the wind, more towards the tips
        let sway = 0.035 * d as f32 * (c.time * 1.1 + d as f32 * 0.7 + hash01(n as u64) * 3.0).sin();
        let a = ang + sway;
        let reach = (grow * 6.0 - d as f32).clamp(0.0, 1.0);
        if reach <= 0.0 {
            continue;
        }
        let tip = [p[0] + a.sin() * len * reach, p[1] - a.cos() * len * reach];
        let col = scale(mix(c.col(n), [0.42, 0.3, 0.2], if d < 2 { 0.55 } else { 0.25 }), c.lit(n));
        o.stroke(p, tip, w, rgba(col, 1.0));
        o.disc(tip, w * 0.5, rgba(col, 1.0));
        let (dx, dy) = (tip[0] - p[0], tip[1] - p[1]);
        let l = (dx * dx + dy * dy).sqrt().max(0.01);
        let (nx, ny) = (-dy / l * (w * 0.5 + 2.0), dx / l * (w * 0.5 + 2.0));
        o.hit(Shape::Poly(vec![[p[0] + nx, p[1] + ny], [tip[0] + nx, tip[1] + ny], [tip[0] - nx, tip[1] - ny], [p[0] - nx, p[1] - ny]]), n);
        if d == 1 && len > 40.0 && !placed.iter().any(|q: &[f32; 2]| (q[0] - tip[0]).abs() < 100.0 && (q[1] - tip[1]).abs() < 18.0) {
            placed.push(tip);
            o.label(tip[0] - 50.0, tip[1] - 18.0, 100.0, c.tree.nodes[n].name.clone(), false);
        }
        if d >= 7 {
            continue;
        }
        let kids: Vec<usize> = c.kids(n).into_iter().take(if d < 2 { 7 } else { 5 }).collect();
        let sum: f32 = kids.iter().map(|&k| (c.size)(k) as f32).sum::<f32>().max(1.0);
        let spread = (1.25 - d as f32 * 0.08).max(0.6);
        let mut at = -spread * 0.5;
        for k in kids {
            let part = (c.size)(k) as f32 / sum;
            let ka = a + at + part * spread * 0.5 + (hash01(k as u64) - 0.5) * 0.2;
            at += part * spread;
            let kw = (w * ((c.size)(k) as f32 / (c.size)(n).max(1) as f32).sqrt()).max(1.0);
            if c.tree.nodes[k].is_dir {
                if kw >= 1.0 && (c.size)(k) as f32 / total > 0.0005 {
                    stack.push((k, tip, ka, len * (0.62 + 0.22 * hash01(k as u64 + 5)), kw, d + 1));
                }
            } else if (c.size)(k) as f32 / total > 0.002 {
                // a big file is a leaf
                let lp = [tip[0] + ka.sin() * 9.0, tip[1] - ka.cos() * 9.0];
                let r = (((c.size)(k) as f32 / total).sqrt() * 40.0).clamp(2.5, 14.0) * reach;
                o.disc(lp, r, rgba(scale(c.col(k), 1.1 * c.lit(k)), 0.9));
                o.hit(Shape::Circle(lp, r + 2.0), k);
            }
        }
    }
    // the ground
    o.rect(0.0, c.h - 8.0, c.w, 8.0, [0.18, 0.15, 0.12, 0.9]);
    let _ = (PI, TAU);
}

/// Complex numbers for the Poincaré disc.
#[derive(Clone, Copy)]
struct Z(f32, f32);

impl Z {
    fn mul(self, o: Z) -> Z {
        Z(self.0 * o.0 - self.1 * o.1, self.0 * o.1 + self.1 * o.0)
    }
    fn div(self, o: Z) -> Z {
        let d = o.0 * o.0 + o.1 * o.1;
        Z((self.0 * o.0 + self.1 * o.1) / d, (self.1 * o.0 - self.0 * o.1) / d)
    }
    fn conj(self) -> Z {
        Z(self.0, -self.1)
    }
    fn abs2(self) -> f32 {
        self.0 * self.0 + self.1 * self.1
    }
}

/// The disc moved so that `a` is in the middle: z -> (z - a) / (1 - conj(a) z).
fn mobius(z: Z, a: Z) -> Z {
    Z(z.0 - a.0, z.1 - a.1).div(Z(1.0, 0.0).sub(a.conj().mul(z)))
}

impl Z {
    fn sub(self, o: Z) -> Z {
        Z(self.0 - o.0, self.1 - o.1)
    }
}

/// The whole tree in a disc; the focus is large in the middle, far branches shrink to the rim.
pub(crate) fn lens(c: &Ctx, o: &mut Out, focus: Option<usize>) {
    // a radial layout in the hyperbolic plane: each level one step out, angles by size
    let layout = memo("lens", c, || {
        let mut nodes: Vec<(usize, Option<usize>, Z)> = Vec::new();
        let mut queue = std::collections::VecDeque::new();
        queue.push_back((c.root, None::<usize>, 0usize, 0.0f32, TAU));
        while let Some((n, parent, depth, a0, a1)) = queue.pop_front() {
            if nodes.len() >= 400 {
                break;
            }
            let r = (depth as f32 * 0.9 * 0.5).tanh();
            let a = (a0 + a1) * 0.5;
            let idx = nodes.len();
            nodes.push((n, parent, Z(r * a.sin(), -r * a.cos())));
            if depth >= 6 {
                continue;
            }
            let kids: Vec<usize> = c.kids(n).into_iter().take(9).collect();
            let sum: f32 = kids.iter().map(|&k| ((c.size)(k) as f32).sqrt()).sum::<f32>().max(1.0);
            let mut s = a0;
            for k in kids {
                let span = ((c.size)(k) as f32).sqrt() / sum * (a1 - a0);
                queue.push_back((k, Some(idx), depth + 1, s, s + span));
                s += span;
            }
        }
        nodes
    });
    let centre = [c.w * 0.5, c.h * 0.5];
    let rad = c.w.min(c.h) * 0.5 - 12.0;
    let target = focus.and_then(|f| layout.iter().find(|x| x.0 == f)).map_or(Z(0.0, 0.0), |x| x.2);
    // glide to the new focus
    let e = c.intro(0.9);
    let a = Z(target.0 * e, target.1 * e);
    let pos: Vec<Z> = layout.iter().map(|x| mobius(x.2, a)).collect();
    let screen = |z: Z| [centre[0] + z.0 * rad, centre[1] + z.1 * rad];
    o.disc(centre, rad, [0.06, 0.07, 0.1, 0.6]);
    o.circle_line(centre, rad, [1.0, 1.0, 1.0, 0.25]);
    for (i, x) in layout.iter().enumerate() {
        if let Some(p) = x.1 {
            let (za, zb) = (pos[p], pos[i]);
            let pts: Vec<[f32; 2]> = (0..=8).map(|k| {
                let t = k as f32 / 8.0;
                screen(Z(za.0 + (zb.0 - za.0) * t, za.1 + (zb.1 - za.1) * t))
            }).collect();
            let fade = (1.0 - zb.abs2()).clamp(0.1, 1.0);
            o.polyline(&pts, [0.75, 0.8, 0.9, 0.35 * fade]);
        }
    }
    let total = (c.size)(c.root).max(1) as f32;
    for (i, x) in layout.iter().enumerate() {
        let z = pos[i];
        let k = (1.0 - z.abs2()).max(0.0);
        let r = (3.0 + ((c.size)(x.0) as f32 / total).sqrt() * 26.0) * k + 0.6;
        let p = screen(z);
        o.disc(p, r, rgba(scale(c.col(x.0), c.lit(x.0)), 0.95));
        o.hit(Shape::Circle(p, r.max(4.0)), x.0);
        if k > 0.55 && r > 5.0 {
            o.label(p[0] + r + 3.0, p[1] - 8.0, 130.0 * k, c.tree.nodes[x.0].name.clone(), i == 0 || Some(x.0) == focus);
        }
    }
}
