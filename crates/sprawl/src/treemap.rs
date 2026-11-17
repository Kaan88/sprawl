//! The treemap panel: keeps the GPU renderer and the scene in step with the window, lays out
//! the scanned tree (or the live picture while a scan runs) and handles the mouse over it.

use std::collections::HashMap;
use std::time::Instant;

use sprawl_core::{Node, Tree, ROOT};
use windows_core::Interface;

use crate::bindings::Microsoft::UI::Xaml::Controls::{ScrollViewer, SwapChainPanel};
use crate::bindings::Microsoft::UI::Xaml::FrameworkElement;
use crate::gpu::{Frame, Gpu};
use crate::scene::{ext_key, Info, Mode, Scene, CITY_DEPTH, FREE};
use crate::state::{find, text, State, View, ROW_H};
use crate::paint;

/// While a scan runs the map shows the root's top-level entries growing as they are found.
#[derive(Default)]
pub(crate) struct LiveMap {
    pub(crate) tree: Tree,
    pub(crate) index: HashMap<String, usize>,
}

impl LiveMap {
    pub(crate) fn new() -> Self {
        let root = Node { is_dir: true, ..Default::default() };
        LiveMap { tree: Tree { nodes: vec![root], ..Default::default() }, index: HashMap::new() }
    }

    /// Update sizes of the top-level entries found so far.
    pub(crate) fn update(&mut self, top: &[(String, u64)]) {
        for (name, size) in top {
            let i = match self.index.get(name) {
                Some(&i) => i,
                None => {
                    let i = self.tree.nodes.len();
                    self.tree.nodes.push(Node { name: name.clone(), files: 1, parent: Some(ROOT), ..Default::default() });
                    self.tree.nodes[ROOT].children.push(i);
                    self.index.insert(name.clone(), i);
                    i
                }
            };
            self.tree.nodes[i].size = *size;
        }
        let nodes = &self.tree.nodes;
        let mut kids = nodes[ROOT].children.clone();
        kids.sort_by(|a, b| nodes[*b].size.cmp(&nodes[*a].size));
        self.tree.nodes[ROOT].children = kids;
    }
}

pub(crate) struct MapView {
    pub(crate) panel: SwapChainPanel,
    pub(crate) gpu: Option<Gpu>,
    pub(crate) gpu_error: Option<String>,
    pub(crate) scene: Scene,
    pub(crate) dirty: bool,
    pub(crate) dims: (f32, f32, f32, f32),
    pub(crate) last: Instant,
    pub(crate) drawn_once: bool,
    pub(crate) live: Option<LiveMap>,
    pub(crate) by_alloc: bool,
    pub(crate) show_labels: bool,
    pub(crate) show_free: bool,
    /// Free bytes of the scanned drive, when a whole drive was scanned.
    pub(crate) free: Option<u64>,
    pub(crate) cushion_strength: f32,
    pub(crate) mirror: bool,
    /// How far the pointer moved with the middle or right button down since it was pressed
    /// (a right click that barely moved opens the menu).
    pub(crate) drag_dist: f64,
    /// Antialiasing samples per pixel for the 3D views (1 = off).
    pub(crate) aa: u32,
    pub(crate) last_pointer: (f64, f64),
    pub(crate) galaxy: crate::galaxy::Galaxy,
    pub(crate) capture: Option<std::path::PathBuf>,
}

impl MapView {
    pub(crate) fn new(panel: SwapChainPanel) -> Self {
        MapView {
            panel,
            gpu: None,
            gpu_error: None,
            scene: Scene::default(),
            dirty: true,
            dims: (0.0, 0.0, 1.0, 1.0),
            last: Instant::now(),
            drawn_once: false,
            live: None,
            by_alloc: false,
            show_labels: true,
            show_free: false,
            free: None,
            cushion_strength: 1.0,
            mirror: true,
            drag_dist: 0.0,
            aa: 4,
            last_pointer: (0.0, 0.0),
            galaxy: crate::galaxy::Galaxy::default(),
            capture: None,
        }
    }
}

fn palette_colour(i: usize) -> [f32; 3] {
    let (r, g, b) = paint::colour(i);
    [r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0]
}

