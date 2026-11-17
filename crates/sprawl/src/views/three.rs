//! The 3D views: Rose Stack (one coxcomb disc per scan, stacked in time), Ridgeline (a
//! landscape on the treemap) and Molecule (ball and stick). World units, z up; the triangles are
//! shaded here and drawn with depth by the mesh pass.

use std::f32::consts::TAU;

use super::{hash01, mix, rgba, scale, Cam, Ctx, Out, Paint, Shape, ViewState};
use crate::gpu::Vertex;
use crate::scene::{look_at, mul, norm, perspective, xform};

const LIGHT: [f32; 3] = [-0.45, 0.35, 0.82];

fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}

/// A lit triangle (flat shaded from its own normal, either side).
fn tri(o: &mut Out, a: [f32; 3], b: [f32; 3], c: [f32; 3], col: [f32; 3], alpha: f32) {
    let n = norm(cross(sub(b, a), sub(c, a)));
    let l = dot(n, norm(LIGHT)).abs();
    let k = 0.38 + 0.72 * l;
    let col = rgba(scale(col, k), alpha);
    for p in [a, b, c] {
        o.tris.push(Vertex { pos: p, col });
    }
}

fn quad(o: &mut Out, a: [f32; 3], b: [f32; 3], c: [f32; 3], d: [f32; 3], col: [f32; 3], alpha: f32) {
    tri(o, a, b, c, col, alpha);
    tri(o, a, c, d, col, alpha);
}

/// The camera around `target` at `reach` (times the user's zoom); fills `vp` and `eye`.
fn camera(c: &Ctx, cam: Cam, target: [f32; 3], reach: f32, st: &mut ViewState) {
    let d = reach * cam.dist;
    // a slow turn on its own, plus what the user dragged
    let yaw = cam.yaw + c.time * 0.05;
    let e = [target[0] + d * cam.tilt.cos() * yaw.sin(), target[1] - d * cam.tilt.cos() * yaw.cos(), target[2] + d * cam.tilt.sin()];
    let p = perspective(34f32.to_radians(), c.w / c.h.max(1.0), d * 0.05, d * 6.0);
    st.vp = mul(&p, &look_at(e, target, [0.0, 0.0, 1.0]));
    st.eye = e;
}

/// Where a world point lands on the panel, when it is in front of the camera.
fn screen(c: &Ctx, vp: &[f32; 16], p: [f32; 3]) -> Option<[f32; 2]> {
    let q = xform(vp, [p[0], p[1], p[2], 1.0]);
    (q[3] > 0.01).then(|| [(q[0] / q[3] + 1.0) * 0.5 * c.w, (1.0 - q[1] / q[3]) * 0.5 * c.h])
}

