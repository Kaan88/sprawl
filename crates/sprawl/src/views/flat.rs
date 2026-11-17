//! Batch 1, the 2D partitions: Rings (a sunburst), Flame Stack (an icicle grown upwards), Waffle
//! (equal squares) and Library (the biggest files as books).

use std::f32::consts::TAU;

use super::{hash01, mix, rgba, scale, Ctx, Out, Shape};

/// One ring per level around the root; the centre goes back up.
pub(crate) fn rings(c: &Ctx, o: &mut Out) {
    let centre = [c.w * 0.5, c.h * 0.5];
    let outer = (c.w.min(c.h) * 0.5 - 14.0).max(40.0);
    let core = outer * 0.17;
    let levels = 6usize;
    let ring = (outer - core) / levels as f32;
    let total = (c.size)(c.root).max(1) as f32;
    // the rings sweep open when the view or its root appears
    let sweep = c.intro(0.7);
    o.disc(centre, core, rgba(scale(c.col(c.root), 0.55 * c.lit(c.root)), 0.95));
    o.circle_line(centre, core, [1.0, 1.0, 1.0, 0.18]);
    o.hit(Shape::Circle(centre, core), c.root);
    let name = &c.tree.nodes[c.root].name;
    o.label(centre[0] - core * 0.85, centre[1] - 16.0, core * 1.7, name.clone(), true);
    o.label(centre[0] - core * 0.85, centre[1] + 1.0, core * 1.7, (c.fmt)((c.size)(c.root)), false);
    let mut stack: Vec<(usize, f32, f32, usize)> = vec![(c.root, 0.0, TAU, 0)];
    let mut labels = 0;
    while let Some((n, a0, a1, depth)) = stack.pop() {
        if depth >= levels {
            continue;
        }
        let mut a = a0;
        for k in c.kids(n) {
            let span = (c.size)(k) as f32 / (c.size)(n).max(1) as f32 * (a1 - a0);
            let (s, e) = (a, a + span);
            a = e;
            let r0 = core + depth as f32 * ring;
            let r1 = r0 + ring;
            // too thin to see: the rest of this folder is smaller still
            if span * r1 < 1.2 {
                break;
            }
            let (ss, ee) = (s * sweep, e * sweep);
            let gap = (0.6 / r1).min(span * 0.2);
            let base = c.col(k);
            let shade = 0.95 - depth as f32 * 0.07;
            let col = scale(base, shade * c.lit(k));
            o.wedge(centre, r0 + 0.8, r1 - 0.8, ss + gap, ee - gap, rgba(col, 0.97));
            // a soft lighter rim on the outside edge
            o.wedge(centre, r1 - 2.4, r1 - 0.8, ss + gap, ee - gap, rgba(scale(col, 1.25), 0.9));
            o.hit(Shape::Wedge(centre, r0, r1, ss, ee), k);
            // a name where the arc is long enough
            let mid = (ss + ee) * 0.5;
            let rm = (r0 + r1) * 0.5;
            let arc = (ee - ss) * rm;
            if arc > 70.0 && ring > 18.0 && labels < 60 {
                labels += 1;
                let (lx, ly) = (centre[0] + rm * mid.sin(), centre[1] - rm * mid.cos());
                let w = arc.min(150.0);
                o.label(lx - w * 0.5, ly - 8.0, w, c.tree.nodes[k].name.clone(), depth == 0);
            }
            if c.tree.nodes[k].is_dir {
                stack.push((k, s, e, depth + 1));
            }
        }
    }
    let _ = total;
}

/// The folder tree as a flame graph: root at the bottom, children stacked above.
pub(crate) fn flame(c: &Ctx, o: &mut Out) {
    let bar = 24.0;
    let (x0, w) = (10.0, c.w - 20.0);
    let base_y = c.h - 12.0 - bar;
    let levels = ((base_y - 8.0) / bar).floor().max(1.0) as usize + 1;
    let grow = c.intro(0.8);
    let mut stack: Vec<(usize, f32, f32, usize)> = vec![(c.root, x0, w, 0)];
    while let Some((n, x, bw, depth)) = stack.pop() {
        if depth >= levels || bw < 0.8 {
            continue;
        }
        // each level rises after the one below it
        let appear = ((grow * levels as f32 * 0.6) - depth as f32 * 0.6 + 1.0).clamp(0.0, 1.0);
        if appear <= 0.0 {
            continue;
        }
        let y = base_y - depth as f32 * bar;
        let base = c.col(n);
        // warm flicker on top of the type colour: a nod to the name
        let flick = 1.0 + 0.04 * ((c.time * 3.0 + n as f32 * 0.37).sin());
        let col = scale(base, (0.9 + 0.06 * (depth % 2) as f32) * c.lit(n) * flick);
        let h = (bar - 2.0) * appear;
        o.rect_v(x + 0.5, y + (bar - 2.0 - h), (bw - 1.0).max(0.6), h, rgba(scale(col, 1.18), 0.98), rgba(scale(col, 0.82), 0.98));
        o.hit(Shape::Rect([x, y, bw, bar]), n);
        if bw > 46.0 && appear > 0.9 {
            o.label(x + 5.0, y + 3.0, bw - 8.0, c.name(n), depth == 0);
        }
        if c.tree.nodes[n].is_dir {
            let total = (c.size)(n).max(1) as f32;
            let mut cx = x;
            for k in c.kids(n) {
                let kw = (c.size)(k) as f32 / total * bw;
                if kw < 0.8 {
                    break;
                }
                stack.push((k, cx, kw, depth + 1));
                cx += kw;
            }
        }
    }
}

