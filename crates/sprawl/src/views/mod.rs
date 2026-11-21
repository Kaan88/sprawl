//! The other views of the map panel (from the View Lab): each one builds coloured triangles,
//! lines, labels and hit shapes every frame from the scanned tree, drawn by the GPU's mesh pass.
//! The treemap, the city and the galaxy stay in `scene` and `galaxy`.

use std::collections::HashMap;

use sprawl_core::{Tree, ROOT};

use crate::gpu::{Label, Vertex};
use crate::state::{text, State};

mod flat;
mod packed;
mod flows;
mod time;
mod three;

/// One of the other views.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub(crate) enum Kind {
    Rings,
    Flame,
    Waffle,
    Library,
    Suds,
    Glass,
    Spectrum,
    Flow,
    Strata,
    Tideline,
    TreeRings,
    Isobars,
    Rose,
    Grove,
    Lens,
    Ridgeline,
    Molecule,
}

/// (view, its command, its name, what to do with it)
pub(crate) const ALL: [(Kind, &str, &str, &str); 17] = [
    (Kind::Rings, "ViewRings", "Rings", "Rings: one ring per folder level, arc length is size. Double-click a ring to zoom in, the centre to go back."),
    (Kind::Flame, "ViewFlame", "Flame Stack", "Flame Stack: width is size, height is depth. Double-click a bar to zoom in, the base to go back."),
    (Kind::Waffle, "ViewWaffle", "Waffle", "Waffle: every square is the same number of bytes; the glowing ones can be won back. The Showing button limits it to those."),
    (Kind::Library, "ViewLibrary", "Library", "Library: the biggest files as books, spine width is size, shelved by folder. Hover a book to pull it out."),
    (Kind::Suds, "ViewSuds", "Suds", "Suds: folders as bubbles, area is size. In Risk colour each fills with what can be won back."),
    (Kind::Glass, "ViewGlass", "Stained Glass", "Stained Glass: a Voronoi treemap, each pane's area is its size. Double-click a pane to go into it."),
    (Kind::Spectrum, "ViewSpectrum", "Spectrum", "Spectrum: every file type as a bar around the ring, length is the square root of its bytes."),
    (Kind::Flow, "ViewFlow", "Flow", "Flow: bytes from file type, to folder, to what the Advisor says about them. Hover a block."),
    (Kind::Strata, "ViewStrata", "Strata", "Strata: folders as layers, safe to clean on top, keep-it at the bottom; the pale cap is what can go."),
    (Kind::Tideline, "ViewTideline", "Tideline", "Tideline: the biggest folders across your scans as streams. Streams that swell are filling the disk."),
    (Kind::TreeRings, "ViewTreeRings", "Tree Rings", "Tree Rings: each big folder as a log, one ring per scan; warm rings are fast growth."),
    (Kind::Isobars, "ViewIsobars", "Isobars", "Isobars: a weather map of change since the scan before. Warm is growth, cool is shrinkage."),
    (Kind::Rose, "ViewRose", "Rose Stack", "Rose Stack: one disc per scan, oldest at the bottom; a wedge that flares is growing. Drag to turn."),
    (Kind::Grove, "ViewGrove", "Grove", "Grove: the folder tree as a tree, branch thickness is size, big files are leaves."),
    (Kind::Lens, "ViewLens", "Lens", "Lens: the whole tree in one disc. Double-click a node to bring it to the middle."),
    (Kind::Ridgeline, "ViewRidgeline", "Ridgeline", "Ridgeline: a landscape on the treemap, big files rise as ridges. Drag to turn, wheel to come closer."),
    (Kind::Molecule, "ViewMolecule", "Molecule", "Molecule: folders and big files as atoms, bonds are containment. Drag to turn."),
];

/// What the colours of the views mean.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, Hash)]
pub(crate) enum Paint {
    #[default]
    Types,
    Risk,
    Owner,
    Growth,
}

pub(crate) const PAINTS: [(Paint, &str, &str); 4] = [(Paint::Types, "PaintTypes", "Types"), (Paint::Risk, "PaintRisk", "Risk"), (Paint::Owner, "PaintOwner", "Owner"), (Paint::Growth, "PaintGrowth", "Growth")];

/// The More views menu: (group, its views). The last group is a submenu.
pub(crate) const GROUPS: [(&str, &[Kind]); 4] = [
    ("Size", &[Kind::Rings, Kind::Flame, Kind::Waffle, Kind::Suds, Kind::Glass]),
    ("What can go", &[Kind::Flow]),
    ("Over time", &[Kind::Tideline]),
    ("For fun", &[Kind::Library, Kind::Spectrum, Kind::Strata, Kind::TreeRings, Kind::Isobars, Kind::Rose, Kind::Grove, Kind::Lens, Kind::Ridgeline, Kind::Molecule]),
];

impl Kind {
    /// The hint's first clause: what the view shows, for its menu entry.
    pub(crate) fn short(self) -> &'static str {
        let h = self.hint();
        let body = h.split_once(": ").map_or(h, |x| x.1);
        let end = body.find(['.', ';', ',']).unwrap_or(body.len());
        &body[..end]
    }

    pub(crate) fn is_3d(self) -> bool {
        matches!(self, Kind::Rose | Kind::Ridgeline | Kind::Molecule)
    }
    pub(crate) fn name(self) -> &'static str {
        ALL.iter().find(|a| a.0 == self).map_or("", |a| a.2)
    }
    pub(crate) fn hint(self) -> &'static str {
        ALL.iter().find(|a| a.0 == self).map_or("", |a| a.3)
    }
}

