//! Batch 4 (2D): Tideline (a streamgraph across scans), Tree Rings (each big folder a log, one
//! ring per scan) and Isobars (a weather map of what changed), from the earlier scans.

use std::f32::consts::TAU;

use super::{hash01, mix, rgba, scale, Ctx, Out, Shape};

/// When there is nothing to compare yet.
fn no_history(c: &Ctx, o: &mut Out) -> bool {
    if !c.history.is_empty() {
        return false;
    }
    o.label(c.w * 0.5 - 220.0, c.h * 0.5 - 18.0, 440.0, "No earlier scans of this folder yet.".into(), true);
    o.label(c.w * 0.5 - 220.0, c.h * 0.5 + 2.0, 440.0, "Each scan is kept, so the next one shows how it changed.".into(), false);
    true
}

/// The root's biggest children now, and their sizes at each scan (oldest first, now last).
fn series(c: &Ctx, most: usize) -> (Vec<usize>, Vec<Vec<f32>>) {
    let kids: Vec<usize> = c.kids(c.root).into_iter().take(most).collect();
    let mut cols: Vec<Vec<f32>> = c.history.iter().map(|(_, v)| kids.iter().map(|&k| v.get(k).copied().unwrap_or(0) as f32).collect()).collect();
    cols.push(kids.iter().map(|&k| (c.size)(k) as f32).collect());
    (kids, cols)
}

/// Catmull-Rom through `p` at `t` in 0..p.len()-1.
fn spline(p: &[f32], t: f32) -> f32 {
    let n = p.len();
    if n == 1 {
        return p[0];
    }
    let i = (t.floor() as usize).min(n - 2);
    let u = t - i as f32;
    let at = |k: isize| p[k.clamp(0, n as isize - 1) as usize];
    let (p0, p1, p2, p3) = (at(i as isize - 1), at(i as isize), at(i as isize + 1), at(i as isize + 2));
    0.5 * ((2.0 * p1) + (-p0 + p2) * u + (2.0 * p0 - 5.0 * p1 + 4.0 * p2 - p3) * u * u + (-p0 + 3.0 * p1 - 3.0 * p2 + p3) * u * u * u)
}

fn when(t: std::time::SystemTime) -> String {
    crate::format::fmt_time(t).get(5..10).unwrap_or("").to_string()
}

pub(crate) fn tideline(c: &Ctx, o: &mut Out) {
    if no_history(c, o) {
        return;
    }
    let (kids, cols) = series(c, 10);
    let steps = cols.len();
    let (left, right, top, bottom) = (20.0, c.w - 20.0, 26.0, c.h - 26.0);
    let peak = cols.iter().map(|col| col.iter().sum::<f32>()).fold(1.0f32, f32::max);
    let k = (bottom - top) / peak;
    let mid = (top + bottom) * 0.5;
    let samples = (steps - 1).max(1) * 16;
    let xs = |s: usize| left + (right - left) * s as f32 / samples as f32;
    let grow = c.intro(1.2);
    // per stream, its lower and upper edge at every sample (centred: a silhouette)
    let mut lower = vec![vec![0.0f32; samples + 1]; kids.len()];
    let mut upper = lower.clone();
    for s in 0..=samples {
        let t = s as f32 / samples as f32 * (steps - 1) as f32;
        let vals: Vec<f32> = (0..kids.len()).map(|i| spline(&cols.iter().map(|col| col[i]).collect::<Vec<_>>(), t).max(0.0) * k * grow).collect();
        let mut y = mid - vals.iter().sum::<f32>() * 0.5;
        for i in 0..kids.len() {
            lower[i][s] = y;
            y += vals[i];
            upper[i][s] = y;
        }
    }
    for (i, &n) in kids.iter().enumerate() {
        let col = scale(c.col(n), c.lit(n));
        for s in 0..samples {
            let (x0, x1) = (xs(s), xs(s + 1));
            o.quad([x0, lower[i][s]], [x1, lower[i][s + 1]], [x1, upper[i][s + 1]], [x0, upper[i][s]], rgba(col, 0.88));
        }
        let mut poly: Vec<[f32; 2]> = (0..=samples).step_by(4).map(|s| [xs(s), upper[i][s]]).collect();
        poly.extend((0..=samples).rev().step_by(4).map(|s| [xs(s), lower[i][s]]));
        o.hit(Shape::Poly(poly), n);
        // a name where the stream is widest
        let (sw, wide) = (0..=samples).map(|s| (s, upper[i][s] - lower[i][s])).fold((0, 0.0), |a, b| if b.1 > a.1 { b } else { a });
        if wide > 16.0 {
            o.label(xs(sw) - 60.0, (upper[i][sw] + lower[i][sw]) * 0.5 - 8.0, 120.0, c.tree.nodes[n].name.clone(), false);
        }
    }
    // the playhead sweeping through time
    let ph = left + (right - left) * ((c.time * 0.08).fract());
    o.rect(ph - 1.0, top - 6.0, 2.0, bottom - top + 12.0, [1.0, 1.0, 1.0, 0.5]);
    for (i, (t, _)) in c.history.iter().enumerate() {
        let x = left + (right - left) * i as f32 / (steps - 1).max(1) as f32;
        o.label(x - 30.0, bottom + 6.0, 60.0, when(*t), false);
    }
    o.label(right - 40.0, bottom + 6.0, 60.0, "Now".into(), true);
}