/// The root as a grid of equal squares; the ones the Advisor can win back glow.
pub(crate) fn waffle(c: &Ctx, o: &mut Out, only_reclaim: bool) {
    let total = (c.size)(c.root).max(1);
    let kids = c.kids(c.root);
    // as many squares as fit at about 22 DIPs, between 100 and 600
    let legend_w = 260.0f32.min(c.w * 0.32);
    let (gw, gh) = (c.w - legend_w - 36.0, c.h - 36.0);
    let target = ((gw * gh) / (22.0 * 22.0)).clamp(100.0, 600.0);
    let cell = ((gw * gh) / target).sqrt().floor().max(8.0);
    let cols = (gw / cell).floor().max(1.0) as usize;
    let rows = (gh / cell).floor().max(1.0) as usize;
    let count = cols * rows;
    let unit = total as f64 / count as f64;
    // largest remainder: each child's share of the squares
    let mut shares: Vec<(usize, usize, f64)> = kids.iter().map(|&k| {
        let exact = (c.size)(k) as f64 / unit;
        (k, exact.floor() as usize, exact - exact.floor())
    }).collect();
    let mut used: usize = shares.iter().map(|s| s.1).sum();
    let mut order: Vec<usize> = (0..shares.len()).collect();
    order.sort_by(|&a, &b| shares[b].2.partial_cmp(&shares[a].2).unwrap_or(std::cmp::Ordering::Equal));
    for i in order {
        if used >= count {
            break;
        }
        shares[i].1 += 1;
        used += 1;
    }
    let ox = 18.0;
    let oy = (c.h - rows as f32 * cell) * 0.5;
    let pulse = 0.5 + 0.5 * (c.time * 2.4).sin();
    let mut i = 0usize;
    let fill = c.intro(0.9);
    for (k, n, _) in &shares {
        let lit = ((c.reclaim_of(*k) as f64 / unit).round() as usize).min(*n);
        for j in 0..*n {
            if i >= count {
                break;
            }
            let (cx, cy) = (ox + (i % cols) as f32 * cell, oy + (i / cols) as f32 * cell);
            // squares fill in row by row as the view opens
            if (i as f32) < fill * count as f32 {
                let win = j < lit;
                let base = c.col(*k);
                let (col, a) = if only_reclaim && !win {
                    (scale(base, 0.35), 0.35)
                } else if win {
                    (mix(scale(base, c.lit(*k)), [0.55, 1.0, 0.7], 0.25 + 0.25 * pulse), 1.0)
                } else {
                    (scale(base, 0.88 * c.lit(*k)), 0.95)
                };
                let pad = (cell * 0.09).max(1.0);
                o.rect(cx + pad, cy + pad, cell - pad * 2.0, cell - pad * 2.0, rgba(col, a));
                if win {
                    let g = 0.35 + 0.35 * pulse;
                    o.rect(cx + pad - 1.0, cy + pad - 1.0, cell - pad * 2.0 + 2.0, 1.0, [0.6, 1.0, 0.75, g]);
                }
                o.hit(Shape::Rect([cx, cy, cell, cell]), *k);
            }
            i += 1;
        }
    }
    // the key: one row per top folder, then what a square is worth
    let lx = c.w - legend_w - 6.0;
    let mut ly = oy;
    o.label(lx, ly, legend_w, format!("One square = {}", (c.fmt)(unit.round() as u64)), true);
    ly += 22.0;
    let win: u64 = kids.iter().map(|&k| c.reclaim_of(k)).sum();
    if win > 0 {
        o.label(lx, ly, legend_w, format!("{} squares can be won back ({})", (win as f64 / unit).round() as u64, (c.fmt)(win)), false);
        ly += 22.0;
    }
    for &(k, n, _) in shares.iter().take(((c.h - ly - 10.0) / 20.0).max(0.0) as usize) {
        if n == 0 {
            continue;
        }
        o.rect(lx, ly + 4.0, 10.0, 10.0, rgba(c.col(k), 1.0));
        o.label(lx + 16.0, ly, legend_w - 16.0, format!("{}  ·  {n}", c.tree.nodes[k].name), false);
        o.hit(Shape::Rect([lx, ly, legend_w, 20.0]), k);
        ly += 20.0;
    }
}