impl State {
    /// Ask for a new layout on the next frame (tree, root, units or options changed).
    pub(crate) fn render_map(&mut self) {
        self.map.dirty = true;
        if let Some(t) = &self.tree {
            text(&self.root, "MapPath", &format!("{}", t.path_of(self.map_root).display()));
        }
    }

    /// The galaxy replaces the treemap once a scan has finished.
    pub(crate) fn galaxy_on(&self) -> bool {
        self.map.scene.mode == Mode::Galaxy && self.map.live.is_none() && self.tree.is_some()
    }

    fn relayout(&mut self) {
        // History panel: sizes as they were then, colours by what changed since the snapshot before.
        let (hist_display, hist_base) = self.hist_view().unwrap_or((None, None));
        if self.galaxy_on() {
            let m = &mut self.map;
            let t = self.tree.as_ref().unwrap();
            let alloc = &self.alloc;
            let by_alloc = m.by_alloc && alloc.len() == t.nodes.len();
            m.scene.clear();
            let hd = hist_display.as_deref();
            m.galaxy.relayout(t, self.map_root, &|n| hd.map_or(if by_alloc { alloc[n] } else { t.nodes[n].size }, |d| d[n]), &|k| palette_colour(k));
            m.dirty = false;
            return;
        }
        self.map.galaxy.clear();
        let m = &mut self.map;
        if let Some(live) = &m.live {
            let lt = &live.tree;
            m.scene.relayout(lt, ROOT, &|n| lt.nodes[n].size, &|n| Info { colour: palette_colour(n.saturating_sub(1)), height: 4.0, ..Default::default() }, None);
        } else if let Some(t) = &self.tree {
            let alloc = &self.alloc;
            let by_alloc = m.by_alloc && alloc.len() == t.nodes.len();
            let colours = &self.colours;
            let free = if m.show_free && self.map_root == ROOT { m.free } else { None };
            let city = m.scene.mode == Mode::City;
            // City heights: item counts on a log scale, the busiest building about a seventh of the
            // city's width, so the skyline reads as streets and blocks instead of a wall of towers.
            let hmax = (m.scene.w * m.scene.h).sqrt() * 0.18;
            let span = (t.nodes[self.map_root].items().max(1) as f32 * 0.25).max(10.0).log10();
            let now = std::time::SystemTime::now();
            let hd = hist_display.as_deref();
            let hb = hist_base.as_deref();
            let size_now = |n: usize| hd.map_or(t.nodes[n].size, |d| d[n]);
            let free = if hd.is_some() { None } else { free };
            m.scene.relayout(t, self.map_root, &|n| if n == FREE { 0 } else { hd.map_or(if by_alloc { alloc[n] } else { t.nodes[n].size }, |d| d[n]) }, &|n| {
                if n == FREE {
                    return Info { colour: [0.20, 0.23, 0.29], ext_key: u32::MAX, height: 1.0, ..Default::default() };
                }
                if city {
                    let node = &t.nodes[n];
                    let items = if node.is_dir { node.items() } else { 1 };
                    let height = 3.0 + hmax * ((1.0 + items as f32).log10() / span).min(1.0).powf(1.3);
                    let age = node.modified.and_then(|m| now.duration_since(m).ok()).map(|d| d.as_secs_f32() / 86_400.0).unwrap_or(365.0);
                    let accent = if age < 14.0 {
                        [0.30, 0.82, 1.0]
                    } else if age < 60.0 {
                        [0.55, 0.5, 1.0]
                    } else if age < 180.0 {
                        [0.95, 0.45, 0.8]
                    } else if age < 400.0 {
                        [1.0, 0.65, 0.3]
                    } else {
                        [0.55, 0.6, 0.75]
                    };
                    let seed = ((n as u32).wrapping_mul(2_654_435_761) >> 8) as f32 / 16_777_216.0;
                    return Info {
                        colour: [accent[0] * 0.17 + 0.035, accent[1] * 0.17 + 0.04, accent[2] * 0.17 + 0.055],
                        ext_key: u32::MAX - 1,
                        height,
                        extra: [seed, 0.82 - 0.62 * (age / 500.0).min(1.0), (age / 420.0).min(1.0), if age < 60.0 { 1.0 } else { 0.35 }],
                    };
                }
                if let Some(b) = hb {
                    return Info { colour: crate::history::change_colour(b[n], size_now(n)), ext_key: ext_key(&t.nodes[n].extension()), ..Default::default() };
                }
                let ext = t.nodes[n].extension();
                let colour = colours
                    .get(&ext)
                    .map(|&i| palette_colour(i))
                    .unwrap_or([paint::GREY.0 as f32 / 255.0, paint::GREY.1 as f32 / 255.0, paint::GREY.2 as f32 / 255.0]);
                Info { colour, ext_key: ext_key(&ext), ..Default::default() }
            }, free);
        } else {
            m.scene.clear();
        }
        m.dirty = false;
    }

