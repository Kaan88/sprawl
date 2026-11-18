//! The glass overlay shown over the treemap while a scan runs: a block map of how far the read
//! has got, live counters, a throughput sparkline and the biggest file types racing each other.

use std::collections::VecDeque;

use windows_core::{Interface, HSTRING};

use crate::bindings::Microsoft::UI::Xaml::Controls::{Border, Image, TextBlock};
use crate::bindings::Microsoft::UI::Xaml::Media::{Brush, ImageSource, SolidColorBrush};
use crate::bindings::Microsoft::UI::Xaml::{FrameworkElement, Thickness, UIElement, Visibility};
use crate::bindings::Windows::UI::Color;
use crate::format::thousands;
use crate::state::{find, State};
use crate::{app, paint};

/// What the scanner reports while it runs.
#[derive(Default, Clone)]
pub(crate) struct Live {
    pub(crate) files: u64,
    pub(crate) folders: u64,
    pub(crate) bytes: u64,
    /// 0..1 when the total is known (MFT read), else None.
    pub(crate) fraction: Option<f64>,
    /// Sizes so far of the scanned folder's direct children.
    pub(crate) top: Vec<(String, u64)>,
    /// (extension, bytes, files), biggest first.
    pub(crate) by_ext: Vec<(String, u64, u64)>,
    pub(crate) method: &'static str,
    pub(crate) phase: &'static str,
}

const CELLS_X: usize = 60;
const CELLS_Y: usize = 7;
const BMP_W: usize = 604;
const BMP_H: usize = 150;
pub(crate) const LANES: usize = 6;

struct Lane {
    ext: Option<String>,
    y: f32,
    vel: f32,
    width: f32,
}

pub(crate) struct Hud {
    opacity: f32,
    cells: Vec<Option<(u8, u8, u8)>>,
    rates: VecDeque<f32>,
    last_items: u64,
    since_sample: f32,
    since_bitmap: f32,
    time: f32,
    lanes: Vec<Lane>,
    /// Used space of the volume, to estimate progress when the scanner can't tell.
    pub(crate) expected_bytes: u64,
    elements: Option<Elements>,
}

struct Elements {
    hud: UIElement,
    image: Image,
    pct: TextBlock,
    files: TextBlock,
    folders: TextBlock,
    bytes: TextBlock,
    rate: TextBlock,
    method: TextBlock,
    lanes: Vec<(FrameworkElement, TextBlock, Border, TextBlock)>,
}

impl Default for Hud {
    fn default() -> Self {
        Hud {
            opacity: 0.0,
            cells: vec![None; CELLS_X * CELLS_Y],
            rates: VecDeque::new(),
            last_items: 0,
            since_sample: 0.0,
            since_bitmap: 1.0,
            time: 0.0,
            lanes: (0..LANES).map(|_| Lane { ext: None, y: (LANES * 22) as f32, vel: 0.0, width: 0.0 }).collect(),
            expected_bytes: 0,
            elements: None,
        }
    }
}

impl Hud {
    pub(crate) fn reset(&mut self, expected_bytes: u64) {
        let els = self.elements.take();
        *self = Hud { expected_bytes, elements: els, ..Default::default() };
    }
}

pub(crate) fn brush(c: (u8, u8, u8)) -> Option<Brush> {
    let b = SolidColorBrush::new().ok()?;
    b.SetColor(Color { A: 255, R: c.0, G: c.1, B: c.2 }).ok()?;
    b.cast().ok()
}

fn fill(px: &mut [u8], x0: i32, y0: i32, x1: i32, y1: i32, c: (u8, u8, u8), a: f32) {
    for y in y0.max(0)..y1.min(BMP_H as i32) {
        for x in x0.max(0)..x1.min(BMP_W as i32) {
            let i = (y as usize * BMP_W + x as usize) * 4;
            // premultiplied BGRA, blended over what's there
            let inv = 1.0 - a;
            px[i] = (c.2 as f32 * a + px[i] as f32 * inv) as u8;
            px[i + 1] = (c.1 as f32 * a + px[i + 1] as f32 * inv) as u8;
            px[i + 2] = (c.0 as f32 * a + px[i + 2] as f32 * inv) as u8;
            px[i + 3] = (255.0 * a + px[i + 3] as f32 * inv) as u8;
        }
    }
}

