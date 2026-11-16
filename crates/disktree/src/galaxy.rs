//! The galaxy view: the biggest top-level folders become spiral arms, every folder below them
//! a star (brightness and size from its size), the biggest files planets circling their
//! folder's star, and small files a haze of dust. The camera orbits slowly; drag to turn,
//! double-click a star to fly into that folder.

use std::collections::HashMap;
use std::f32::consts::TAU;

use disktree_core::Tree;

use crate::gpu::Instance;
use crate::scene::{cross, look_at, mul, norm, perspective, xform};

/// Radius of the galaxy disc in world units.
const R: f32 = 500.0;
/// Stars (folders) drawn at most; the smallest go first.
const MAX_STARS: usize = 3500;
const MAX_PLANETS: usize = 600;
const MAX_DUST: usize = 9000;

#[derive(Clone, Copy, PartialEq)]
enum Kind {
    Star,
    Planet,
    Dust,
    Core,
}

#[derive(Clone, Copy)]
struct Body {
    node: usize,
    kind: Kind,
    pos: [f32; 3],
    vel: [f32; 3],
    target: [f32; 3],
    radius: f32,
    colour: [f32; 3],
    bright: f32,
    /// Planets: index of the star they circle, orbit radius, speed and phase.
    orbit: (usize, f32, f32, f32),
    glow: f32,
    pulse: f32,
}

pub struct Galaxy {
    bodies: Vec<Body>,
    /// node -> body, for stars and planets
    index: HashMap<usize, usize>,
    pub instances: Vec<Instance>,
    pub vp: [f32; 16],
    pub eye: [f32; 3],
    pub right: [f32; 3],
    pub up: [f32; 3],
    pub w: f32,
    pub h: f32,
    pub animate: bool,
    pub hover: Option<usize>,
    pub selected: Option<usize>,
    spin: f32,
    drag_yaw: f32,
    drag_tilt: f32,
    time: f32,
    /// Wheel zoom: current and target magnification (1 = whole galaxy), and the point on the
    /// disc the camera looks at (current and target).
    zoom: f32,
    zoom_t: f32,
    focus: [f32; 2],
    focus_t: [f32; 2],
}

impl Default for Galaxy {
    fn default() -> Self {
        Galaxy {
            bodies: Vec::new(),
            index: HashMap::new(),
            instances: Vec::new(),
            vp: [0.0; 16],
            eye: [0.0, 0.0, 1.0],
            right: [1.0, 0.0, 0.0],
            up: [0.0, 1.0, 0.0],
            w: 1.0,
            h: 1.0,
            animate: true,
            hover: None,
            selected: None,
            spin: 0.4,
            drag_yaw: 0.0,
            drag_tilt: 0.0,
            time: 0.0,
            zoom: 1.0,
            zoom_t: 1.0,
            focus: [0.0; 2],
            focus_t: [0.0; 2],
        }
    }
}

/// A repeatable pseudo-random number in 0..1 from a node id and a salt.
fn hash(n: usize, salt: u32) -> f32 {
    let mut x = (n as u32).wrapping_mul(0x9E37_79B1) ^ salt.wrapping_mul(0x85EB_CA77);
    x ^= x >> 15;
    x = x.wrapping_mul(0x2C1B_3C6D);
    x ^= x >> 12;
    (x >> 8) as f32 / 16_777_216.0
}

impl Galaxy {
    pub fn clear(&mut self) {
        self.bodies.clear();
        self.index.clear();
        self.instances.clear();
    }

