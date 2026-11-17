//! Batch 2: Suds (bubbles that settle), Stained Glass (a Voronoi treemap that relaxes as you
//! watch) and Spectrum (file types as a circular equaliser).

use std::collections::HashMap;
use std::f32::consts::TAU;

use super::{hash01, memo, mix, rgba, scale, Ctx, Out, Paint, Shape};

/// The root's biggest children (at most `most`), and what the rest add up to.
fn top_kids(c: &Ctx, most: usize) -> Vec<usize> {
    let mut k = c.kids(c.root);
    k.truncate(most);
    k
}

/// Folders as soap bubbles: area is size; they drift to the middle and push each other apart.
pub(crate) fn suds(c: &Ctx, o: &mut Out, state: &mut HashMap<usize, [f32; 4]>) {
    let kids = top_kids(c, 42);
    let total: f32 = kids.iter().map(|&k| (c.size)(k) as f32).sum::<f32>().max(1.0);
    let area = c.w * c.h * 0.36;
    let centre = [c.w * 0.5, c.h * 0.5];
    let radius = |k: usize| ((c.size)(k) as f32 / total * area / std::f32::consts::PI).sqrt().max(4.0);
    state.retain(|k, _| kids.contains(k));
    for &k in &kids {
        state.entry(k).or_insert_with(|| {
            let a = hash01(k as u64) * TAU;
            let d = 0.35 + 0.3 * hash01(k as u64 + 1);
            [centre[0] + a.cos() * c.w * d * 0.5, centre[1] + a.sin() * c.h * d * 0.5, 0.0, -2.0 - 3.0 * hash01(k as u64 + 2)]
        });
    }
    // a few steps of the toy physics: pulled to the middle, pushed apart, damped
    for _ in 0..3 {
        for &k in &kids {
            let b = state.get_mut(&k).unwrap();
            b[2] += (centre[0] - b[0]) * 0.0016;
            b[3] += (centre[1] - b[1]) * 0.0022;
        }
        for i in 0..kids.len() {
            for j in i + 1..kids.len() {
                let (ri, rj) = (radius(kids[i]), radius(kids[j]));
                let (a, b) = (state[&kids[i]], state[&kids[j]]);
                let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
                let d = (dx * dx + dy * dy).sqrt().max(0.01);
                let overlap = ri + rj + 3.0 - d;
                if overlap > 0.0 {
                    let (nx, ny) = (dx / d, dy / d);
                    let share = rj / (ri + rj);
                    let push = overlap * 0.5;
                    if let Some(p) = state.get_mut(&kids[i]) {
                        p[0] -= nx * push * share;
                        p[1] -= ny * push * share;
                    }
                    if let Some(p) = state.get_mut(&kids[j]) {
                        p[0] += nx * push * (1.0 - share);
                        p[1] += ny * push * (1.0 - share);
                    }
                }
            }
        }
        for &k in &kids {
            let r = radius(k);
            let b = state.get_mut(&k).unwrap();
            b[2] *= 0.86;
            b[3] *= 0.86;
            b[0] = (b[0] + b[2]).clamp(r, (c.w - r).max(r));
            b[1] = (b[1] + b[3]).clamp(r.min(c.h * 0.5), (c.h - r).max(c.h * 0.5));
        }
    }
    let wobble = |k: usize| 1.0 + 0.012 * (c.time * 1.7 + hash01(k as u64) * 6.0).sin();
    for &k in kids.iter().rev() {
        let b = state[&k];
        let r = radius(k) * wobble(k);
        let p = [b[0], b[1]];
        let base = scale(c.col(k), c.lit(k));
        // the film: clear in the middle, tinted towards the edge
        o.disc(p, r, rgba(base, 0.22));
        o.wedge(p, r * 0.82, r, 0.0, TAU, rgba(base, 0.35));
        // in Risk colour, what can be won back fills it from the bottom
        let share = if c.paint == Paint::Risk { (c.reclaim_of(k) as f32 / (c.size)(k).max(1) as f32).clamp(0.0, 1.0) } else { 0.0 };
        if share > 0.0 {
            let level = p[1] + r - 2.0 * r * share;
            let strips = 24;
            for s in 0..strips {
                let y0 = p[1] - r + 2.0 * r * s as f32 / strips as f32;
                let y1 = y0 + 2.0 * r / strips as f32;
                if y1 < level {
                    continue;
                }
                let ym = (y0.max(level) + y1) * 0.5;
                let half = (r * r - (ym - p[1]).powi(2)).max(0.0).sqrt();
                let wave = 1.5 * (c.time * 2.0 + p[0] * 0.05).sin();
                o.rect(p[0] - half, y0.max(level + wave), half * 2.0, y1 - y0.max(level + wave), [0.30, 0.78, 0.50, 0.55]);
            }
        }
        o.circle_line(p, r, rgba(scale(base, 1.4), 0.75));
        // a highlight, top left
        o.disc([p[0] - r * 0.38, p[1] - r * 0.42], r * 0.16, [1.0, 1.0, 1.0, 0.16]);
        o.disc([p[0] - r * 0.44, p[1] - r * 0.48], r * 0.07, [1.0, 1.0, 1.0, 0.35]);
        o.hit(Shape::Circle(p, r), k);
        if r > 30.0 {
            let w = r * 1.6;
            o.label(p[0] - w * 0.5, p[1] - 15.0, w, c.tree.nodes[k].name.clone(), true);
            o.label(p[0] - w * 0.5, p[1] + 1.0, w, (c.fmt)((c.size)(k)), false);
        }
    }
}