/// One disc per scan (oldest at the bottom); each wedge a top folder, radius by size.
pub(crate) fn rose(c: &Ctx, o: &mut Out, st: &mut ViewState) {
    let kids: Vec<usize> = c.kids(c.root).into_iter().take(12).collect();
    let mut layers: Vec<Vec<f32>> = c.history.iter().map(|(_, v)| kids.iter().map(|&k| v.get(k).copied().unwrap_or(0) as f32).collect()).collect();
    layers.push(kids.iter().map(|&k| (c.size)(k) as f32).collect());
    let peak = layers.iter().flatten().fold(1.0f32, |a, &b| a.max(b));
    let n = kids.len().max(1);
    let gap = 0.35 + 0.35 * (0.5 + 0.5 * (c.time * 0.6).sin());
    let thick = 10.0;
    let layer_h = thick * (1.0 + gap);
    let height = layer_h * layers.len() as f32;
    camera(c, st.cam, [0.0, 0.0, height * 0.5], 300.0 + height, st);
    let vp = st.vp;
    let grow = c.intro(1.0);
    for (s, sizes) in layers.iter().enumerate() {
        let z0 = s as f32 * layer_h;
        let z1 = z0 + thick;
        for (i, &k) in kids.iter().enumerate() {
            let r = (sizes[i] / peak).sqrt() * 110.0 * grow;
            if r < 0.5 {
                continue;
            }
            let (a0, a1) = (i as f32 / n as f32 * TAU + 0.02, (i + 1) as f32 / n as f32 * TAU - 0.02);
            let prev = if s == 0 { sizes[i] } else { layers[s - 1][i] };
            let base = c.col(k);
            let col = if c.paint == Paint::Growth && prev > 0.0 {
                let g = sizes[i] / prev;
                if g > 1.0 { mix(base, [1.0, 0.4, 0.2], ((g - 1.0) * 2.0).min(0.8)) } else { mix(base, [0.3, 0.55, 1.0], ((1.0 - g) * 3.0).min(0.8)) }
            } else {
                base
            };
            let col = scale(col, c.lit(k) * if s + 1 == layers.len() { 1.1 } else { 0.85 });
            let steps = 10;
            let at = |a: f32, rr: f32, z: f32| [a.sin() * rr, -a.cos() * rr, z];
            let mut top_outline = vec![];
            for j in 0..steps {
                let (b0, b1) = (a0 + (a1 - a0) * j as f32 / steps as f32, a0 + (a1 - a0) * (j + 1) as f32 / steps as f32);
                tri(o, [0.0, 0.0, z1], at(b0, r, z1), at(b1, r, z1), col, 1.0);
                tri(o, [0.0, 0.0, z0], at(b1, r, z0), at(b0, r, z0), col, 1.0);
                quad(o, at(b0, r, z0), at(b1, r, z0), at(b1, r, z1), at(b0, r, z1), scale(col, 0.9), 1.0);
                if let Some(p) = screen(c, &vp, at(b0, r, z1)) {
                    top_outline.push(p);
                }
            }
            quad(o, [0.0, 0.0, z0], at(a0, r, z0), at(a0, r, z1), [0.0, 0.0, z1], scale(col, 0.8), 1.0);
            quad(o, [0.0, 0.0, z0], at(a1, r, z0), at(a1, r, z1), [0.0, 0.0, z1], scale(col, 0.8), 1.0);
            if let (Some(p0), Some(p1)) = (screen(c, &vp, [0.0, 0.0, z1]), screen(c, &vp, at(a1, r, z1))) {
                top_outline.push(p1);
                top_outline.push(p0);
                o.hit(Shape::Poly(top_outline), k);
            }
            if s + 1 == layers.len() && r > 30.0 {
                let mid = (a0 + a1) * 0.5;
                if let Some(p) = screen(c, &vp, at(mid, r + 12.0, z1)) {
                    o.label(p[0] - 50.0, p[1] - 8.0, 100.0, c.tree.nodes[k].name.clone(), false);
                }
            }
        }
    }
    if c.history.is_empty() {
        o.label(14.0, 10.0, 520.0, "Only one disc: the earlier scans of this folder stack up here.".into(), false);
    }
}

/// The heightfield of the root, from its treemap.
#[derive(Default)]
pub(crate) struct Terrain {
    gw: usize,
    gh: usize,
    size: [f32; 2],
    z: Vec<f32>,
    col: Vec<[f32; 3]>,
    tiles: Vec<(usize, [f32; 4], f32)>,
}