    /// Lay out the folders under `root`. Bodies that were already there keep their place and glide to
    /// the new one; new ones come out of their parent's star (or the core).
    pub fn relayout(&mut self, t: &Tree, root: usize, weight: &dyn Fn(usize) -> u64, arm_colour: &dyn Fn(usize) -> [f32; 3]) {
        let old: HashMap<(usize, u8), [f32; 3]> = self.bodies.iter().map(|b| ((b.node, b.kind as u8), b.pos)).collect();
        let total = weight(root).max(1) as f32;
        let mut bodies = Vec::new();
        bodies.push(Body {
            node: root,
            kind: Kind::Core,
            pos: [0.0; 3],
            vel: [0.0; 3],
            target: [0.0; 3],
            radius: 30.0,
            colour: [1.0, 0.86, 0.66],
            bright: 1.0,
            orbit: (0, 0.0, 0.0, 0.0),
            glow: 0.0,
            pulse: 0.0,
        });

        // Arms: the biggest folders directly under the root; the rest join the arm with least weight.
        let mut tops: Vec<usize> = t.nodes[root].children.iter().copied().filter(|&c| t.nodes[c].is_dir && weight(c) > 0).collect();
        tops.sort_by_key(|&c| std::cmp::Reverse(weight(c)));
        let arms = tops.len().clamp(1, 6);
        let mut arm_of: Vec<Vec<usize>> = vec![Vec::new(); arms];
        let mut arm_w = vec![0u64; arms];
        for (i, &c) in tops.iter().enumerate() {
            let k = if i < arms { i } else { (0..arms).min_by_key(|&k| arm_w[k]).unwrap_or(0) };
            arm_of[k].push(c);
            arm_w[k] += weight(c);
        }

        let star_budget = MAX_STARS;
        let mut planets = 0usize;
        let mut dust = 0usize;
        let files_total: u64 = t.nodes[root].files.max(1);
        let dust_per_file = (MAX_DUST as f64 / files_total as f64).min(0.25);
        for (k, heads) in arm_of.iter().enumerate() {
            if heads.is_empty() {
                continue;
            }
            // Breadth first, biggest first, so the arm runs from big shallow folders near the core to small
            // deep ones at its tip.
            let budget = (star_budget as f32 * (arm_w[k] as f32 / total).max(0.06)).max(40.0) as usize;
            let mut order = Vec::new();
            let mut level: Vec<usize> = heads.clone();
            while !level.is_empty() && order.len() < budget {
                level.sort_by_key(|&c| std::cmp::Reverse(weight(c)));
                let mut next = Vec::new();
                for &n in &level {
                    if order.len() >= budget {
                        break;
                    }
                    order.push(n);
                    next.extend(t.nodes[n].children.iter().copied().filter(|&c| t.nodes[c].is_dir && weight(c) > 0));
                }
                level = next;
            }
            let base = k as f32 / arms as f32 * TAU;
            let col = arm_colour(k);
            let count = order.len().max(1) as f32;
            let first_star = bodies.len();
            let mut star_index: HashMap<usize, usize> = HashMap::new();
            for (i, &n) in order.iter().enumerate() {
                let s = (i as f32 + 0.5) / count;
                let r = R * (0.13 + 0.85 * s.powf(0.75));
                let spread = 0.30 * (1.0 - 0.45 * s);
                let a = base + 2.8 * s + (hash(n, 1) - 0.5) * spread * 2.0;
                let rr = r * (1.0 + (hash(n, 2) - 0.5) * 0.22);
                let z = (hash(n, 3) - 0.5) * R * 0.07 * (1.0 - s * 0.7);
                let frac = weight(n) as f32 / total;
                let white = (frac * 4.0).min(0.6);
                let c = [col[0] + (1.0 - col[0]) * white, col[1] + (1.0 - col[1]) * white, col[2] + (1.0 - col[2]) * white];
                star_index.insert(n, bodies.len());
                bodies.push(Body {
                    node: n,
                    kind: Kind::Star,
                    pos: [0.0; 3],
                    vel: [0.0; 3],
                    target: [rr * a.cos(), rr * a.sin(), z],
                    radius: 1.6 + 26.0 * frac.powf(0.42),
                    colour: c,
                    bright: 0.45 + 0.55 * frac.powf(0.25),
                    orbit: (0, 0.0, 0.0, 0.0),
                    glow: 0.0,
                    pulse: 0.0,
                });
            }
            // Planets (the biggest files of each star's folder) and dust (a sample of everything else).
            for si in first_star..bodies.len() {
                let n = bodies[si].node;
                let star_r = bodies[si].radius;
                let mut files: Vec<usize> = t.nodes[n].children.iter().copied().filter(|&c| !t.nodes[c].is_dir).collect();
                files.sort_by_key(|&c| std::cmp::Reverse(weight(c)));
                for (j, &f) in files.iter().take(3).enumerate() {
                    let frac = weight(f) as f32 / total;
                    if planets >= MAX_PLANETS || frac < 0.0015 {
                        break;
                    }
                    planets += 1;
                    bodies.push(Body {
                        node: f,
                        kind: Kind::Planet,
                        pos: [0.0; 3],
                        vel: [0.0; 3],
                        target: [0.0; 3],
                        radius: 1.2 + 9.0 * frac.powf(0.45),
                        colour: [0.75 + 0.25 * hash(f, 4), 0.8, 0.95],
                        bright: 0.9,
                        orbit: (si, star_r * 1.6 + 8.0 + j as f32 * 7.0, 0.25 + hash(f, 5) * 0.35, hash(f, 6) * TAU),
                        glow: 0.0,
                        pulse: 0.0,
                    });
                }
                let small = t.nodes[n].children.iter().filter(|&&c| !t.nodes[c].is_dir).count();
                let want = ((small as f64 * dust_per_file).round() as usize).min(MAX_DUST - dust);
                let p = bodies[si].target;
                for d in 0..want {
                    let seed = n.wrapping_mul(131).wrapping_add(d);
                    let a = hash(seed, 7) * TAU;
                    let rad = (8.0 + star_r * 2.0) * hash(seed, 8).sqrt() * 2.2;
                    dust += 1;
                    bodies.push(Body {
                        node: n,
                        kind: Kind::Dust,
                        pos: [0.0; 3],
                        vel: [0.0; 3],
                        target: [p[0] + rad * a.cos(), p[1] + rad * a.sin(), p[2] + (hash(seed, 9) - 0.5) * 10.0],
                        radius: 0.9 + hash(seed, 10) * 0.9,
                        colour: [col[0] * 0.8 + 0.2, col[1] * 0.8 + 0.2, col[2] * 0.8 + 0.2],
                        bright: 0.25 + hash(seed, 11) * 0.3,
                        orbit: (0, 0.0, 0.0, 0.0),
                        glow: 0.0,
                        pulse: 0.0,
                    });
                }
            }
            let _ = star_index;
        }

        // Start positions: where it was, else at its parent's old place, else the core.
        for b in &mut bodies {
            let key = (b.node, b.kind as u8);
            b.pos = match old.get(&key) {
                Some(&p) => p,
                None => t.nodes[b.node]
                    .parent
                    .and_then(|p| old.get(&(p, Kind::Star as u8)).copied())
                    .unwrap_or([b.target[0] * 0.05, b.target[1] * 0.05, 0.0]),
            };
        }
        self.index = bodies.iter().enumerate().filter(|(_, b)| b.kind != Kind::Dust).map(|(i, b)| (b.node, i)).collect();
        self.bodies = bodies;
    }

