//! The treemap panel: keeps the GPU renderer and the scene in step with the window, lays out
//! the scanned tree (or the live picture while a scan runs) and handles the mouse over it.

use std::collections::HashMap;
use std::time::Instant;

use disktree_core::{Node, Tree, ROOT};
use windows_core::Interface;

use crate::bindings::Microsoft::UI::Xaml::Controls::{ScrollViewer, SwapChainPanel};
use crate::bindings::Microsoft::UI::Xaml::FrameworkElement;
use crate::gpu::{Frame, Gpu};
use crate::scene::{ext_key, Info, Scene};
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
            capture: None,
        }
    }
}

fn palette_colour(i: usize) -> [f32; 3] {
    let (r, g, b) = paint::PALETTE[i % paint::PALETTE.len()];
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

    fn relayout(&mut self) {
        let m = &mut self.map;
        if let Some(live) = &m.live {
            let lt = &live.tree;
            m.scene.relayout(lt, ROOT, &|n| lt.nodes[n].size, &|n| Info { colour: palette_colour(n.saturating_sub(1)), ext_key: 0 });
        } else if let Some(t) = &self.tree {
            let alloc = &self.alloc;
            let by_alloc = m.by_alloc && alloc.len() == t.nodes.len();
            let colours = &self.colours;
            m.scene.relayout(t, self.map_root, &|n| if by_alloc { alloc[n] } else { t.nodes[n].size }, &|n| {
                let ext = t.nodes[n].extension();
                let colour = colours
                    .get(&ext)
                    .map(|&i| palette_colour(i))
                    .unwrap_or([paint::GREY.0 as f32 / 255.0, paint::GREY.1 as f32 / 255.0, paint::GREY.2 as f32 / 255.0]);
                Info { colour, ext_key: ext_key(&ext) }
            });
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
        self.hud_frame(dt);

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
                        Ok(g) => self.map.gpu = Some(g),
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
        // A selected folder lights up its whole block on the map.
        if self.map.live.is_none() {
            if let Some(t) = &self.tree {
                let folder = self.selected.filter(|&n| n != self.map_root && t.nodes.get(n).is_some_and(|n| n.is_dir));
                self.map.scene.set_hover_folder(t, folder);
            }
        }
        let busy = self.map.scene.update(dt);
        let want_capture = self.map.capture.is_some();
        if busy || !self.map.drawn_once || want_capture {
            if let Some(g) = &self.map.gpu {
                if g.ready() || want_capture {
                    let labels = if self.map.show_labels { self.map_labels() } else { Vec::new() };
                    let m = &mut self.map;
                    let frame = Frame {
                        vp: m.scene.vp,
                        eye: m.scene.eye,
                        light: [-0.45, 0.55, 0.75],
                        size: [m.scene.w, m.scene.h],
                        cushion: m.scene.cushion,
                        instances: &m.scene.instances,
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

    fn map_labels(&self) -> Vec<crate::gpu::Label> {
        let m = &self.map;
        if let Some(live) = &m.live {
            return m.scene.labels(&live.tree, &|_| String::new()).into_iter().filter(|l| !l.text.is_empty()).collect();
        }
        let Some(t) = &self.tree else { return Vec::new() };
        let by_alloc = m.by_alloc && self.alloc.len() == t.nodes.len();
        m.scene.labels(t, &|n| {
            let v = if by_alloc { self.alloc[n] } else { t.nodes[n].size };
            format!("{}\\ ({})", t.nodes[n].name, self.size(v))
        })
    }

    // ------------------------------------------------------------ pointer

    pub(crate) fn map_hover(&mut self, x: f64, y: f64) {
        if self.shot.is_some() {
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
        if let (Some(t), Some(n)) = (&self.tree, n) {
            let v = t.nodes[n].size;
            text(&self.root, "MapPath", &format!("{}   ({})", t.path_of(n).display(), self.size(v)));
        }
    }

    pub(crate) fn map_leave(&mut self) {
        self.map.scene.leave();
        if let Some(t) = &self.tree {
            text(&self.root, "MapPath", &format!("{}", t.path_of(self.map_root).display()));
        }
    }

    pub(crate) fn map_node_at(&self, x: f64, y: f64) -> Option<usize> {
        if self.map.live.is_some() {
            return None;
        }
        self.map.scene.hit(x as f32, y as f32)
    }

    pub(crate) fn map_click(&mut self, x: f64, y: f64) {
        let Some(n) = self.map_node_at(x, y) else { return };
        self.reveal(n);
    }

    /// Double-click: zoom into the folder one level below the current treemap root.
    pub(crate) fn map_zoom_at(&mut self, x: f64, y: f64) {
        let Some(n) = self.map_node_at(x, y) else { return };
        let Some(t) = &self.tree else { return };
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
        if delta > 0 {
            self.map_zoom_at(x, y);
        } else if let Some(p) = self.tree.as_ref().and_then(|t| t.nodes[self.map_root].parent) {
            self.map_root = p;
            self.render_map();
        }
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