/// A convex polygon clipped to the half-plane where `a·p <= b`.
fn clip(poly: &[[f32; 2]], a: [f32; 2], b: f32) -> Vec<[f32; 2]> {
    let mut out = Vec::with_capacity(poly.len() + 2);
    let inside = |p: [f32; 2]| a[0] * p[0] + a[1] * p[1] <= b;
    for i in 0..poly.len() {
        let (p, q) = (poly[i], poly[(i + 1) % poly.len()]);
        let (ip, iq) = (inside(p), inside(q));
        if ip {
            out.push(p);
        }
        if ip != iq {
            let (fp, fq) = (a[0] * p[0] + a[1] * p[1] - b, a[0] * q[0] + a[1] * q[1] - b);
            let t = fp / (fp - fq);
            out.push([p[0] + (q[0] - p[0]) * t, p[1] + (q[1] - p[1]) * t]);
        }
    }
    out
}

fn area_centroid(p: &[[f32; 2]]) -> (f32, [f32; 2]) {
    let (mut a, mut cx, mut cy) = (0.0, 0.0, 0.0);
    for i in 0..p.len() {
        let (u, v) = (p[i], p[(i + 1) % p.len()]);
        let cr = u[0] * v[1] - v[0] * u[1];
        a += cr;
        cx += (u[0] + v[0]) * cr;
        cy += (u[1] + v[1]) * cr;
    }
    if a.abs() < 1e-3 {
        return (0.0, p.first().copied().unwrap_or([0.0, 0.0]));
    }
    (a * 0.5, [cx / (3.0 * a), cy / (3.0 * a)])
}

/// The power diagram of the seeds inside `rect`: one convex cell per seed.
fn power_cells(seeds: &[(usize, [f32; 2], f32)], rect: [f32; 4]) -> Vec<Vec<[f32; 2]>> {
    let [x, y, w, h] = rect;
    let frame = vec![[x, y], [x + w, y], [x + w, y + h], [x, y + h]];
    seeds
        .iter()
        .enumerate()
        .map(|(i, (_, p, wi))| {
            let mut cell = frame.clone();
            for (j, (_, q, wj)) in seeds.iter().enumerate() {
                if i == j || cell.is_empty() {
                    continue;
                }
                // |x-p|²-wi <= |x-q|²-wj  ⇔  2(q-p)·x <= |q|²-|p|² - wj + wi
                let a = [2.0 * (q[0] - p[0]), 2.0 * (q[1] - p[1])];
                let b = q[0] * q[0] + q[1] * q[1] - p[0] * p[0] - p[1] * p[1] - wj + wi;
                cell = clip(&cell, a, b);
            }
            cell
        })
        .collect()
}