    /// Names next to the biggest stars, and the hovered or selected one.
    pub fn labels(&self, t: &Tree, text: &dyn Fn(usize) -> String) -> Vec<crate::gpu::Label> {
        let mut stars: Vec<&Body> = self.bodies.iter().filter(|b| matches!(b.kind, Kind::Star | Kind::Planet) && b.node < t.nodes.len()).collect();
        stars.sort_by(|a, b| b.radius.total_cmp(&a.radius));
        let pinned: Vec<&Body> = stars.iter().copied().filter(|b| Some(b.node) == self.hover || Some(b.node) == self.selected).collect();
        let screen = |p: [f32; 3]| {
            let q = xform(&self.vp, [p[0], p[1], p[2], 1.0]);
            (q[3] > 0.0).then(|| ((q[0] / q[3] * 0.5 + 0.5) * self.w, (0.5 - q[1] / q[3] * 0.5) * self.h))
        };
        let core = screen([0.0; 3]).unwrap_or((-1e4, -1e4));
        let big = stars.iter().copied().filter(|b| b.kind == Kind::Star);
        let mut out = Vec::new();
        let mut taken: Vec<(f32, f32)> = Vec::new();
        let n_pinned = pinned.len();
        for (i, b) in pinned.into_iter().chain(big).enumerate() {
            let Some((x, y)) = screen(b.pos) else { continue };
            if x < 0.0 || y < 0.0 || x > self.w || y > self.h {
                continue;
            }
            // spread the names out: none on the bright core, none crowding another
            if i >= n_pinned {
                let near = |p: (f32, f32), r: f32| (p.0 - x).hypot((p.1 - y) * 1.8) < r;
                if out.len() >= n_pinned + 12 || near(core, 70.0) || taken.iter().any(|&t| near(t, 120.0)) {
                    continue;
                }
            }
            taken.push((x, y));
            let s = text(b.node);
            let w = s.chars().count() as f32 * 6.8 + 8.0;
            let lit = Some(b.node) == self.hover || Some(b.node) == self.selected;
            out.push(crate::gpu::Label { x: x + 9.0, y: y - 9.0, max_w: w, text: s, strong: lit });
        }
        out
    }