pub(crate) fn tree_rings(c: &Ctx, o: &mut Out) {
    if no_history(c, o) {
        return;
    }
    let (kids, cols) = series(c, 6);
    let steps = cols.len();
    let n = kids.len().max(1);
    let slot = (c.w - 20.0) / n as f32;
    let max_r = (slot * 0.45).min(c.h * 0.42);
    let peak = kids.iter().enumerate().map(|(i, _)| cols.iter().map(|col| col[i]).fold(0.0, f32::max)).fold(1.0, f32::max);
    // the rings grow back every few seconds, oldest first
    let cycle = (c.time % 9.0) / 6.0;
    for (i, &k) in kids.iter().enumerate() {
        let centre = [10.0 + slot * (i as f32 + 0.5), c.h * 0.46];
        let seed = hash01(k as u64);
        let radius = |v: f32| (v / peak).sqrt() * max_r;
        let shape = |a: f32, s: usize| 1.0 + 0.035 * (5.0 * a + seed * 9.0 + s as f32 * 0.4).sin() + 0.02 * (9.0 * a + seed * 4.0).sin();
        // outside in: each ring a filled wobbly disc, the next one on top
        for s in (0..steps).rev() {
            let shown = ((cycle * steps as f32) - s as f32).clamp(0.0, 1.0);
            if shown <= 0.0 {
                continue;
            }
            let r = radius(cols[s][i]) * shown;
            let prev = if s == 0 { cols[s][i] } else { cols[s - 1][i] };
            let ratio = if prev > 0.0 { cols[s][i] / prev } else { 2.0 };
            // warm for growth, pale for none, cool for shrinkage
            let col = if ratio > 1.0 { mix([0.75, 0.62, 0.42], [1.0, 0.42, 0.2], ((ratio - 1.0) * 2.0).min(1.0)) } else { mix([0.75, 0.62, 0.42], [0.35, 0.6, 0.9], ((1.0 - ratio) * 3.0).min(1.0)) };
            let col = scale(col, if s % 2 == 0 { 1.0 } else { 0.86 });
            let steps_a = 72;
            for a in 0..steps_a {
                let (a0, a1) = (a as f32 / steps_a as f32 * TAU, (a + 1) as f32 / steps_a as f32 * TAU);
                let p0 = [centre[0] + a0.sin() * r * shape(a0, s), centre[1] - a0.cos() * r * shape(a0, s)];
                let p1 = [centre[0] + a1.sin() * r * shape(a1, s), centre[1] - a1.cos() * r * shape(a1, s)];
                o.tri(centre, p0, p1, rgba(col, 1.0));
            }
        }
        let r = radius(cols[steps - 1][i]);
        o.hit(Shape::Circle(centre, r.max(6.0)), k);
        o.label(centre[0] - slot * 0.45, centre[1] + max_r + 6.0, slot * 0.9, c.tree.nodes[k].name.clone(), true);
        let first = cols[0][i];
        let now = cols[steps - 1][i];
        let note = if first > 0.0 { format!("{}  ({:+.0} %)", (c.fmt)(now as u64), (now / first - 1.0) * 100.0) } else { (c.fmt)(now as u64) };
        o.label(centre[0] - slot * 0.45, centre[1] + max_r + 22.0, slot * 0.9, note, false);
    }
}