/// A Voronoi treemap: each child of the root a pane whose area follows its size, relaxing every
/// frame (weights adjust, seeds move to their pane's middle).
pub(crate) fn glass(c: &Ctx, o: &mut Out, seeds: &mut Vec<(usize, [f32; 2], f32)>, made_for: &mut Option<(usize, u64)>) {
    let kids = top_kids(c, 60);
    let rect = [10.0, 10.0, c.w - 20.0, c.h - 20.0];
    if *made_for != Some((c.root, c.gen)) || seeds.len() != kids.len() {
        *made_for = Some((c.root, c.gen));
        seeds.clear();
        for &k in &kids {
            let p = [rect[0] + rect[2] * (0.05 + 0.9 * hash01(k as u64 * 3 + 1)), rect[1] + rect[3] * (0.05 + 0.9 * hash01(k as u64 * 3 + 2))];
            seeds.push((k, p, 0.0));
        }
    }
    let total: f32 = kids.iter().map(|&k| (c.size)(k) as f32).sum::<f32>().max(1.0);
    let whole = rect[2] * rect[3];
    let mean = whole / kids.len().max(1) as f32;
    let mut cells = power_cells(seeds, rect);
    for _ in 0..4 {
        for (i, s) in seeds.iter_mut().enumerate() {
            let target = (c.size)(s.0) as f32 / total * whole;
            let (a, centre) = area_centroid(&cells[i]);
            if a <= 1.0 {
                // squeezed out: give it room again
                s.2 *= 0.5;
                continue;
            }
            s.1 = [s.1[0] + (centre[0] - s.1[0]) * 0.6, s.1[1] + (centre[1] - s.1[1]) * 0.6];
            s.2 += (target - a) / target.max(1.0) * mean * 0.35;
        }
        // keep each seed inside its own pane: a weight may not outgrow the distance to a neighbour
        for i in 0..seeds.len() {
            for j in 0..seeds.len() {
                if i == j {
                    continue;
                }
                let d2 = (seeds[i].1[0] - seeds[j].1[0]).powi(2) + (seeds[i].1[1] - seeds[j].1[1]).powi(2);
                if seeds[i].2 - seeds[j].2 > d2 * 0.9 {
                    seeds[i].2 = seeds[j].2 + d2 * 0.9;
                }
            }
        }
        cells = power_cells(seeds, rect);
    }
    let shimmer = |k: usize| 1.0 + 0.05 * (c.time * 0.9 + hash01(k as u64) * 9.0).sin();
    for (i, cell) in cells.iter().enumerate() {
        if cell.len() < 3 {
            continue;
        }
        let k = seeds[i].0;
        let base = scale(c.col(k), c.lit(k) * shimmer(k));
        let (a, centre) = area_centroid(cell);
        // glass: lighter towards the top left of each pane
        let light = |p: [f32; 2]| {
            let t = ((p[0] - centre[0]) * -0.6 + (p[1] - centre[1]) * -0.8) / a.abs().sqrt().max(1.0);
            rgba(scale(base, 1.0 + t.clamp(-0.5, 0.5) * 0.5), 0.92)
        };
        for j in 1..cell.len() - 1 {
            o.tri3([cell[0], cell[j], cell[j + 1]], [light(cell[0]), light(cell[j]), light(cell[j + 1])]);
        }
        // lead lines
        for j in 0..cell.len() {
            o.stroke(cell[j], cell[(j + 1) % cell.len()], 3.0, [0.06, 0.06, 0.08, 0.95]);
        }
        o.hit(Shape::Poly(cell.clone()), k);
        let side = a.abs().sqrt();
        if side > 70.0 {
            let w = side * 0.9;
            o.label(centre[0] - w * 0.5, centre[1] - 15.0, w, c.tree.nodes[k].name.clone(), true);
            o.label(centre[0] - w * 0.5, centre[1] + 1.0, w, (c.fmt)((c.size)(k)), false);
        }
    }
}

/// A file type's family, for grouping the bars.
use sprawl_core::kinds::family;