    /// Make the star or planet for `node` (or the nearest one above it) flare up.
    pub fn pulse(&mut self, t: &Tree, node: usize) {
        let mut n = Some(node);
        while let Some(i) = n {
            if let Some(b) = self.index.get(&i).and_then(|&b| self.bodies.get_mut(b)) {
                b.pulse = 1.0;
                return;
            }
            n = t.nodes[i].parent;
        }
    }

    pub fn drag(&mut self, dx: f32, dy: f32) {
        self.drag_yaw -= dx * 0.006;
        self.drag_tilt = (self.drag_tilt - dy * 0.004).clamp(-0.9, 0.4);
    }

    /// Wheel zoom toward the point under the cursor (screen DIPs). Zooming all the way out
    /// centres the galaxy again.
    pub fn zoom(&mut self, x: f32, y: f32, delta: i32) {
        let step = 1.25f32.powf(delta as f32 / 120.0);
        let old = self.zoom_t;
        self.zoom_t = (old * step).clamp(1.0, 14.0);
        if self.zoom_t <= 1.0001 {
            self.focus_t = [0.0; 2];
            return;
        }
        if let Some(p) = self.on_disc(x, y) {
            // move the focus toward the point under the cursor, so it stays roughly in place
            let k = 1.0 - old / self.zoom_t;
            for i in 0..2 {
                self.focus_t[i] += (p[i] - self.focus_t[i]) * k;
            }
            let r = self.focus_t[0].hypot(self.focus_t[1]).max(1e-3);
            if r > R {
                self.focus_t = [self.focus_t[0] / r * R, self.focus_t[1] / r * R];
            }
        }
    }

    /// Where a screen point meets the galaxy's plane (z = 0).
    fn on_disc(&self, x: f32, y: f32) -> Option<[f32; 2]> {
        let inv = crate::scene::invert(&self.vp)?;
        let (nx, ny) = (x / self.w.max(1.0) * 2.0 - 1.0, 1.0 - y / self.h.max(1.0) * 2.0);
        let a = xform(&inv, [nx, ny, 0.0, 1.0]);
        let b = xform(&inv, [nx, ny, 1.0, 1.0]);
        let (a, b) = ([a[0] / a[3], a[1] / a[3], a[2] / a[3]], [b[0] / b[3], b[1] / b[3], b[2] / b[3]]);
        let dz = b[2] - a[2];
        if dz.abs() < 1e-6 {
            return None;
        }
        let t = -a[2] / dz;
        (t > 0.0).then(|| [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t])
    }

    fn camera(&mut self, dt: f32) -> bool {
        if self.animate {
            self.spin += dt * 0.035;
        }
        let (w, h) = (self.w.max(1.0), self.h.max(1.0));
        let p = perspective(32f32.to_radians(), w / h, 5.0, 40_000.0);
        let yaw = self.spin + self.drag_yaw;
        let tilt = (1.13 + self.drag_tilt).clamp(0.12, 1.4);
        let make = |d: f32| {
            let e = [d * tilt.sin() * yaw.sin(), -d * tilt.sin() * yaw.cos(), d * tilt.cos()];
            (mul(&p, &look_at(e, [0.0, 0.0, 0.0], [0.0, 0.0, 1.0])), e)
        };
        let (mut lo, mut hi) = (R * 0.3, R * 40.0);
        for _ in 0..22 {
            let d = (lo + hi) / 2.0;
            let (m, _) = make(d);
            let ok = (0..24).all(|k| {
                let a = k as f32 / 24.0 * TAU;
                let q = xform(&m, [R * 0.93 * a.cos(), R * 0.93 * a.sin(), 0.0, 1.0]);
                q[3] > 0.0 && (q[0] / q[3]).abs() <= 0.98 && (q[1] / q[3]).abs() <= 0.97
            });
            if ok {
                hi = d;
            } else {
                lo = d;
            }
        }
        // wheel zoom: glide toward the target magnification and focus
        let kz = if self.animate { 1.0 - (-dt * 8.0).exp() } else { 1.0 };
        self.zoom += (self.zoom_t - self.zoom) * kz;
        for i in 0..2 {
            self.focus[i] += (self.focus_t[i] - self.focus[i]) * kz;
        }
        let gliding = (self.zoom_t - self.zoom).abs() > 1e-3 || (self.focus_t[0] - self.focus[0]).abs() + (self.focus_t[1] - self.focus[1]).abs() > 0.05;
        let (_, e0) = make(hi / self.zoom);
        let c = [self.focus[0], self.focus[1], 0.0];
        let e = [e0[0] + c[0], e0[1] + c[1], e0[2]];
        let m = mul(&p, &look_at(e, c, [0.0, 0.0, 1.0]));
        self.vp = m;
        self.eye = e;
        let f = norm([-e[0], -e[1], -e[2]]);
        self.right = norm(cross(f, [0.0, 0.0, 1.0]));
        self.up = cross(self.right, f);
        gliding
    }