impl State {
    fn hud_elements(&mut self) -> bool {
        if self.hud.elements.is_some() {
            return true;
        }
        let r = &self.root;
        let els = (|| -> windows_core::Result<Elements> {
            let mut lanes = Vec::new();
            for k in 0..LANES {
                lanes.push((
                    find::<FrameworkElement>(r, &format!("Lane{k}"))?,
                    find::<TextBlock>(r, &format!("Lane{k}Name"))?,
                    find::<Border>(r, &format!("Lane{k}Bar"))?,
                    find::<TextBlock>(r, &format!("Lane{k}Val"))?,
                ));
            }
            Ok(Elements {
                hud: find(r, "Hud")?,
                image: find(r, "HudImage")?,
                pct: find(r, "HudPct")?,
                files: find(r, "HudFiles")?,
                folders: find(r, "HudFolders")?,
                bytes: find(r, "HudBytes")?,
                rate: find(r, "HudRate")?,
                method: find(r, "HudMethod")?,
                lanes,
            })
        })();
        match els {
            Ok(e) => {
                self.hud.elements = Some(e);
                true
            }
            Err(e) => {
                eprintln!("hud: {e:?}");
                false
            }
        }
    }

    pub(crate) fn hud_frame(&mut self, dt: f32) {
        if !self.hud_elements() {
            return;
        }
        let live = self.job.as_ref().map(|j| j.live());
        let target = if live.is_some() { 1.0 } else { 0.0 };
        let h = &mut self.hud;
        h.time += dt;
        let before = h.opacity;
        h.opacity += (target - h.opacity) * (dt * 6.0).min(1.0);
        if (h.opacity - target).abs() < 0.01 {
            h.opacity = target;
        }
        let els = h.elements.as_ref().unwrap();
        if h.opacity != before || h.opacity > 0.0 {
            let _ = els.hud.SetOpacity(h.opacity as f64);
            let _ = els.hud.SetVisibility(if h.opacity > 0.0 { Visibility::Visible } else { Visibility::Collapsed });
        }
        let Some(live) = live else { return };

        // progress estimate
        let fraction = live.fraction.unwrap_or_else(|| {
            if h.expected_bytes > 0 {
                (live.bytes as f64 / h.expected_bytes as f64).min(0.99)
            } else {
                1.0 - (-(live.files as f64) / 250_000.0).exp()
            }
        });
        let items = live.files + live.folders;
        h.since_sample += dt;
        if h.since_sample >= 0.1 {
            let rate = (items.saturating_sub(h.last_items)) as f32 / h.since_sample;
            h.rates.push_back(rate);
            if h.rates.len() > 90 {
                h.rates.pop_front();
            }
            h.last_items = items;
            h.since_sample = 0.0;
        }
        let set = |t: &TextBlock, s: String| {
            let _ = t.SetText(&HSTRING::from(s));
        };
        set(&els.pct, format!("{:.0}%", fraction * 100.0));
        set(&els.files, thousands(live.files));
        set(&els.folders, thousands(live.folders));
        set(&els.bytes, crate::format::size(live.bytes));
        set(&els.rate, thousands(h.rates.back().copied().unwrap_or(0.0) as u64));
        set(&els.method, if live.phase.is_empty() { live.method.to_uppercase() } else { format!("{}  ·  {}", live.method.to_uppercase(), live.phase.to_uppercase()) });

        // colour of the cells read since the last frame: the biggest file type right now
        let lead = live.by_ext.first().map(|_| paint::colour(0)).unwrap_or((76, 194, 255));
        let done = ((fraction * (CELLS_X * CELLS_Y) as f64) as usize).min(CELLS_X * CELLS_Y);
        let rank_colour = |k: usize| paint::colour(k);
        for i in 0..done {
            if h.cells[i].is_none() {
                // pick a type weighted by size so the map's colours mirror what's being found
                let total: u64 = live.by_ext.iter().take(12).map(|e| e.1).sum::<u64>().max(1);
                let mut t = ((i as u64).wrapping_mul(2654435761) % 1000) * total / 1000;
                let mut col = lead;
                for (k, e) in live.by_ext.iter().take(12).enumerate() {
                    if t < e.1 {
                        col = rank_colour(k);
                        break;
                    }
                    t -= e.1;
                }
                h.cells[i] = Some(col);
            }
        }

        h.since_bitmap += dt;
        if h.since_bitmap >= 1.0 / 30.0 {
            h.since_bitmap = 0.0;
            let mut px = vec![0u8; BMP_W * BMP_H * 4];
            let map_h = 84.0;
            let (cw, ch) = (BMP_W as f32 / CELLS_X as f32, map_h / CELLS_Y as f32);
            for i in 0..CELLS_X * CELLS_Y {
                let (x, y) = ((i % CELLS_X) as f32 * cw, (i / CELLS_X) as f32 * ch);
                let (c, a) = match h.cells[i] {
                    Some(c) => (c, 0.9),
                    None if i == done => ((255, 255, 255), 0.45 + 0.45 * (h.time * 14.0).sin().abs()),
                    None => ((255, 255, 255), 0.07),
                };
                fill(&mut px, (x + 1.0) as i32, (y + 1.0) as i32, (x + cw - 1.0) as i32, (y + ch - 1.0) as i32, c, a);
            }
            // sparkline
            let top = map_h + 14.0;
            let sh = BMP_H as f32 - top - 2.0;
            let max = h.rates.iter().copied().fold(1.0f32, f32::max) * 1.15;
            let n = h.rates.len();
            if n > 1 {
                let pts: Vec<(f32, f32)> =
                    h.rates.iter().enumerate().map(|(i, &v)| (i as f32 / 89.0 * BMP_W as f32, top + sh - v / max * sh)).collect();
                for x in 0..BMP_W {
                    let fx = x as f32 / BMP_W as f32 * 89.0;
                    let k = fx.floor() as usize;
                    if k + 1 >= n {
                        break;
                    }
                    let t = fx - k as f32;
                    let y = pts[k].1 + (pts[k + 1].1 - pts[k].1) * t;
                    for yy in (y as i32)..(top + sh) as i32 {
                        let a = 0.32 * (1.0 - (yy as f32 - y) / sh).max(0.0);
                        fill(&mut px, x as i32, yy, x as i32 + 1, yy + 1, (76, 194, 255), a);
                    }
                    fill(&mut px, x as i32, y as i32 - 1, x as i32 + 1, y as i32 + 2, (120, 214, 255), 1.0);
                }
                let (lx, ly) = pts[n - 1];
                fill(&mut px, lx as i32 - 3, ly as i32 - 3, lx as i32 + 3, ly as i32 + 3, (255, 255, 255), 1.0);
            }
            if let Ok(bmp) = app::bitmap(BMP_W, BMP_H, &px) {
                if let Ok(src) = bmp.cast::<ImageSource>() {
                    let _ = els.image.SetSource(&src);
                }
            }
        }

        // file-type race
        let top: Vec<&(String, u64, u64)> = live.by_ext.iter().take(LANES).collect();
        let max = top.first().map(|e| e.1).unwrap_or(1).max(1);
        // keep lanes with their extension where possible
        for lane in h.lanes.iter_mut() {
            if lane.ext.as_ref().is_some_and(|x| !top.iter().any(|e| &e.0 == x)) {
                lane.ext = None;
            }
        }
        for e in &top {
            if !h.lanes.iter().any(|l| l.ext.as_ref() == Some(&e.0)) {
                if let Some(free) = h.lanes.iter_mut().find(|l| l.ext.is_none()) {
                    free.ext = Some(e.0.clone());
                    free.y = (LANES * 22) as f32;
                    free.width = 0.0;
                }
            }
        }
        for (k, lane) in h.lanes.iter_mut().enumerate() {
            let (fe, name, bar, val) = &els.lanes[k];
            let Some(ext) = lane.ext.clone() else {
                let _ = fe.SetOpacity(0.0);
                continue;
            };
            let rank = top.iter().position(|e| e.0 == ext).unwrap_or(LANES);
            let e = top[rank.min(top.len() - 1)];
            let ty = rank as f32 * 22.0;
            lane.vel += ((ty - lane.y) * 160.0 - lane.vel * 18.0) * dt;
            lane.y += lane.vel * dt;
            let tw = (e.1 as f32 / max as f32) * 170.0;
            lane.width += (tw - lane.width) * (dt * 8.0).min(1.0);
            let _ = fe.SetOpacity(1.0);
            let _ = fe.SetMargin(Thickness { Left: 0.0, Top: lane.y as f64, Right: 0.0, Bottom: 0.0 });
            let shown = if ext == "(none)" { "(none)".to_string() } else { format!(".{ext}") };
            let _ = name.SetText(&HSTRING::from(shown));
            let _ = val.SetText(&HSTRING::from(crate::format::size(e.1)));
            let _ = bar.SetWidth(lane.width.max(2.0) as f64);
            if let Some(b) = brush(rank_colour(rank)) {
                let _ = bar.SetBackground(&b);
            }
        }
    }
}