/// A shape the pointer can hit, and the node it stands for.
#[derive(Clone, Debug)]
pub(crate) enum Shape {
    Rect([f32; 4]),
    /// Centre, inner and outer radius, start and end angle (radians, clockwise from up).
    Wedge([f32; 2], f32, f32, f32, f32),
    Circle([f32; 2], f32),
    /// A convex or simple polygon.
    Poly(Vec<[f32; 2]>),
}

impl Shape {
    fn contains(&self, x: f32, y: f32) -> bool {
        match self {
            Shape::Rect([rx, ry, w, h]) => x >= *rx && y >= *ry && x < rx + w && y < ry + h,
            Shape::Wedge([cx, cy], r0, r1, a0, a1) => {
                let (dx, dy) = (x - cx, y - cy);
                let r = (dx * dx + dy * dy).sqrt();
                let mut a = dx.atan2(-dy);
                if a < 0.0 {
                    a += std::f32::consts::TAU;
                }
                r >= *r0 && r < *r1 && a >= *a0 && a < *a1
            }
            Shape::Circle([cx, cy], r) => (x - cx).powi(2) + (y - cy).powi(2) < r * r,
            Shape::Poly(p) => {
                let mut inside = false;
                let mut j = p.len().wrapping_sub(1);
                for i in 0..p.len() {
                    let (a, b) = (p[i], p[j]);
                    if (a[1] > y) != (b[1] > y) && x < (b[0] - a[0]) * (y - a[1]) / (b[1] - a[1]) + a[0] {
                        inside = !inside;
                    }
                    j = i;
                }
                inside
            }
        }
    }
}

/// What a view draws in one frame.
#[derive(Default)]
pub(crate) struct Out {
    pub tris: Vec<Vertex>,
    pub lines: Vec<Vertex>,
    pub labels: Vec<Label>,
    pub hits: Vec<(Shape, usize)>,
    /// Hover text for shapes that aren't one node (a file type's bar).
    pub tips: Vec<(Shape, String)>,
}

pub(crate) fn rgba(c: [f32; 3], a: f32) -> [f32; 4] {
    [c[0], c[1], c[2], a]
}

pub(crate) fn mix(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] {
    [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t, a[2] + (b[2] - a[2]) * t]
}

pub(crate) fn scale(c: [f32; 3], k: f32) -> [f32; 3] {
    [(c[0] * k).min(1.0), (c[1] * k).min(1.0), (c[2] * k).min(1.0)]
}