    /// Advance springs, orbits and the camera, and rebuild the instances. True while anything moves.
    pub fn update(&mut self, dt: f32) -> bool {
        self.time += dt;
        let gliding = self.camera(dt);
        let k = if self.animate { 1.0 - (-dt * 5.0).exp() } else { 1.0 };
        let mut moving = self.animate || gliding;
        // stars and dust glide to their targets
        for b in &mut self.bodies {
            if b.kind == Kind::Planet {
                continue;
            }
            for i in 0..3 {
                let d = b.target[i] - b.pos[i];
                b.pos[i] += d * k;
                if d.abs() > 0.05 {
                    moving = true;
                }
            }
            let _ = b.vel;
        }
        // planets circle their star
        for i in 0..self.bodies.len() {
            if self.bodies[i].kind != Kind::Planet {
                continue;
            }
            let (si, r, speed, phase) = self.bodies[i].orbit;
            let c = self.bodies[si].pos;
            let a = phase + self.time * speed;
            self.bodies[i].pos = [c[0] + r * a.cos(), c[1] + r * a.sin(), c[2] + r * 0.18 * (a * 1.3).sin()];
        }
        // hover and selection glow
        let ease = 1.0 - (-dt * 9.0).exp();
        for b in &mut self.bodies {
            let lit = (self.hover == Some(b.node) || self.selected == Some(b.node)) && b.kind != Kind::Dust;
            let target = if lit { 1.0 } else { 0.0 };
            b.glow += (target - b.glow) * ease;
            b.pulse = (b.pulse - dt / 1.8).max(0.0);
        }

        self.instances.clear();
        // backdrop: a huge billboard behind everything
        self.instances.push(Instance { rect: [0.0, 0.0, -R * 0.4, R * 9.0], parent: [0.0; 4], color: [0.0; 4], fx: [0.0, 4.0, 0.0, 0.0] });
        for b in &self.bodies {
            let kind = match b.kind {
                Kind::Star => 0.0,
                Kind::Planet => 1.0,
                Kind::Dust => 2.0,
                Kind::Core => 3.0,
            };
            let twinkle = if b.kind == Kind::Star { 0.85 + 0.15 * (self.time * (1.0 + hash(b.node, 12) * 2.0) + hash(b.node, 13) * TAU).sin() } else { 1.0 };
            self.instances.push(Instance {
                rect: [b.pos[0], b.pos[1], b.pos[2], b.radius],
                parent: [0.0; 4],
                color: [b.colour[0], b.colour[1], b.colour[2], b.bright * twinkle * (1.0 + b.pulse * 2.5)],
                fx: [b.glow.max((b.pulse * 9.0).sin().abs() * b.pulse), kind, 0.0, 0.0],
            });
        }
        moving
    }

    fn screen(&self, p: [f32; 3]) -> Option<(f32, f32, f32)> {
        let q = xform(&self.vp, [p[0], p[1], p[2], 1.0]);
        if q[3] <= 0.0 {
            return None;
        }
        Some(((q[0] / q[3] + 1.0) * 0.5 * self.w, (1.0 - q[1] / q[3]) * 0.5 * self.h, q[3]))
    }

    /// The star or planet under a point in DIPs.
    pub fn pick(&self, x: f32, y: f32) -> Option<usize> {
        let px_per_unit = |depth: f32| self.h / (2.0 * (16f32.to_radians()).tan() * depth);
        let mut best: Option<(f32, usize)> = None;
        for b in &self.bodies {
            if b.kind == Kind::Dust {
                continue;
            }
            let Some((sx, sy, depth)) = self.screen(b.pos) else { continue };
            let r = (b.radius * px_per_unit(depth) * 1.5).max(7.0);
            let d = ((sx - x).powi(2) + (sy - y).powi(2)).sqrt();
            if d <= r && best.map_or(true, |(bd, _)| d / r < bd) {
                best = Some((d / r, b.node));
            }
        }
        best.map(|b| b.1)
    }
}