    /// Called once per composition frame.
    pub(crate) fn frame(&mut self) {
        let now = Instant::now();
        let dt = (now - self.map.last).as_secs_f32().min(0.05);
        self.map.last = now;

        self.poll();
        self.welcome_frame();
        self.hud_frame(dt);
        if self.filter_pending.is_some_and(|t| t.elapsed().as_millis() > 350) {
            self.apply_filter();
        }

        // Track the panel's size and DPI.
        let p = &self.map.panel;
        let fe: FrameworkElement = match p.cast() {
            Ok(f) => f,
            Err(_) => return,
        };
        let w = fe.ActualWidth().unwrap_or(0.0) as f32;
        let h = fe.ActualHeight().unwrap_or(0.0) as f32;
        let sx = p.CompositionScaleX().unwrap_or(1.0);
        let sy = p.CompositionScaleY().unwrap_or(1.0);
        if w < 8.0 || h < 8.0 {
            self.shot_tick();
            return;
        }
        if (w, h, sx, sy) != self.map.dims {
            self.map.dims = (w, h, sx, sy);
            self.map.scene.w = w;
            self.map.scene.h = h;
            self.map.dirty = true;
            match &mut self.map.gpu {
                Some(g) => {
                    if let Err(e) = g.resize(w, h, (sx, sy)) {
                        eprintln!("resize: {e}");
                    }
                }
                None if self.map.gpu_error.is_none() => {
                    let made = p.cast::<windows_core::IUnknown>().map_err(|e| e.to_string()).and_then(|u| Gpu::new(&u, w, h, (sx, sy)));
                    match made {
                        Ok(mut g) => {
                            if let Err(e) = g.set_samples(self.map.aa) {
                                eprintln!("antialiasing: {e}");
                            }
                            self.map.gpu = Some(g)
                        }
                        Err(e) => {
                            eprintln!("GPU treemap unavailable: {e}");
                            text(&self.root, "FootLeft", &format!("The 3D treemap could not start: {e}"));
                            self.map.gpu_error = Some(e);
                        }
                    }
                }
                None => {}
            }
        }
        if self.map.dirty {
            self.relayout();
            self.map.drawn_once = false;
        }
        self.map.scene.selected = self.selected;
        self.map.scene.highlight_ext = self.ext_selected.as_deref().map(ext_key);
        // the plan preview: only the plan lit, its tiles shrinking away
        let preview = if self.view == crate::state::View::Advisor && self.map.live.is_none() { self.plan_preview() } else { None };
        self.map.scene.shrink = preview.clone();
        self.map.scene.lens = if let Some(p) = preview { Some((p, true)) } else { match self.advisor.lens_mode {
            crate::advisor::Lens::Off => None,
            // not on the live picture of a scan: its node numbers are its own
            mode if self.view == crate::state::View::Advisor && self.map.live.is_none() => self.advice().map(|r| (r.lens.clone(), mode == crate::advisor::Lens::Risk)),
            _ => None,
        } };
        // A selected folder lights up its whole block on the map.
        if self.map.live.is_none() {
            if let Some(t) = &self.tree {
                let folder = self.selected.filter(|&n| n != self.map_root && t.nodes.get(n).is_some_and(|n| n.is_dir));
                self.map.scene.set_hover_folder(t, folder);
            }
        }
        self.drain_changes();
        self.hist_tick();
        let galaxy = self.galaxy_on();
        let busy = if galaxy {
            let m = &mut self.map;
            m.galaxy.w = m.scene.w;
            m.galaxy.h = m.scene.h;
            m.galaxy.animate = m.scene.animate;
            m.galaxy.selected = self.selected;
            m.galaxy.update(dt)
        } else {
            self.map.scene.update(dt)
        };
        let want_capture = self.map.capture.is_some();
        if busy || !self.map.drawn_once || want_capture {
            if let Some(g) = &self.map.gpu {
                if g.ready() || want_capture {
                    let labels = if self.map.show_labels { self.map_labels(galaxy) } else { Vec::new() };
                    let m = &mut self.map;
                    let (vp, eye) = if galaxy { (m.galaxy.vp, m.galaxy.eye) } else { (m.scene.vp, m.scene.eye) };
                    let frame = Frame {
                        vp,
                        eye,
                        light: [-0.45, 0.55, 0.75],
                        size: [m.scene.w, m.scene.h],
                        cushion: m.scene.cushion,
                        time: { static T0: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new(); T0.get_or_init(std::time::Instant::now).elapsed().as_secs_f32() },
                        mirror: m.mirror && m.scene.tilt_on && !galaxy,
                        floor: (m.mirror && m.scene.tilt_on && !galaxy).then(|| {
                            let (w, h) = (m.scene.w, m.scene.h);
                            let rect = if m.scene.mode == Mode::Map {
                                [-w * 0.6, -h * 0.3, w * 2.2, h * 1.9]
                            } else {
                                let l = w.max(h) * 1.2;
                                [w / 2.0 - l, h / 2.0 - l, l * 2.0, l * 2.0]
                            };
                            crate::gpu::Instance {
                                rect,
                                parent: if m.scene.mode == Mode::Map { [w / 2.0, h / 2.0, 0.0, 0.0] } else { [w / 2.0, h / 2.0, 0.0, 0.0] },
                                color: [0.0, 0.0, 0.0, 2.0],
                                fx: [0.0; 4],
                            }
                        }),
                        instances: if galaxy { &[] } else { &m.scene.instances },
                        stars: if galaxy { &m.galaxy.instances } else { &[] },
                        right: m.galaxy.right,
                        up: m.galaxy.up,
                        labels: &labels,
                    };
                    let g = m.gpu.as_mut().unwrap();
                    g.want_capture = want_capture;
                    if let Err(e) = g.render(&frame) {
                        eprintln!("render: {e}");
                    }
                    m.drawn_once = true;
                    if let Some(path) = m.capture.take() {
                        let msg = match g.captured.take() {
                            Some((cw, ch, mut px)) => {
                                for q in px.chunks_exact_mut(4) {
                                    q.swap(0, 2);
                                    q[3] = 255;
                                }
                                match std::fs::write(&path, paint::png(cw, ch, &px)) {
                                    Ok(()) => format!("Saved the treemap picture to {}", path.display()),
                                    Err(e) => format!("Couldn't save the picture: {e}"),
                                }
                            }
                            None => "Couldn't read the treemap picture back from the GPU".to_string(),
                        };
                        text(&self.root, "FootLeft", &msg);
                    }
                }
            }
        }
        self.shot_tick();
    }