impl Out {
    pub fn clear(&mut self) {
        self.tris.clear();
        self.lines.clear();
        self.labels.clear();
        self.hits.clear();
        self.tips.clear();
    }
    pub fn tri(&mut self, a: [f32; 2], b: [f32; 2], c: [f32; 2], col: [f32; 4]) {
        for p in [a, b, c] {
            self.tris.push(Vertex { pos: [p[0], p[1], 0.0], col });
        }
    }
    /// A triangle with a colour per corner.
    pub fn tri3(&mut self, p: [[f32; 2]; 3], c: [[f32; 4]; 3]) {
        for k in 0..3 {
            self.tris.push(Vertex { pos: [p[k][0], p[k][1], 0.0], col: c[k] });
        }
    }
    pub fn quad(&mut self, a: [f32; 2], b: [f32; 2], c: [f32; 2], d: [f32; 2], col: [f32; 4]) {
        self.tri(a, b, c, col);
        self.tri(a, c, d, col);
    }
    pub fn rect(&mut self, x: f32, y: f32, w: f32, h: f32, col: [f32; 4]) {
        self.quad([x, y], [x + w, y], [x + w, y + h], [x, y + h], col);
    }
    /// A rectangle shading from `top` to `bottom`.
    pub fn rect_v(&mut self, x: f32, y: f32, w: f32, h: f32, top: [f32; 4], bottom: [f32; 4]) {
        self.tri3([[x, y], [x + w, y], [x + w, y + h]], [top, top, bottom]);
        self.tri3([[x, y], [x + w, y + h], [x, y + h]], [top, bottom, bottom]);
    }
    pub fn line(&mut self, a: [f32; 2], b: [f32; 2], col: [f32; 4]) {
        self.lines.push(Vertex { pos: [a[0], a[1], 0.0], col });
        self.lines.push(Vertex { pos: [b[0], b[1], 0.0], col });
    }
    pub fn polyline(&mut self, p: &[[f32; 2]], col: [f32; 4]) {
        for w in p.windows(2) {
            self.line(w[0], w[1], col);
        }
    }
    /// A thick line as a quad.
    pub fn stroke(&mut self, a: [f32; 2], b: [f32; 2], width: f32, col: [f32; 4]) {
        let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
        let l = (dx * dx + dy * dy).sqrt().max(1e-4);
        let (nx, ny) = (-dy / l * width * 0.5, dx / l * width * 0.5);
        self.quad([a[0] + nx, a[1] + ny], [b[0] + nx, b[1] + ny], [b[0] - nx, b[1] - ny], [a[0] - nx, a[1] - ny], col);
    }
    /// A ring segment; angles clockwise from straight up.
    pub fn wedge(&mut self, c: [f32; 2], r0: f32, r1: f32, a0: f32, a1: f32, col: [f32; 4]) {
        let steps = (((a1 - a0) * r1 / 5.0).ceil() as usize).clamp(1, 96);
        let at = |r: f32, a: f32| [c[0] + r * a.sin(), c[1] - r * a.cos()];
        for k in 0..steps {
            let (s, e) = (a0 + (a1 - a0) * k as f32 / steps as f32, a0 + (a1 - a0) * (k + 1) as f32 / steps as f32);
            self.quad(at(r0, s), at(r1, s), at(r1, e), at(r0, e), col);
        }
    }
    pub fn disc(&mut self, c: [f32; 2], r: f32, col: [f32; 4]) {
        self.wedge(c, 0.0, r, 0.0, std::f32::consts::TAU, col);
    }
    pub fn circle_line(&mut self, c: [f32; 2], r: f32, col: [f32; 4]) {
        let steps = ((r * 0.8) as usize).clamp(12, 120);
        let pts: Vec<[f32; 2]> = (0..=steps).map(|k| {
            let a = k as f32 / steps as f32 * std::f32::consts::TAU;
            [c[0] + r * a.sin(), c[1] - r * a.cos()]
        }).collect();
        self.polyline(&pts, col);
    }
    pub fn label(&mut self, x: f32, y: f32, max_w: f32, text: String, strong: bool) {
        if max_w >= 24.0 {
            self.labels.push(Label { x, y, max_w, text, strong });
        }
    }
    pub fn hit(&mut self, s: Shape, n: usize) {
        self.hits.push((s, n));
    }
    pub fn tip(&mut self, s: Shape, text: String) {
        self.tips.push((s, text));
    }
    pub fn tip_at(&self, x: f32, y: f32) -> Option<&str> {
        self.tips.iter().rev().find(|(s, _)| s.contains(x, y)).map(|(_, t)| t.as_str())
    }
    /// What is under the pointer: the last shape drawn there.
    pub fn pick(&self, x: f32, y: f32) -> Option<usize> {
        self.hits.iter().rev().find(|(s, _)| s.contains(x, y)).map(|(_, n)| *n)
    }
}

/// What a view gets to build its frame.
pub(crate) struct Ctx<'a> {
    pub tree: &'a Tree,
    /// Changes whenever the tree does (for what a view keeps between frames).
    pub gen: u64,
    pub root: usize,
    /// The size shown (length or allocated).
    pub size: &'a dyn Fn(usize) -> u64,
    /// Every node's colour under the colour switch.
    pub colour: &'a [[f32; 3]],
    /// Bytes below each node the Advisor says can be won back (empty when there is no report).
    pub reclaim: &'a [u64],
    /// The Advisor's risk code of each node (0 none, 1 green, 2 amber, 3 orange), empty without a report.
    pub risk: &'a [u8],
    pub paint: Paint,
    pub w: f32,
    pub h: f32,
    /// Seconds since this view (or its root) appeared, and since the program started.
    pub t: f32,
    pub time: f32,
    pub hover: Option<usize>,
    pub selected: Option<usize>,
    pub fmt: &'a dyn Fn(u64) -> String,
    /// Earlier scans, oldest first: (when, size of each node of this tree then).
    pub history: &'a [(std::time::SystemTime, Vec<u64>)],
    /// File type colours.
    pub ext_colour: &'a dyn Fn(&str) -> [f32; 3],
}

impl Ctx<'_> {
    pub fn col(&self, n: usize) -> [f32; 3] {
        self.colour.get(n).copied().unwrap_or([0.4, 0.42, 0.48])
    }
    pub fn reclaim_of(&self, n: usize) -> u64 {
        self.reclaim.get(n).copied().unwrap_or(0)
    }
    pub fn risk_of(&self, n: usize) -> u8 {
        self.risk.get(n).copied().unwrap_or(0)
    }
    /// The node's children with something in them, biggest first.
    pub fn kids(&self, n: usize) -> Vec<usize> {
        let mut k: Vec<usize> = self.tree.nodes[n].children.iter().copied().filter(|&c| (self.size)(c) > 0).collect();
        k.sort_by_key(|&c| std::cmp::Reverse((self.size)(c)));
        k
    }
    /// "name\ (size)"
    pub fn name(&self, n: usize) -> String {
        let node = &self.tree.nodes[n];
        format!("{}{} ({})", node.name, if node.is_dir { "\\" } else { "" }, (self.fmt)((self.size)(n)))
    }
    /// Highlight when hovered or selected.
    pub fn lit(&self, n: usize) -> f32 {
        if self.hover == Some(n) {
            1.35
        } else if self.selected == Some(n) {
            1.2
        } else {
            1.0
        }
    }
    /// 0..1 over `secs` from when the view appeared, eased.
    pub fn intro(&self, secs: f32) -> f32 {
        let x = (self.t / secs).clamp(0.0, 1.0);
        1.0 - (1.0 - x).powi(3)
    }
}