fn build_terrain(c: &Ctx) -> Terrain {
    let size = [400.0f32, (400.0 * c.h / c.w.max(1.0)).clamp(200.0, 400.0)];
    let bounds = sprawl_core::Rect::new(0.0, 0.0, size[0] as f64, size[1] as f64);
    let tiles = sprawl_core::treemap(c.tree, c.root, bounds, 1.5, 4000);
    let (gw, gh) = (160usize, ((160.0 * size[1] / size[0]) as usize).max(40));
    let biggest = tiles.iter().map(|t| (c.size)(t.node)).max().unwrap_or(1).max(1) as f32;
    let mut z = vec![0.0f32; gw * gh];
    let mut col = vec![[0.2, 0.22, 0.3]; gw * gh];
    let mut out_tiles = Vec::with_capacity(tiles.len());
    for t in &tiles {
        let h = ((c.size)(t.node) as f32 / biggest).sqrt() * 70.0;
        let r = &t.rect;
        let (x0, x1) = ((r.x as f32 / size[0] * (gw - 1) as f32).ceil() as usize, ((r.x + r.w) as f32 / size[0] * (gw - 1) as f32).floor() as usize);
        let (y0, y1) = ((r.y as f32 / size[1] * (gh - 1) as f32).ceil() as usize, ((r.y + r.h) as f32 / size[1] * (gh - 1) as f32).floor() as usize);
        for gy in y0..=y1.min(gh - 1) {
            for gx in x0..=x1.min(gw - 1) {
                let (px, py) = (gx as f32 / (gw - 1) as f32 * size[0], gy as f32 / (gh - 1) as f32 * size[1]);
                let (u, v) = (((px - r.x as f32) / r.w as f32).clamp(0.0, 1.0), ((py - r.y as f32) / r.h as f32).clamp(0.0, 1.0));
                // a mesa: flat on top, sloping at the edges
                let bump = ((u * std::f32::consts::PI).sin() * (v * std::f32::consts::PI).sin()).powf(0.35);
                let i = gy * gw + gx;
                if h * bump > z[i] {
                    z[i] = h * bump;
                    col[i] = c.col(t.node);
                }
            }
        }
        out_tiles.push((t.node, [r.x as f32, r.y as f32, r.w as f32, r.h as f32], h));
    }
    // smooth once so the ridges flow into each other
    let mut s = z.clone();
    for gy in 1..gh - 1 {
        for gx in 1..gw - 1 {
            let i = gy * gw + gx;
            s[i] = z[i] * 0.6 + (z[i - 1] + z[i + 1] + z[i - gw] + z[i + gw]) * 0.1;
        }
    }
    Terrain { gw, gh, size, z: s, col, tiles: out_tiles }
}

pub(crate) fn ridgeline(c: &Ctx, o: &mut Out, st: &mut ViewState) {
    let fresh = st.terrain.as_ref().is_none_or(|t| (t.0, t.1, t.2) != (c.root, c.gen, c.paint));
    if fresh {
        st.terrain = Some((c.root, c.gen, c.paint, build_terrain(c)));
    }
    camera(c, st.cam, [0.0, 0.0, 0.0], 520.0, st);
    let vp = st.vp;
    let t = &st.terrain.as_ref().unwrap().3;
    let (sx, sy) = (t.size[0], t.size[1]);
    let grow = c.intro(1.2);
    let water = 4.0 + 1.2 * (c.time * 0.7).sin();
    let at = |gx: usize, gy: usize| -> [f32; 3] {
        let i = gy * t.gw + gx;
        [gx as f32 / (t.gw - 1) as f32 * sx - sx * 0.5, sy * 0.5 - gy as f32 / (t.gh - 1) as f32 * sy, t.z[i] * grow]
    };
    let colour = |gx: usize, gy: usize| -> [f32; 3] {
        let i = gy * t.gw + gx;
        let z = t.z[i] * grow;
        if z < water { mix(t.col[i], [0.1, 0.25, 0.45], 0.6) } else { t.col[i] }
    };
    for gy in 0..t.gh - 1 {
        for gx in 0..t.gw - 1 {
            let (a, b, cc, d) = (at(gx, gy), at(gx + 1, gy), at(gx + 1, gy + 1), at(gx, gy + 1));
            let col = scale(colour(gx, gy), 1.0);
            tri(o, a, d, cc, col, 1.0);
            tri(o, a, cc, b, col, 1.0);
        }
    }
    // the water line, a sheet that rises and falls
    let w = [[-sx * 0.5, -sy * 0.5, water], [sx * 0.5, -sy * 0.5, water], [sx * 0.5, sy * 0.5, water], [-sx * 0.5, sy * 0.5, water]];
    let wc = rgba([0.25, 0.5, 0.85], 0.28);
    for p in [w[0], w[1], w[2], w[0], w[2], w[3]] {
        o.tris.push(Vertex { pos: p, col: wc });
    }
    // tiles as hit shapes: their footprint on top of the land, biggest last so they win
    let mut tiles: Vec<&(usize, [f32; 4], f32)> = t.tiles.iter().collect();
    tiles.sort_by(|a, b| a.2.partial_cmp(&b.2).unwrap_or(std::cmp::Ordering::Equal));
    for (n, r, h) in tiles {
        let z = h * grow * 0.8;
        let corners = [[r[0], r[1]], [r[0] + r[2], r[1]], [r[0] + r[2], r[1] + r[3]], [r[0], r[1] + r[3]]];
        let poly: Option<Vec<[f32; 2]>> = corners.iter().map(|p| screen(c, &vp, [p[0] - sx * 0.5, sy * 0.5 - p[1], z])).collect();
        if let Some(poly) = poly {
            o.hit(Shape::Poly(poly), *n);
        }
    }
    // names on the highest peaks
    let mut peaks: Vec<&(usize, [f32; 4], f32)> = t.tiles.iter().collect();
    peaks.sort_by(|a, b| b.2.partial_cmp(&a.2).unwrap_or(std::cmp::Ordering::Equal));
    for (n, r, h) in peaks.into_iter().take(8) {
        let p = [r[0] + r[2] * 0.5 - sx * 0.5, sy * 0.5 - (r[1] + r[3] * 0.5), h * grow + 8.0];
        if let Some(s) = screen(c, &vp, p) {
            o.label(s[0] - 60.0, s[1] - 18.0, 120.0, c.tree.nodes[*n].name.clone(), false);
        }
    }
}