    fn map_labels(&self, galaxy: bool) -> Vec<crate::gpu::Label> {
        let m = &self.map;
        if let Some(live) = &m.live {
            if m.scene.mode != Mode::Map {
                return Vec::new();
            }
            return m.scene.labels(&live.tree, &|_| String::new()).into_iter().filter(|l| !l.text.is_empty()).collect();
        }
        let Some(t) = &self.tree else { return Vec::new() };
        let by_alloc = m.by_alloc && self.alloc.len() == t.nodes.len();
        let text = |n: usize| {
            let v = if by_alloc { self.alloc[n] } else { t.nodes[n].size };
            let slash = if t.nodes[n].is_dir { "\\" } else { "" };
            format!("{}{slash} ({})", t.nodes[n].name, self.size(v))
        };
        let ls = if galaxy {
            m.galaxy.labels(t, &text)
        } else if m.scene.mode == Mode::City {
            m.scene.city_labels(t, &text)
        } else {
            return m.scene.labels(t, &text);
        };
        // keep the first of any labels that would overlap
        let mut kept: Vec<crate::gpu::Label> = Vec::new();
        for l in ls {
            if !kept.iter().any(|k| l.x < k.x + k.max_w && k.x < l.x + l.max_w && (l.y - k.y).abs() < 16.0) {
                kept.push(l);
            }
        }
        kept
    }