/// A 3D view's camera: turned by dragging, closer with the wheel.
#[derive(Clone, Copy)]
pub(crate) struct Cam {
    pub yaw: f32,
    pub tilt: f32,
    pub dist: f32,
}

impl Default for Cam {
    fn default() -> Self {
        Cam { yaw: 0.6, tilt: 0.55, dist: 1.0 }
    }
}

/// The views' state, kept in the map.
#[derive(Default)]
pub(crate) struct Views {
    pub kind: Option<Kind>,
    pub paint: Paint,
    /// The map key's XAML, as last loaded.
    pub key_shown: String,
    /// With animations off: what the last frame was built from, and when that last changed.
    pub stamp: u64,
    pub stamp_at: f32,
    pub out: Out,
    pub hover: Option<usize>,
    /// When the view or its root last changed (program seconds).
    pub since: f32,
    pub shown: Option<(Kind, usize, u64)>,
    pub cam: Cam,
    pub vp: [f32; 16],
    pub eye: [f32; 3],
    /// Colours, reclaim and risk per node, for (tree generation, colour switch, report generation).
    cache: Option<(u64, Paint, Option<u64>, Vec<[f32; 3]>, Vec<u64>, Vec<u8>)>,
    /// Earlier scans of this tree (by tree generation), and the ones still loading.
    history: Option<(u64, Vec<(std::time::SystemTime, Vec<u64>)>)>,
    history_rx: Option<(u64, std::sync::mpsc::Receiver<Vec<(std::time::SystemTime, Vec<u64>)>>)>,
    /// Per-view state: the bubbles, the glass seeds, the molecule, the lens focus, toggles.
    pub suds: HashMap<usize, [f32; 4]>,
    pub glass: Vec<(usize, [f32; 2], f32)>,
    pub glass_for: Option<packed::GlassFor>,
    pub atoms: HashMap<usize, [f32; 6]>,
    pub atoms_for: Option<(usize, u64)>,
    pub lens_focus: Option<usize>,
    pub waffle_only: bool,
    pub terrain: Option<(usize, u64, Paint, three::Terrain)>,
}

fn time_now() -> f32 {
    static T0: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();
    T0.get_or_init(std::time::Instant::now).elapsed().as_secs_f32()
}

impl State {
    /// One of the other views is on, over a finished scan.
    pub(crate) fn views_on(&self) -> bool {
        self.map.views.kind.is_some() && self.tree.is_some() && self.map.live.is_none()
    }

    /// Switch to view `k` (None: back to the treemap, city or galaxy buttons).
    pub(crate) fn set_view_kind(&mut self, k: Option<Kind>) {
        self.map.views.kind = k;
        self.map.views.hover = None;
        self.map.views.shown = None;
        self.map.views.cam = Cam::default();
        if k.is_some() {
            // the three buttons all look off while another view is on
            for name in ["ModeMap", "ModeCity", "ModeGalaxy"] {
                crate::look::paint(&self.root, name, crate::look::Pick::Mode, false);
            }
        }
        crate::look::paint(&self.root, "ViewMore", crate::look::Pick::Mode, k.is_some());
        text(&self.root, "ViewMoreText", k.map_or("More views", |k| k.name()));
        if let Ok(e) = crate::state::find::<crate::bindings::Microsoft::UI::Xaml::UIElement>(&self.root, "ViewOnly") {
            let _ = e.SetVisibility(if k == Some(Kind::Waffle) { crate::bindings::Microsoft::UI::Xaml::Visibility::Visible } else { crate::bindings::Microsoft::UI::Xaml::Visibility::Collapsed });
        }
        // the view's hint as a caption on the map, where it is looked at
        text(&self.root, "MapCaption", k.map_or("", |k| k.hint()));
        if let Ok(e) = crate::state::find::<crate::bindings::Microsoft::UI::Xaml::UIElement>(&self.root, "MapCaption") {
            let _ = e.SetVisibility(if k.is_some() { crate::bindings::Microsoft::UI::Xaml::Visibility::Visible } else { crate::bindings::Microsoft::UI::Xaml::Visibility::Collapsed });
        }
        // remembered for next time
        if let Some(k) = k {
            if let Some(cmd) = ALL.iter().find(|a| a.0 == k).map(|a| a.1) {
                self.settings.set("view", cmd);
                self.settings.save();
            }
        }
        self.map.drawn_once = false;
        self.tweaks_room();
        self.render_map();
    }

    pub(crate) fn set_paint(&mut self, p: Paint) {
        self.map.views.paint = p;
        text(&self.root, "ViewPaintText", &format!("Colour: {}", PAINTS.iter().find(|x| x.0 == p).map_or("", |x| x.2)));
        if let Some(cmd) = PAINTS.iter().find(|x| x.0 == p).map(|x| x.1) {
            self.settings.set("paint", cmd);
            self.settings.save();
        }
        self.map.drawn_once = false;
        self.render_map();
    }