/// A rough sphere.
fn sphere(o: &mut Out, p: [f32; 3], r: f32, col: [f32; 3]) {
    let (rings, segs) = (7, 12);
    let at = |i: usize, j: usize| {
        let th = i as f32 / rings as f32 * std::f32::consts::PI;
        let ph = j as f32 / segs as f32 * TAU;
        [p[0] + r * th.sin() * ph.cos(), p[1] + r * th.sin() * ph.sin(), p[2] + r * th.cos()]
    };
    for i in 0..rings {
        for j in 0..segs {
            quad(o, at(i, j), at(i + 1, j), at(i + 1, j + 1), at(i, j + 1), col, 1.0);
        }
    }
}

/// A thin six-sided rod from `a` to `b`.
fn rod(o: &mut Out, a: [f32; 3], b: [f32; 3], r: f32, col: [f32; 3]) {
    let d = norm(sub(b, a));
    let up = if d[2].abs() < 0.9 { [0.0, 0.0, 1.0] } else { [1.0, 0.0, 0.0] };
    let u = norm(cross(d, up));
    let v = cross(d, u);
    let ring = |c: [f32; 3], k: usize| {
        let ang = k as f32 / 6.0 * TAU;
        [c[0] + (u[0] * ang.cos() + v[0] * ang.sin()) * r, c[1] + (u[1] * ang.cos() + v[1] * ang.sin()) * r, c[2] + (u[2] * ang.cos() + v[2] * ang.sin()) * r]
    };
    for k in 0..6 {
        quad(o, ring(a, k), ring(b, k), ring(b, k + 1), ring(a, k + 1), col, 1.0);
    }
}