    // ------------------------------------------------------------ pointer

    /// The pointer moved over the map. `rotate`: left button held (turns the city and galaxy);
    /// `pan`: middle or right button held (drags the view).
    pub(crate) fn map_hover(&mut self, x: f64, y: f64, rotate: bool, pan: bool) {
        if self.shot.is_some() && std::env::var_os("SPRAWL_SWITCH").is_none() {
            return;
        }
        let (px, py) = self.map.last_pointer;
        self.map.last_pointer = (x, y);
        let (dx, dy) = ((x - px) as f32, (y - py) as f32);
        if pan {
            self.map.drag_dist += (dx * dx + dy * dy).sqrt() as f64;
        }
        if rotate && self.map.scene.mode == Mode::City {
            self.map.scene.drag(dx, dy);
        }
        if pan && !self.galaxy_on() {
            self.map.scene.pan(x as f32, y as f32, dx, dy);
        }
        if self.galaxy_on() {
            if rotate {
                self.map.galaxy.drag(dx, dy);
            }
            if pan {
                self.map.galaxy.pan(x as f32, y as f32, dx, dy);
            }
            let n = self.map.galaxy.pick(x as f32, y as f32);
            if n == self.map.galaxy.hover {
                return;
            }
            self.map.galaxy.hover = n;
            if let Some(t) = &self.tree {
                let line = match n {
                    Some(n) => {
                        let kind = if t.nodes[n].is_dir { "" } else { "   planet" };
                        format!("{}   ({}){kind}", t.path_of(n).display(), self.size(t.nodes[n].size))
                    }
                    None => format!("{}", t.path_of(self.map_root).display()),
                };
                text(&self.root, "MapPath", &line);
            }
            return;
        }
        let before = self.map.scene.hover_leaf;
        let n = self.map.scene.pointer(x as f32, y as f32);
        if n == before {
            return;
        }
        if self.map.live.is_some() {
            return;
        }
        if n == Some(FREE) {
            let free = self.map.free.unwrap_or(0);
            text(&self.root, "MapPath", &format!("Free space   ({})", self.size(free)));
            return;
        }
        if let (Some(t), Some(n)) = (&self.tree, n.filter(|&n| n != FREE)) {
            let v = t.nodes[n].size;
            text(&self.root, "MapPath", &format!("{}   ({})", t.path_of(n).display(), self.size(v)));
        }
    }

    /// Switch between the treemap, the city and the galaxy.
    pub(crate) fn set_mode(&mut self, mode: Mode) {
        if self.map.scene.mode == mode {
            return;
        }
        self.map.scene.mode = mode;
        self.map.scene.reset_view();
        self.map.galaxy.reset_view();
        // Tiles change shape (folders become buildings), so start the layout afresh.
        self.map.scene.clear();
        self.render_map();
        for (name, m) in [("ModeMap", Mode::Map), ("ModeCity", Mode::City), ("ModeGalaxy", Mode::Galaxy)] {
            if let Ok(b) = crate::state::find::<crate::bindings::Microsoft::UI::Xaml::Controls::Control>(&self.root, name) {
                if let Some(br) = crate::hud::brush(if m == mode { (44, 59, 99) } else { (30, 33, 44) }) {
                    let _ = b.SetBackground(&br);
                }
            }
        }
        let _ = CITY_DEPTH;
    }

    /// A button went down on the map.
    pub(crate) fn map_press(&mut self, x: f64, y: f64) {
        self.map.last_pointer = (x, y);
        self.map.drag_dist = 0.0;
    }