/// The biggest files as books on shelves, grouped by top folder, widest first in each.
pub(crate) fn library(c: &Ctx, o: &mut Out) {
    let shelf_h = 118.0f32.min((c.h - 24.0) / 2.0).max(40.0);
    let shelves = ((c.h - 16.0) / (shelf_h + 16.0)).floor().max(1.0) as usize;
    let (left, right) = (16.0, c.w - 16.0);
    let room = (right - left) * shelves as f32 * 0.92;
    // as many books as fit at a readable width
    let most = ((room / 11.0) as usize).clamp(12, 260);
    let files: Vec<usize> = super::biggest_files(c, 2000).iter().copied().take(most).collect();
    if files.is_empty() {
        return;
    }
    // shelved by the top folder they are in: biggest group first, widest book first inside
    let top = |mut f: usize| {
        while let Some(p) = c.tree.nodes[f].parent {
            if p == c.root {
                break;
            }
            f = p;
        }
        f
    };
    let mut groups: Vec<(usize, Vec<usize>)> = Vec::new();
    for &f in &files {
        let g = top(f);
        match groups.iter_mut().find(|(q, _)| *q == g) {
            Some((_, v)) => v.push(f),
            None => groups.push((g, vec![f])),
        }
    }
    let gap = 16.0;
    let sum: f32 = files.iter().map(|&f| (c.size)(f) as f32).sum();
    let width = |f: usize, k: f32| ((c.size)(f) as f32 * k + 6.0).min(84.0);
    let mut k = room / sum.max(1.0);
    for _ in 0..40 {
        let need: f32 = files.iter().map(|&f| width(f, k)).sum::<f32>() + groups.len() as f32 * gap;
        if need <= room {
            break;
        }
        k *= 0.88;
    }
    let shelf_y = |s: usize| 10.0 + s as f32 * (shelf_h + 16.0);
    let (mut shelf, mut x) = (0usize, left);
    let grow = c.intro(0.8);
    let mut placed = 0usize;
    'groups: for (g, books) in &groups {
        let mut run_start = x;
        let mut run_shelf = shelf;
        let flush = |o: &mut Out, from: f32, to: f32, s: usize| {
            let name = &c.tree.nodes[*g].name;
            if to - from >= 44.0 {
                o.label(from, shelf_y(s) + shelf_h + 2.0, to - from, name.clone(), false);
            }
        };
        for &f in books {
            let bw = width(f, k);
            if x + bw > right {
                flush(o, run_start, x, run_shelf);
                shelf += 1;
                x = left;
                run_start = x;
                run_shelf = shelf;
                if shelf >= shelves {
                    break 'groups;
                }
            }
            let sy = shelf_y(shelf);
            let seed = hash01(f as u64);
            // books slide in from the left, one after another
            let shown = (grow * files.len() as f32 * 1.2 - placed as f32).clamp(0.0, 1.0);
            placed += 1;
            let bh = shelf_h * (0.7 + 0.28 * seed) * shown;
            let pull = if c.hover == Some(f) { 14.0 } else { 0.0 };
            let y = sy + shelf_h - bh - pull;
            let col = scale(c.col(f), (0.78 + 0.3 * hash01(f as u64 + 7)) * c.lit(f));
            o.rect_v(x + 0.5, y, bw - 1.5, bh, rgba(scale(col, 1.15), 1.0), rgba(scale(col, 0.72), 1.0));
            if bw > 9.0 {
                o.rect(x + 2.0, y + bh * 0.1, bw - 5.0, 2.0, rgba(scale(col, 1.45), 0.55));
                o.rect(x + 2.0, y + bh * 0.84, bw - 5.0, 2.0, rgba(scale(col, 1.45), 0.55));
            }
            o.hit(Shape::Rect([x, y, bw, bh + pull]), f);
            x += bw;
        }
        flush(o, run_start, x, run_shelf);
        x += gap;
    }
    for s in 0..shelves {
        let y = shelf_y(s) + shelf_h;
        o.rect(left - 6.0, y, right - left + 12.0, 4.0, [0.42, 0.33, 0.24, 0.9]);
        o.rect(left - 6.0, y + 4.0, right - left + 12.0, 2.0, [0.0, 0.0, 0.0, 0.35]);
    }
}