/// Folders and big files as atoms, containment as bonds; a force layout in 3D.
pub(crate) fn molecule(c: &Ctx, o: &mut Out, st: &mut ViewState) {
    // the atoms: breadth first from the root, the biggest children of each, about 120 in all
    let atoms = super::memo("atoms", c, || {
        let mut list: Vec<(usize, Option<usize>)> = vec![(c.root, None)];
        let mut q = std::collections::VecDeque::from([(c.root, 0usize)]);
        while let Some((n, d)) = q.pop_front() {
            if d >= 5 {
                continue;
            }
            for k in c.kids(n).into_iter().take(6) {
                if list.len() >= 120 {
                    break;
                }
                if (c.size)(k) as f32 / (c.size)(c.root).max(1) as f32 > 0.0015 {
                    list.push((k, Some(n)));
                    if c.tree.nodes[k].is_dir {
                        q.push_back((k, d + 1));
                    }
                }
            }
        }
        list
    });
    let total = (c.size)(c.root).max(1) as f32;
    let radius = |n: usize| 3.0 + ((c.size)(n) as f32 / total).cbrt() * 30.0;
    if st.atoms_for != Some((c.root, c.gen)) {
        st.atoms_for = Some((c.root, c.gen));
        st.atoms.clear();
        for (i, (n, _)) in atoms.iter().enumerate() {
            let s = i as u64 * 3;
            st.atoms.insert(*n, [(hash01(s) - 0.5) * 200.0, (hash01(s + 1) - 0.5) * 200.0, (hash01(s + 2) - 0.5) * 200.0, 0.0, 0.0, 0.0]);
        }
        st.atoms.insert(c.root, [0.0; 6]);
    }
    // a few steps of the layout each frame: bonds pull, everything repels, the root stays put
    for _ in 0..4 {
        let snapshot: Vec<(usize, [f32; 3])> = atoms.iter().map(|(n, _)| (*n, { let a = st.atoms[n]; [a[0], a[1], a[2]] })).collect();
        let mut force: std::collections::HashMap<usize, [f32; 3]> = snapshot.iter().map(|(n, _)| (*n, [0.0; 3])).collect();
        for i in 0..snapshot.len() {
            for j in i + 1..snapshot.len() {
                let d = sub(snapshot[j].1, snapshot[i].1);
                let l2 = dot(d, d).max(25.0);
                let f = 2400.0 / l2;
                let l = l2.sqrt();
                let push = [d[0] / l * f, d[1] / l * f, d[2] / l * f];
                let fi = force.get_mut(&snapshot[i].0).unwrap();
                fi[0] -= push[0];
                fi[1] -= push[1];
                fi[2] -= push[2];
                let fj = force.get_mut(&snapshot[j].0).unwrap();
                fj[0] += push[0];
                fj[1] += push[1];
                fj[2] += push[2];
            }
        }
        for (n, p) in atoms.iter() {
            let Some(p) = p else { continue };
            let (a, b) = (st.atoms[n], st.atoms[p]);
            let d = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
            let l = dot(d, d).sqrt().max(0.1);
            let rest = radius(*n) + radius(*p) + 14.0;
            let k = (l - rest) * 0.05;
            let f = [d[0] / l * k, d[1] / l * k, d[2] / l * k];
            let fa = force.get_mut(n).unwrap();
            fa[0] += f[0];
            fa[1] += f[1];
            fa[2] += f[2];
            let fb = force.get_mut(p).unwrap();
            fb[0] -= f[0];
            fb[1] -= f[1];
            fb[2] -= f[2];
        }
        for (n, _) in atoms.iter() {
            if *n == c.root {
                continue;
            }
            let f = force[n];
            let a = st.atoms.get_mut(n).unwrap();
            for k in 0..3 {
                a[3 + k] = (a[3 + k] + f[k].clamp(-20.0, 20.0)) * 0.6;
                a[k] += a[3 + k];
            }
        }
    }
    // stand back far enough to see every atom
    let spread = st.atoms.values().map(|a| (a[0] * a[0] + a[1] * a[1] + a[2] * a[2]).sqrt()).fold(100.0f32, f32::max);
    camera(c, st.cam, [0.0, 0.0, 0.0], spread * 2.6, st);
    let vp = st.vp;
    let grow = c.intro(0.8);
    for (n, p) in atoms.iter() {
        let a = st.atoms[n];
        let pa = [a[0] * grow, a[1] * grow, a[2] * grow];
        if let Some(p) = p {
            let b = st.atoms[p];
            rod(o, pa, [b[0] * grow, b[1] * grow, b[2] * grow], 1.2, [0.62, 0.66, 0.74]);
        }
        let r = radius(*n);
        sphere(o, pa, r, scale(c.col(*n), c.lit(*n)));
        if let Some(s) = screen(c, &vp, pa) {
            // apparent size: project a point one radius to the side
            let e = st.eye;
            let side = norm(cross(sub(pa, e), [0.0, 0.0, 1.0]));
            let edge = screen(c, &vp, [pa[0] + side[0] * r, pa[1] + side[1] * r, pa[2] + side[2] * r]).unwrap_or(s);
            let rs = ((edge[0] - s[0]).powi(2) + (edge[1] - s[1]).powi(2)).sqrt();
            o.hit(Shape::Circle(s, rs.max(4.0)), *n);
            if rs > 14.0 {
                o.label(s[0] + rs + 2.0, s[1] - 8.0, 120.0, c.tree.nodes[*n].name.clone(), *n == c.root);
            }
        }
    }
}