    /// Right click on the map (not a drag): the file menu for what is under the pointer.
    pub(crate) fn map_menu(&mut self, x: f64, y: f64) {
        if self.map.drag_dist > 6.0 {
            return;
        }
        let Some(n) = self.map_node_at(x, y) else { return };
        self.selected = Some(n);
        self.refresh_rows();
        self.show_selection();
        let shown = || -> windows_core::Result<()> {
            use crate::bindings::Microsoft::UI::Xaml::Controls::Primitives::{FlyoutBase, FlyoutShowOptions};
            let menu: FlyoutBase = crate::state::find(&self.root, "RowMenu")?;
            let target: crate::bindings::Microsoft::UI::Xaml::DependencyObject = crate::state::find(&self.root, "MapPanel")?;
            let opts = FlyoutShowOptions::new()?;
            opts.SetPosition(Some(crate::bindings::Windows::Foundation::Point { X: x as f32, Y: y as f32 }))?;
            menu.ShowAtWithOptions(&target, &opts)
        };
        if let Err(e) = shown() {
            eprintln!("map menu: {e:?}");
        }
    }

    pub(crate) fn map_leave(&mut self) {
        self.map.scene.leave();
        self.map.galaxy.hover = None;
        if let Some(t) = &self.tree {
            text(&self.root, "MapPath", &format!("{}", t.path_of(self.map_root).display()));
        }
    }

    pub(crate) fn map_node_at(&self, x: f64, y: f64) -> Option<usize> {
        if self.map.live.is_some() {
            return None;
        }
        let len = self.tree.as_ref().map_or(0, |t| t.nodes.len());
        let n = if self.galaxy_on() { self.map.galaxy.pick(x as f32, y as f32) } else { self.map.scene.hit(x as f32, y as f32).filter(|&n| n != FREE) };
        // never hand out a node number the loaded tree doesn't have
        n.filter(|&n| n < len)
    }

    pub(crate) fn map_click(&mut self, x: f64, y: f64) {
        let Some(n) = self.map_node_at(x, y) else { return };
        self.reveal(n);
    }

    /// Double-click: zoom into the folder one level below the current treemap root.
    pub(crate) fn map_zoom_at(&mut self, x: f64, y: f64) {
        let Some(n) = self.map_node_at(x, y) else { return };
        self.map.scene.reset_view();
        self.map.galaxy.reset_view();
        let Some(t) = &self.tree else { return };
        if self.galaxy_on() {
            // fly into the star's folder (a planet's folder for a file)
            let target = if t.nodes[n].is_dir { Some(n) } else { t.nodes[n].parent };
            if let Some(f) = target.filter(|&f| f != self.map_root) {
                self.map_root = f;
                self.render_map();
            }
            return;
        }
        let mut c = n;
        while let Some(p) = t.nodes[c].parent {
            if p == self.map_root {
                break;
            }
            c = p;
        }
        if t.nodes[c].is_dir && c != self.map_root {
            self.map_root = c;
            self.render_map();
        }
    }

    /// Mouse wheel over the map: up zooms into the folder under the pointer, down zooms out.
    pub(crate) fn map_wheel(&mut self, x: f64, y: f64, delta: i32) {
        if self.galaxy_on() {
            self.map.galaxy.zoom(x as f32, y as f32, delta);
            return;
        }
        // the wheel zooms the camera; past the whole map it goes up a folder
        if self.map.scene.view_zoom(x as f32, y as f32, delta) {
            return;
        }
        if delta > 0 {
            self.map_zoom_at(x, y);
        } else if let Some(p) = self.tree.as_ref().and_then(|t| t.nodes[self.map_root].parent) {
            self.map_root = p;
            self.render_map();
        }
        self.map.scene.reset_view();
    }

    /// Select a node and open every folder above it, then scroll it into view.
    pub(crate) fn reveal(&mut self, n: usize) {
        let Some(t) = &self.tree else { return };
        let mut p = t.nodes[n].parent;
        while let Some(i) = p {
            self.expanded.insert(i);
            p = t.nodes[i].parent;
        }
        self.selected = Some(n);
        if self.view != View::Tree {
            self.set_view(View::Tree);
        }
        self.refresh_rows();
        self.show_selection();
        if let Some(k) = self.rows.iter().position(|r| r.0 == n) {
            if let Ok(sv) = find::<ScrollViewer>(&self.root, "TreeScroll") {
                let _ = sv.ChangeView(None, Some((k as f64 * ROW_H - 120.0).max(0.0)), None);
            }
        }
    }
}