/// Every file type as a bar around a ring, grouped by family; length is the square root of bytes.
pub(crate) fn spectrum(c: &Ctx, o: &mut Out) {
    let stats = memo("exts", c, || {
        let mut by: HashMap<String, (u64, u64)> = HashMap::new();
        let mut stack = vec![c.root];
        while let Some(n) = stack.pop() {
            for &k in &c.tree.nodes[n].children {
                if c.tree.nodes[k].is_dir {
                    stack.push(k);
                } else {
                    let e = by.entry(c.tree.nodes[k].extension()).or_default();
                    e.0 += (c.size)(k);
                    e.1 += 1;
                }
            }
        }
        let mut v: Vec<(String, u64, u64)> = by.into_iter().map(|(e, (b, n))| (e, b, n)).filter(|x| x.1 > 0).collect();
        v.sort_by_key(|x| std::cmp::Reverse(x.1));
        v.truncate(96);
        v.sort_by(|a, b| (family(&a.0), std::cmp::Reverse(a.1)).cmp(&(family(&b.0), std::cmp::Reverse(b.1))));
        v
    });
    if stats.is_empty() {
        return;
    }
    let centre = [c.w * 0.5, c.h * 0.5];
    // a ring stretched to use a wide panel
    let ex = (c.w / c.h).clamp(1.0, 2.4);
    let inner = c.h * 0.2;
    let reach = c.h * 0.5 - inner - 22.0;
    let max = stats.iter().map(|x| x.1).max().unwrap_or(1) as f32;
    let n = stats.len();
    let step = TAU / n as f32;
    let grow = c.intro(0.9);
    let ring: Vec<[f32; 2]> = (0..=96).map(|k| {
        let a = k as f32 / 96.0 * TAU;
        [centre[0] + a.sin() * (inner - 4.0) * ex, centre[1] - a.cos() * (inner - 4.0)]
    }).collect();
    o.polyline(&ring, [1.0, 1.0, 1.0, 0.15]);
    let total: u64 = stats.iter().map(|x| x.1).sum();
    o.label(centre[0] - inner * 0.8, centre[1] - 16.0, inner * 1.6, format!("{} file types", n), true);
    o.label(centre[0] - inner * 0.8, centre[1] + 1.0, inner * 1.6, (c.fmt)(total), false);
    for (i, (ext, bytes, count)) in stats.iter().enumerate() {
        let a = i as f32 * step + step * 0.5;
        let len = (*bytes as f32 / max).sqrt() * reach * grow;
        // the bars bounce like a music visualiser; the cap marks the true length
        let bounce = 0.82 + 0.18 * (c.time * (2.0 + hash01(i as u64) * 3.0) + i as f32).sin().abs();
        let dir = [a.sin(), -a.cos()];
        let at = |r: f32| [centre[0] + dir[0] * r * ex, centre[1] + dir[1] * r];
        let col = (c.ext_colour)(ext);
        let width = (step * inner * 0.75).clamp(2.0, 14.0);
        o.stroke(at(inner), at(inner + len * bounce), width, rgba(col, 0.9));
        o.stroke(at(inner + len - 1.0), at(inner + len + 1.5), width + 1.0, rgba(mix(col, [1.0, 1.0, 1.0], 0.6), 1.0));
        let half = width * 0.5 + 1.0;
        let dl = (dir[0] * dir[0] * ex * ex + dir[1] * dir[1]).sqrt();
        let side = [-dir[1] / dl * half, dir[0] * ex / dl * half];
        let (p, q) = (at(inner), at(inner + len.max(6.0) + 3.0));
        let quad = vec![[p[0] + side[0], p[1] + side[1]], [q[0] + side[0], q[1] + side[1]], [q[0] - side[0], q[1] - side[1]], [p[0] - side[0], p[1] - side[1]]];
        let name = if ext.is_empty() { "(no extension)".to_string() } else { format!(".{ext}") };
        o.tip(Shape::Poly(quad), format!("{name}   {}   ·   {count} files", (c.fmt)(*bytes)));
        if i < n && *bytes as f32 > max * 0.08 {
            let lp = at(inner + len + 14.0);
            let w = 70.0;
            let lx = if dir[0] < -0.2 { lp[0] - w } else if dir[0] > 0.2 { lp[0] } else { lp[0] - w * 0.5 };
            o.label(lx, lp[1] - 8.0, w, name, false);
        }
    }
}