    /// The key in the map's corner: what the colours mean now, or why there is nothing to show.
    pub(crate) fn update_map_key(&mut self) {
        let hex = |c: [f32; 3]| format!("#FF{:02X}{:02X}{:02X}", (c[0].clamp(0.0, 1.0) * 255.0) as u8, (c[1].clamp(0.0, 1.0) * 255.0) as u8, (c[2].clamp(0.0, 1.0) * 255.0) as u8);
        let swatch = |c: [f32; 3], label: &str| {
            format!(
                r##"<StackPanel Orientation="Horizontal" Spacing="5"><Rectangle Width="10" Height="10" RadiusX="2" RadiusY="2" Fill="{}" VerticalAlignment="Center"/><TextBlock Text="{}" FontSize="12" Foreground="{{$text1}}"/></StackPanel>"##,
                hex(c),
                crate::ui::esc(label)
            )
        };
        let note = |t: &str| format!(r##"<TextBlock Text="{}" FontSize="12" Foreground="{{$text2}}"/>"##, crate::ui::esc(t));
        let grey = [0.30, 0.33, 0.40];
        let paint = self.map.views.paint;
        let report = self.advice().is_some();
        let body = match paint {
            Paint::Types => {
                let mut top: Vec<&crate::state::ExtRow> = self.exts.iter().collect();
                top.sort_by_key(|e| std::cmp::Reverse(e.size));
                top.iter().take(6).map(|e| swatch(self.colours.get(&e.ext).map(|&k| palette(k)).unwrap_or(grey), &if e.ext.is_empty() { "(none)".to_string() } else { format!(".{}", e.ext) })).collect::<String>()
            }
            Paint::Risk if !report => note("Risk needs the Advisor's check: open the Advisor tab."),
            Paint::Risk if self.advice().is_some_and(|r| !r.lens.iter().any(|&c| c > 0)) => note("Risk: nothing here can be cleaned, so nothing is lit."),
            Paint::Risk => [(1u8, "Safe to delete"), (2, "Takes time to get back"), (3, "Your own files")].iter().map(|(c, l)| swatch(crate::advisor::lens_colour(*c), l)).collect::<String>() + &swatch(grey, "Nothing found"),
            Paint::Owner if !self.views_on() => note("Owner colours show only in the More views (Rings, Suds and others)."),
            Paint::Owner if !report => note("Owner needs the Advisor's check: open the Advisor tab."),
            Paint::Owner => note("One colour per app") + &swatch(grey, "No known owner"),
            Paint::Growth if !report || self.advice().is_some_and(|r| r.growth.is_none()) => note("Growth needs an earlier scan of this folder, a day or more ago. History plays what there is."),
            Paint::Growth if self.advisor.growth_lens.as_ref().is_none_or(|(_, l)| !l.iter().any(|&c| c > 0)) => note("Growth: nothing grew since the scan before."),
            Paint::Growth => note("Lit: what grew since the scan before. More in Advisor > Growth."),
        };
        let xaml = format!(r##"<StackPanel {} Orientation="Horizontal" Spacing="12">{body}</StackPanel>"##, crate::ui::NS);
        if self.map.views.key_shown == xaml {
            return;
        }
        let Ok(host) = crate::state::find::<crate::bindings::Microsoft::UI::Xaml::Controls::Border>(&self.root, "MapKey") else { return };
        if let Ok(el) = crate::state::load::<crate::bindings::Microsoft::UI::Xaml::UIElement>(&xaml) {
            let _ = host.SetChild(&el);
            self.map.views.key_shown = xaml;
        }
    }

    /// The colours, reclaimable bytes and risk of every node, worked out again when the tree, the
    /// colour switch or the Advisor's report changed.
    fn views_cache(&mut self) {
        let Some(t) = self.tree.clone() else { return };
        let paint = self.map.views.paint;
        let report_gen = self.advice().map(|r| r.tree);
        if self.map.views.cache.as_ref().is_some_and(|c| c.0 == self.tree_gen && c.1 == paint && c.2 == report_gen) {
            return;
        }
        let n = t.nodes.len();
        // lens codes from the report: risk of each node, and the growth codes
        let risk: Vec<u8> = self.advice().filter(|r| r.lens.len() == n).map(|r| r.lens.to_vec()).unwrap_or_default();
        let growth: Option<std::sync::Arc<Vec<u8>>> = self.advisor.growth_lens.as_ref().filter(|(g, l)| *g == self.tree_gen && l.len() == n).map(|(_, l)| l.clone());
        let mut reclaim = vec![0u64; if risk.is_empty() { 0 } else { n }];
        if let Some(r) = self.advice() {
            if !reclaim.is_empty() {
                for (i, f) in r.findings.iter().enumerate() {
                    if !r.counted.get(i).copied().unwrap_or(false) || !matches!(f.remedy, sprawl_advisor::Remedy::Delete | sprawl_advisor::Remedy::Command) {
                        continue;
                    }
                    let Some(k) = r.nodes.get(i).copied().flatten() else { continue };
                    let mut p = Some(k);
                    while let Some(q) = p {
                        reclaim[q] = reclaim[q].saturating_add(f.bytes);
                        p = t.nodes[q].parent;
                    }
                }
            }
        }
        let owners: Vec<Option<u32>> = self.advice().filter(|r| r.owners.len() == n).map(|r| r.owners.clone()).unwrap_or_default();
        let grey = [0.30, 0.33, 0.40];
        let colours = &self.colours;
        let type_colour = |i: usize| -> [f32; 3] {
            let ext = t.nodes[i].extension();
            colours.get(&ext).map(|&k| palette(k)).unwrap_or([crate::paint::GREY.0 as f32 / 255.0, crate::paint::GREY.1 as f32 / 255.0, crate::paint::GREY.2 as f32 / 255.0])
        };
        // a node's own colour, when it has one; folders without one take their contents' mix
        let fixed = |i: usize| -> Option<[f32; 3]> {
            match paint {
                Paint::Types => (!t.nodes[i].is_dir).then(|| type_colour(i)),
                Paint::Risk => match risk.get(i).copied().unwrap_or(0) {
                    0 => (!t.nodes[i].is_dir).then_some(grey),
                    c => Some(crate::advisor::lens_colour(c)),
                },
                Paint::Owner => match owners.get(i).copied().flatten() {
                    Some(o) => Some(palette(o as usize * 7 + 3)),
                    None => (!t.nodes[i].is_dir).then_some(grey),
                },
                Paint::Growth => match growth.as_ref().map_or(0, |g| g[i]) {
                    0 => (!t.nodes[i].is_dir).then_some(grey),
                    c => Some(crate::advisor::lens_colour(c)),
                },
            }
        };
        // bottom-up, weighted by size: a pre-order list walked backwards
        let mut order = Vec::with_capacity(n);
        let mut stack = vec![ROOT];
        while let Some(i) = stack.pop() {
            order.push(i);
            stack.extend_from_slice(&t.nodes[i].children);
        }
        let mut acc = vec![[0f64; 4]; n];
        let mut out = vec![grey; n];
        // (size, colour) of each folder's biggest child, for Types
        let mut biggest: Vec<(u64, Option<[f32; 3]>)> = vec![(0, None); n];
        for &i in order.iter().rev() {
            let c = match fixed(i) {
                Some(c) => c,
                None if paint == Paint::Types && biggest[i].1.is_some() => biggest[i].1.unwrap(),
                None if acc[i][3] > 0.0 => [(acc[i][0] / acc[i][3]) as f32, (acc[i][1] / acc[i][3]) as f32, (acc[i][2] / acc[i][3]) as f32],
                None => grey,
            };
            out[i] = c;
            if let Some(p) = t.nodes[i].parent {
                if t.nodes[i].size >= biggest[p].0 {
                    biggest[p] = (t.nodes[i].size, Some(c));
                }
                let w = t.nodes[i].size.max(1) as f64;
                for k in 0..3 {
                    acc[p][k] += c[k] as f64 * w;
                }
                acc[p][3] += w;
            }
        }
        self.map.views.cache = Some((self.tree_gen, paint, report_gen, out, reclaim, risk));
    }

    /// Earlier scans of this folder, as sizes of this tree's nodes then (oldest first), loaded
    /// on another thread (a whole drive's snapshot takes a while).
    fn views_history(&mut self) {
        let gen = self.tree_gen;
        let v = &mut self.map.views;
        if v.history.as_ref().is_some_and(|h| h.0 == gen) {
            return;
        }
        if let Some((g, rx)) = &v.history_rx {
            if *g == gen {
                if let Ok(list) = rx.try_recv() {
                    v.history = Some((gen, list));
                    v.history_rx = None;
                }
                return;
            }
        }
        let (Some(t), Some(base)) = (self.tree.clone(), sprawl_advisor::history::base_dir()) else { return };
        let me = self.scan_taken - std::time::Duration::from_secs(2);
        let (tx, rx) = std::sync::mpsc::channel();
        v.history_rx = Some((gen, rx));
        std::thread::spawn(move || {
            let mut entries: Vec<sprawl_advisor::history::Entry> = sprawl_advisor::history::covering(&base, &t.path_of(ROOT)).into_iter().filter(|e| e.taken < me).collect();
            entries.sort_by_key(|e| e.taken);
            let skip = entries.len().saturating_sub(7);
            let list = entries[skip..]
                .iter()
                .filter_map(|e| sprawl_advisor::history::load(e).ok().map(|old| (e.taken, sprawl_advisor::history::project(&t, &old))))
                .collect();
            let _ = tx.send(list);
        });
    }

    /// Build this frame of the view on; true while it moves.
    pub(crate) fn views_frame(&mut self) -> bool {
        let Some(kind) = self.map.views.kind else { return false };
        self.views_cache();
        if matches!(kind, Kind::Tideline | Kind::TreeRings | Kind::Isobars | Kind::Rose) {
            self.views_history();
        }
        let Some(t) = self.tree.clone() else { return false };
        let now = time_now();
        let key = (kind, self.map_root, self.tree_gen);
        if self.map.views.shown != Some(key) {
            self.map.views.shown = Some(key);
            self.map.views.since = now;
        }
        // Animations off: no intro, no ambient motion, and no redraw once nothing changes (the
        // packed layouts get a few seconds to settle after a change)
        let animate = self.map.scene.animate;
        if !animate {
            use std::hash::{Hash, Hasher};
            let v = &self.map.views;
            let mut h = std::collections::hash_map::DefaultHasher::new();
            (key, v.paint as u8, v.hover, self.selected, v.lens_focus, v.waffle_only, self.map.show_labels).hash(&mut h);
            for f in [v.cam.yaw, v.cam.tilt, v.cam.dist, self.map.scene.w, self.map.scene.h] {
                f.to_bits().hash(&mut h);
            }
            (v.history.as_ref().map(|x| x.1.len()), v.cache.as_ref().map(|c| (c.0, c.2))).hash(&mut h);
            let stamp = h.finish();
            if stamp != v.stamp {
                self.map.views.stamp = stamp;
                self.map.views.stamp_at = now;
            } else if now - v.stamp_at > 3.0 && self.map.drawn_once {
                return false;
            }
        }
        let by_alloc = self.map.by_alloc && self.alloc.len() == t.nodes.len();
        let alloc = &self.alloc;
        let size = |n: usize| if by_alloc { alloc[n] } else { t.nodes[n].size };
        let units = self.units;
        let fmt = |v: u64| crate::state::size_with(units, v);
        let colours = &self.colours;
        let ext_colour = |e: &str| colours.get(e).map(|&k| palette(k)).unwrap_or([0.45, 0.45, 0.48]);
        let v = &mut self.map.views;
        let (_, _, _, colour, reclaim, risk) = v.cache.as_ref().expect("cache");
        let empty = Vec::new();
        let history = v.history.as_ref().map_or(&empty, |h| &h.1);
        let ctx = Ctx {
            tree: &t,
            gen: self.tree_gen,
            root: self.map_root,
            size: &size,
            colour,
            reclaim,
            risk,
            paint: v.paint,
            w: self.map.scene.w,
            h: self.map.scene.h,
            t: if animate { now - v.since } else { 100.0 },
            time: if animate { now } else { 0.0 },
            hover: v.hover,
            selected: self.selected,
            fmt: &fmt,
            history,
            ext_colour: &ext_colour,
        };
        let mut out = std::mem::take(&mut v.out);
        out.clear();
        // the per-view state, apart from what ctx borrows
        let mut st = ViewState {
            suds: std::mem::take(&mut v.suds),
            glass: std::mem::take(&mut v.glass),
            glass_for: v.glass_for,
            atoms: std::mem::take(&mut v.atoms),
            atoms_for: v.atoms_for,
            lens_focus: v.lens_focus,
            waffle_only: v.waffle_only,
            terrain: v.terrain.take(),
            cam: v.cam,
            vp: v.vp,
            eye: v.eye,
        };
        match kind {
            Kind::Rings => flat::rings(&ctx, &mut out),
            Kind::Flame => flat::flame(&ctx, &mut out),
            Kind::Waffle => flat::waffle(&ctx, &mut out, st.waffle_only),
            Kind::Library => flat::library(&ctx, &mut out),
            Kind::Suds => packed::suds(&ctx, &mut out, &mut st.suds),
            Kind::Glass => packed::glass(&ctx, &mut out, &mut st.glass, &mut st.glass_for),
            Kind::Spectrum => packed::spectrum(&ctx, &mut out),
            Kind::Flow => flows::flow(&ctx, &mut out),
            Kind::Strata => flows::strata(&ctx, &mut out),
            Kind::Tideline => time::tideline(&ctx, &mut out),
            Kind::TreeRings => time::tree_rings(&ctx, &mut out),
            Kind::Isobars => time::isobars(&ctx, &mut out),
            Kind::Grove => flows::grove(&ctx, &mut out),
            Kind::Lens => flows::lens(&ctx, &mut out, st.lens_focus),
            Kind::Rose => three::rose(&ctx, &mut out, &mut st),
            Kind::Ridgeline => three::ridgeline(&ctx, &mut out, &mut st),
            Kind::Molecule => three::molecule(&ctx, &mut out, &mut st),
        }
        let v = &mut self.map.views;
        v.out = out;
        v.suds = st.suds;
        v.glass = st.glass;
        v.glass_for = st.glass_for;
        v.atoms = st.atoms;
        v.atoms_for = st.atoms_for;
        v.terrain = st.terrain;
        v.vp = st.vp;
        v.eye = st.eye;
        true
    }

    /// The pointer over a view: what is under it, shown in the map's path line.
    pub(crate) fn views_hover(&mut self, x: f64, y: f64, rotate: bool, dx: f32, dy: f32) {
        let v = &mut self.map.views;
        if rotate && v.kind.is_some_and(Kind::is_3d) {
            v.cam.yaw += dx * 0.008;
            v.cam.tilt = (v.cam.tilt + dy * 0.006).clamp(0.12, 1.35);
        }
        let n = v.out.pick(x as f32, y as f32);
        if n.is_none() {
            if let Some(tip) = v.out.tip_at(x as f32, y as f32).map(str::to_string) {
                v.hover = None;
                text(&self.root, "MapPath", &tip);
                return;
            }
        }
        if n == v.hover {
            return;
        }
        v.hover = n;
        if let Some(t) = &self.tree {
            let line = match n.filter(|&n| n < t.nodes.len()) {
                Some(n) => {
                    let extra = self.map.views.out_note(n, t, &|b| self.size(b));
                    format!("{}   ({}){extra}", t.path_of(n).display(), self.size(t.nodes[n].size))
                }
                None => t.path_of(self.map_root).display().to_string(),
            };
            text(&self.root, "MapPath", &line);
        }
    }

    pub(crate) fn views_pick(&self, x: f64, y: f64) -> Option<usize> {
        let len = self.tree.as_ref().map_or(0, |t| t.nodes.len());
        self.map.views.out.pick(x as f32, y as f32).filter(|&n| n < len)
    }

    /// One rule in every view: a click selects (the tree follows), a double-click goes in, the
    /// wheel zooms. Lens goes in by re-centring on what was double-clicked. True when handled.
    pub(crate) fn views_double(&mut self, x: f64, y: f64) -> bool {
        if self.map.views.kind == Some(Kind::Lens) {
            if let Some(n) = self.views_pick(x, y) {
                self.map.views.lens_focus = Some(n);
                self.map.views.since = time_now();
            }
            return true;
        }
        false
    }

    /// Waffle's "only what can go" switch.
    pub(crate) fn views_only_toggle(&mut self) {
        self.map.views.waffle_only = !self.map.views.waffle_only;
        text(&self.root, "ViewOnlyText", if self.map.views.waffle_only { "Showing: what can go" } else { "Showing: everything" });
        self.map.drawn_once = false;
        self.render_map();
    }

    /// The wheel over a 3D view comes closer; elsewhere it goes up or into folders.
    pub(crate) fn views_wheel(&mut self, delta: i32) -> bool {
        let v = &mut self.map.views;
        if v.kind.is_some_and(Kind::is_3d) {
            v.cam.dist = (v.cam.dist * (1.0 - delta as f32 / 1200.0)).clamp(0.3, 3.0);
            return true;
        }
        false
    }
}

impl Views {
    /// What the path line adds for a node in this view (what can be won back in it).
    fn out_note(&self, n: usize, _t: &Tree, fmt: &dyn Fn(u64) -> String) -> String {
        let r = self.cache.as_ref().and_then(|c| c.4.get(n).copied()).unwrap_or(0);
        if r > 0 {
            format!("   ·   {} can be won back", fmt(r))
        } else {
            String::new()
        }
    }
}

/// The per-view state handed to a builder.
pub(crate) struct ViewState {
    pub suds: HashMap<usize, [f32; 4]>,
    pub glass: Vec<(usize, [f32; 2], f32)>,
    pub glass_for: Option<packed::GlassFor>,
    pub atoms: HashMap<usize, [f32; 6]>,
    pub atoms_for: Option<(usize, u64)>,
    pub lens_focus: Option<usize>,
    pub waffle_only: bool,
    pub terrain: Option<(usize, u64, Paint, three::Terrain)>,
    pub cam: Cam,
    pub vp: [f32; 16],
    pub eye: [f32; 3],
}

pub(crate) fn palette(i: usize) -> [f32; 3] {
    let (r, g, b) = crate::paint::colour(i);
    [r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0]
}

/// A small, stable pseudo-random number in 0..1 from a seed.
pub(crate) fn hash01(seed: u64) -> f32 {
    let mut x = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15);
    x ^= x >> 29;
    x = x.wrapping_mul(0xBF58_476D_1CE4_E5B9);
    x ^= x >> 32;
    (x >> 40) as f32 / (1u64 << 24) as f32
}

thread_local! {
    static MEMO: std::cell::RefCell<HashMap<(&'static str, usize, u64), std::rc::Rc<dyn std::any::Any>>> = std::cell::RefCell::new(HashMap::new());
}

/// Work a view does once per tree and root (walking every file below it), kept between frames.
pub(crate) fn memo<T: 'static>(name: &'static str, c: &Ctx, make: impl FnOnce() -> T) -> std::rc::Rc<T> {
    let key = (name, c.root, c.gen);
    if let Some(v) = MEMO.with(|m| m.borrow().get(&key).cloned()) {
        if let Ok(v) = v.downcast::<T>() {
            return v;
        }
    }
    let v = std::rc::Rc::new(make());
    MEMO.with(|m| {
        let mut m = m.borrow_mut();
        // one entry per view: older roots and trees go
        m.retain(|k, _| k.0 != name);
        m.insert(key, v.clone());
    });
    v
}

/// Every file below `root`, biggest first (at most `most`).
pub(crate) fn biggest_files(c: &Ctx, most: usize) -> std::rc::Rc<Vec<usize>> {
    memo("files", c, || {
        let mut files: Vec<usize> = Vec::new();
        let mut stack = vec![c.root];
        while let Some(n) = stack.pop() {
            for &k in &c.tree.nodes[n].children {
                if c.tree.nodes[k].is_dir {
                    stack.push(k);
                } else if (c.size)(k) > 0 {
                    files.push(k);
                }
            }
        }
        let keep = most.min(files.len());
        if keep > 0 && keep < files.len() {
            files.select_nth_unstable_by_key(keep - 1, |&f| std::cmp::Reverse((c.size)(f)));
            files.truncate(keep);
        }
        files.sort_by_key(|&f| std::cmp::Reverse((c.size)(f)));
        files
    })
}