pub(crate) fn isobars(c: &Ctx, o: &mut Out) {
    if no_history(c, o) {
        return;
    }
    let kids: Vec<usize> = c.kids(c.root).into_iter().take(40).collect();
    let rect = sprawl_core::Rect::new(10.0, 10.0, (c.w - 20.0) as f64, (c.h - 20.0) as f64);
    let rects = sprawl_core::squarify(&kids.iter().map(|&k| (c.size)(k) as f64).collect::<Vec<_>>(), rect);
    // which step plays: the change from one scan to the next, two seconds each
    let steps = c.history.len();
    let step = ((c.time / 2.5) as usize) % steps;
    let before = &c.history[step].1;
    let after = |k: usize| if step + 1 < steps { c.history[step + 1].1.get(k).copied().unwrap_or(0) } else { (c.size)(k) };
    let label_to = if step + 1 < steps { when(c.history[step + 1].0) } else { "now".into() };
    // pressure: growth splatted around each tile's middle
    let cell = 8.0f32;
    let (gw, gh) = (((c.w - 20.0) / cell) as usize + 1, ((c.h - 20.0) / cell) as usize + 1);
    let mut field = vec![0.0f32; gw * gh];
    let scale_b = (c.size)(c.root).max(1) as f32 * 0.02;
    for (i, &k) in kids.iter().enumerate() {
        let r = &rects[i];
        let d = (after(k) as f32 - before.get(k).copied().unwrap_or(0) as f32) / scale_b;
        if d.abs() < 1e-4 {
            continue;
        }
        let (cx, cy) = ((r.x + r.w / 2.0) as f32, (r.y + r.h / 2.0) as f32);
        let sigma = ((r.w * r.h) as f32).sqrt() * 0.45 + 20.0;
        for gy in 0..gh {
            for gx in 0..gw {
                let (px, py) = (10.0 + gx as f32 * cell, 10.0 + gy as f32 * cell);
                let e = ((px - cx).powi(2) + (py - cy).powi(2)) / (2.0 * sigma * sigma);
                if e < 6.0 {
                    field[gy * gw + gx] += d * (-e).exp();
                }
            }
        }
    }
    let band = |v: f32| (v * 6.0).round().clamp(-6.0, 6.0) as i32;
    for gy in 0..gh {
        for gx in 0..gw {
            let b = band(field[gy * gw + gx]);
            let t = b as f32 / 6.0;
            let col = if t >= 0.0 { mix([0.16, 0.17, 0.22], [1.0, 0.36, 0.2], t) } else { mix([0.16, 0.17, 0.22], [0.25, 0.55, 1.0], -t) };
            let (px, py) = (10.0 + gx as f32 * cell, 10.0 + gy as f32 * cell);
            o.rect(px, py, cell + 0.5, cell + 0.5, rgba(col, 0.9));
            // isobars where the band changes
            if gx + 1 < gw && band(field[gy * gw + gx + 1]) != b {
                o.line([px + cell, py], [px + cell, py + cell], [1.0, 1.0, 1.0, 0.45]);
            }
            if gy + 1 < gh && band(field[(gy + 1) * gw + gx]) != b {
                o.line([px, py + cell], [px + cell, py + cell], [1.0, 1.0, 1.0, 0.45]);
            }
        }
    }
    // H and L at the strongest growth and shrinkage
    let (mut hi, mut lo) = ((0usize, 0.0f32), (0usize, 0.0f32));
    for (i, &v) in field.iter().enumerate() {
        if v > hi.1 {
            hi = (i, v);
        }
        if v < lo.1 {
            lo = (i, v);
        }
    }
    for ((i, v), mark) in [(hi, "H"), (lo, "L")] {
        if v.abs() > 0.05 {
            let (px, py) = (10.0 + (i % gw) as f32 * cell, 10.0 + (i / gw) as f32 * cell);
            o.label(px - 6.0, py - 10.0, 30.0, mark.into(), true);
        }
    }
    for (i, &k) in kids.iter().enumerate() {
        let r = &rects[i];
        o.hit(Shape::Rect([r.x as f32, r.y as f32, r.w as f32, r.h as f32]), k);
        if r.w > 80.0 && r.h > 30.0 {
            o.label(r.x as f32 + 6.0, r.y as f32 + 4.0, r.w as f32 - 10.0, c.tree.nodes[k].name.clone(), false);
        }
        o.line([r.x as f32, r.y as f32], [(r.x + r.w) as f32, r.y as f32], [0.0, 0.0, 0.0, 0.25]);
        o.line([r.x as f32, r.y as f32], [r.x as f32, (r.y + r.h) as f32], [0.0, 0.0, 0.0, 0.25]);
    }
    o.label(16.0, c.h - 28.0, 400.0, format!("{} to {label_to}", when(c.history[step].0)), true);
}
