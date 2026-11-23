//! dev (SPRAWL_TIMING): what the benchmark (tools/bench.ps1) reads from the timing log.
//!
//! - Start-up marks, each measured from the process's creation as Windows recorded it (not
//!   from `main`): `bench mark <what>: <ms> ms`.
//! - Every frame's parts while something is being measured: the app's own work in the
//!   Rendering callback, then layout (the last LayoutUpdated), then the frame rendered
//!   (CompositionTarget.Rendered).
//! - A dev step of a screenshot run (SPRAWL_SWITCH, `State::shot_tick`) as an action: from the
//!   step to the frame that shows it, and on until the frames are quiet again, with the XAML
//!   parsed on the way (`load()`): `bench act <step>: ...`.
//!
//! Off, each hook is one cached flag test.

use std::cell::RefCell;
use std::sync::OnceLock;
use std::time::Instant;

use crate::bindings::Microsoft::UI::Xaml::FrameworkElement;

/// Whether the timing log is on: SPRAWL_TIMING set, and never as administrator (the log is in
/// the user's TEMP, as `sprawl_core::timing`'s).
pub(crate) fn on() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("SPRAWL_TIMING").is_some() && sprawl_core::elevation().ok() == Some(false))
}

/// One line in the timing log.
fn log(line: &str) {
    use std::io::Write;
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(std::env::temp_dir().join("sprawl-timing.log")) {
        let _ = writeln!(f, "{line}");
    }
}

/// The process's creation time (GetProcessTimes), as an `Instant` in this process's clock.
fn born() -> Instant {
    static BORN: OnceLock<Instant> = OnceLock::new();
    *BORN.get_or_init(|| {
        #[repr(C)]
        #[derive(Default)]
        struct FileTime {
            lo: u32,
            hi: u32,
        }
        #[link(name = "kernel32")]
        extern "system" {
            fn GetCurrentProcess() -> isize;
            fn GetProcessTimes(p: isize, created: *mut FileTime, exited: *mut FileTime, kernel: *mut FileTime, user: *mut FileTime) -> i32;
        }
        let now = Instant::now();
        let (mut c, mut e, mut k, mut u) = (FileTime::default(), FileTime::default(), FileTime::default(), FileTime::default());
        if unsafe { GetProcessTimes(GetCurrentProcess(), &mut c, &mut e, &mut k, &mut u) } == 0 {
            return now;
        }
        let created = (u64::from(c.hi) << 32) | u64::from(c.lo);
        // SystemTime::now is GetSystemTimePreciseAsFileTime: the same clock as the creation time
        let unix = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default();
        let now_ft = unix.as_nanos() as u64 / 100 + 116_444_736_000_000_000;
        let age = std::time::Duration::from_nanos(now_ft.saturating_sub(created) * 100);
        now.checked_sub(age).unwrap_or(now)
    })
}

/// A start-up moment, in ms since the process was created.
pub(crate) fn mark(what: &str) {
    if on() {
        mark_at(what, Instant::now());
    }
}

fn mark_at(what: &str, at: Instant) {
    log(&format!("bench mark {what}: {:.1} ms", ms(born(), at)));
}

fn ms(a: Instant, b: Instant) -> f64 {
    b.saturating_duration_since(a).as_secs_f64() * 1000.0
}

/// One frame: when its Rendering callback began and ended, its last layout, when it was rendered.
#[derive(Clone, Copy)]
struct Tick {
    start: Instant,
    app_end: Option<Instant>,
    layout: Option<Instant>,
    rendered: Option<Instant>,
}

impl Tick {
    fn app(&self) -> f64 {
        self.app_end.map_or(0.0, |e| ms(self.start, e))
    }
    fn layout(&self) -> f64 {
        match (self.app_end, self.layout) {
            (Some(a), Some(l)) if l > a => ms(a, l),
            _ => 0.0,
        }
    }
    fn render(&self) -> f64 {
        let from = self.layout.filter(|l| Some(*l) > self.app_end).or(self.app_end).unwrap_or(self.start);
        self.rendered.map_or(0.0, |r| ms(from, r))
    }
    fn end(&self) -> Instant {
        self.rendered.or(self.layout).or(self.app_end).unwrap_or(self.start)
    }
    /// The frame's work on the UI thread, callback to rendered.
    fn busy(&self) -> f64 {
        ms(self.start, self.end())
    }
}

/// A dev step being measured, from its start to quiet frames.
struct Action {
    name: String,
    t0: Instant,
    handler: Option<f64>,
    ticks: Vec<Tick>,
    /// Frames in a row that were quick, with no long gap before them.
    quiet: u32,
    loads: (u32, f64, usize),
    /// Notes logged while it ran (the search box shown, results drawn, ...).
    notes: Vec<String>,
}

#[derive(Default)]
struct Frames {
    root: Option<FrameworkElement>,
    cur: Option<Tick>,
    last: Option<Tick>,
    n: u64,
    rendered_n: u64,
    first_layout: bool,
    action: Option<Action>,
    /// The last step begun, and when: later notes are timed from it.
    last_action: Option<(String, Instant)>,
    /// Start-up: frames in a row that were quick, and whether "ready" has been marked.
    quiet: u32,
    quiet_from: Option<Instant>,
    ready: bool,
    /// A scan was shown and its own "ready" is still owed.
    scan_ready_owed: bool,
    scan_shown_owed: bool,
    search_shown: bool,
    search_owed: bool,
    /// load() calls before the first frame (start-up's XAML).
    startup_loads: (u32, f64, usize),
}

thread_local! {
    static F: RefCell<Frames> = RefCell::new(Frames::default());
}

/// Frames this quick (callback to rendered) count as quiet.
const QUIET_MS: f64 = 17.0;
/// This many quiet frames in a row end a measurement.
const QUIET_RUN: u32 = 8;
/// A measurement ends after this long regardless.
const MAX_MS: f64 = 4000.0;

/// The page is made: layout and Loaded are watched on it from now on.
pub(crate) fn watch_root(root: &FrameworkElement) {
    if !on() {
        return;
    }
    F.with(|f| f.borrow_mut().root = Some(root.clone()));
    let _ = root.LayoutUpdated(|_, _| layout()).map(crate::state::keep);
    let _ = root.Loaded(|_, _| mark("root Loaded")).map(crate::state::keep);
    if let Ok(b) = crate::state::find::<FrameworkElement>(root, "SearchBox") {
        let _ = b.Loaded(|_, _| mark("SearchBox Loaded")).map(crate::state::keep);
    }
    let _ = crate::bindings::Microsoft::UI::Xaml::Media::CompositionTarget::Rendered(|_, _| rendered()).map(crate::state::keep);
}

/// The Rendering callback began.
pub(crate) fn frame_begin() {
    if !on() {
        return;
    }
    let now = Instant::now();
    F.with(|f| {
        let mut f = f.borrow_mut();
        // a frame that was never reported rendered ends here
        if let Some(t) = f.cur.take() {
            close(&mut f, t);
        }
        f.n += 1;
        if f.n == 1 {
            drop(f);
            mark("first Rendering callback");
            F.with(|f| f.borrow_mut().cur = Some(Tick { start: now, app_end: None, layout: None, rendered: None }));
            return;
        }
        f.cur = Some(Tick { start: now, app_end: None, layout: None, rendered: None });
    });
}

/// The Rendering callback ended: `search`: the Search tab is the one showing.
pub(crate) fn frame_end(search: bool) {
    if !on() {
        return;
    }
    let now = Instant::now();
    F.with(|f| {
        let mut f = f.borrow_mut();
        if let Some(t) = &mut f.cur {
            t.app_end = Some(now);
        }
        if search && !f.search_shown {
            f.search_owed = true;
        }
        if !search {
            f.search_shown = false;
            f.search_owed = false;
        }
    });
}

fn layout() {
    let now = Instant::now();
    let first = F.with(|f| {
        let mut f = f.borrow_mut();
        if let Some(t) = &mut f.cur {
            t.layout = Some(now);
        }
        !std::mem::replace(&mut f.first_layout, true)
    });
    if first {
        mark("first layout");
    }
}

fn rendered() {
    let now = Instant::now();
    let (first, search_box) = F.with(|f| {
        let mut f = f.borrow_mut();
        f.rendered_n += 1;
        let search_box = f.search_owed && f.root.as_ref().is_some_and(search_box_shows);
        if search_box {
            f.search_owed = false;
            f.search_shown = true;
        }
        if let Some(mut t) = f.cur.take() {
            t.rendered = Some(now);
            close(&mut f, t);
        }
        (f.rendered_n == 1, search_box)
    });
    if first {
        mark("first frame rendered");
    }
    // at start (no step yet): a start-up mark; after a step, its note
    if search_box && F.with(|f| f.borrow().last_action.is_none()) {
        mark("search box shown");
    }
    if search_box {
        note("search box shown");
    }
}

/// The search box is laid out with a size, in a bar that is showing.
fn search_box_shows(root: &FrameworkElement) -> bool {
    use crate::bindings::Microsoft::UI::Xaml::{UIElement, Visibility};
    let bar = crate::state::find::<UIElement>(root, "SearchBar").ok().and_then(|b| b.Visibility().ok()) == Some(Visibility::Visible);
    let w = crate::state::find::<FrameworkElement>(root, "SearchBox").ok().and_then(|b| b.ActualWidth().ok()).unwrap_or(0.0);
    bar && w > 0.0
}

/// A frame is over: start-up's "ready", the action it belongs to.
fn close(f: &mut Frames, t: Tick) {
    let gap = f.last.map_or(0.0, |l| ms(l.end(), t.start));
    let quick = t.busy() < QUIET_MS && gap < 2.0 * QUIET_MS;
    if f.scan_shown_owed && t.rendered.is_some() {
        f.scan_shown_owed = false;
        mark_at("scan shown", t.end());
    }
    if quick {
        if f.quiet == 0 {
            f.quiet_from = Some(t.start);
        }
        f.quiet += 1;
    } else {
        f.quiet = 0;
    }
    if f.quiet >= QUIET_RUN && f.rendered_n > 0 {
        if let Some(from) = f.quiet_from {
            if !f.ready {
                f.ready = true;
                mark_at("ready for input", from);
                let (n, m, b) = f.startup_loads;
                log(&format!("bench startup load() {n} calls {m:.1} ms {} KB", b / 1024));
            }
            if f.scan_ready_owed {
                f.scan_ready_owed = false;
                mark_at("ready after scan", from);
            }
        }
    }
    if let Some(a) = &mut f.action {
        a.ticks.push(t);
        a.quiet = if quick { a.quiet + 1 } else { 0 };
        if a.quiet >= QUIET_RUN || ms(a.t0, t.end()) > MAX_MS {
            let a = f.action.take().unwrap();
            report(a);
        }
    }
    f.last = Some(t);
}


/// Before `set_tree`: the next frame rendered is the scan shown, and the quiet after it ready.
pub(crate) fn scan_shown() {
    if !on() {
        return;
    }
    F.with(|f| {
        let mut f = f.borrow_mut();
        f.scan_shown_owed = true;
        f.scan_ready_owed = true;
        f.quiet = 0;
    });
}

/// A dev step is still being measured.
pub(crate) fn measuring() -> bool {
    on() && F.with(|f| f.borrow().action.is_some())
}

/// A dev step starts.
pub(crate) fn action_begin(name: &str) {
    if !on() {
        return;
    }
    F.with(|f| {
        let mut f = f.borrow_mut();
        if let Some(a) = f.action.take() {
            report(a);
        }
        f.last_action = Some((name.to_string(), Instant::now()));
        f.action = Some(Action { name: name.to_string(), t0: Instant::now(), handler: None, ticks: Vec::new(), quiet: 0, loads: (0, 0.0, 0), notes: Vec::new() });
    });
}

/// The step's own code returned.
pub(crate) fn action_handled() {
    if !on() {
        return;
    }
    let now = Instant::now();
    F.with(|f| {
        if let Some(a) = &mut f.borrow_mut().action {
            a.handler = Some(ms(a.t0, now));
        }
    });
}

/// Something worth its own line, with the time since the step that is being measured (or
/// since the process started).
pub(crate) fn note(what: &str) {
    if !on() {
        return;
    }
    let now = Instant::now();
    let line = F.with(|f| {
        let mut f = f.borrow_mut();
        if let Some(a) = &mut f.action {
            a.notes.push(format!("{what} at +{:.1}", ms(a.t0, now)));
        }
        // after the step that came last, measured or not any more
        match &f.last_action {
            Some((name, t0)) => format!("bench note {what} (+{:.1} ms after {name}, at {:.1} ms)", ms(*t0, now), ms(born(), now)),
            None => format!("bench note {what} (at {:.1} ms)", ms(born(), now)),
        }
    });
    log(&line);
}

/// A `load()` took `took` ms for `bytes` of XAML (`name`: its first x:Name).
pub(crate) fn loaded(took: f64, bytes: usize, name: &str) {
    F.with(|f| {
        let mut f = f.borrow_mut();
        if f.n == 0 || !f.ready {
            let l = &mut f.startup_loads;
            *l = (l.0 + 1, l.1 + took, l.2 + bytes);
        }
        if let Some(a) = &mut f.action {
            a.loads = (a.loads.0 + 1, a.loads.1 + took, a.loads.2 + bytes);
        }
    });
    if took >= 2.0 {
        log(&format!("bench load {name} ({} KB): {took:.1} ms", bytes / 1024));
    }
}

fn report(a: Action) {
    let first = a.ticks.first();
    let to_frame = first.map_or(0.0, |t| ms(a.t0, t.end()));
    // the last frame that was not quick: the result is fully drawn by its end
    let slow: Vec<&Tick> = a.ticks.iter().filter(|t| t.busy() >= QUIET_MS).collect();
    let settled = slow.last().map_or(to_frame, |t| ms(a.t0, t.end())).max(to_frame);
    let worst = a.ticks.iter().map(|t| t.busy()).fold(0.0, f64::max);
    let mut gaps = 0.0f64;
    for w in a.ticks.windows(2) {
        gaps = gaps.max(ms(w[0].end(), w[1].start));
    }
    let (n, m, b) = a.loads;
    let f0 = first.copied();
    log(&format!(
        "bench act {}: handler {:.1} | first frame {:.1} (app {:.1}, layout {:.1}, render {:.1}) | settled {:.1} | slow frames {} | worst frame {:.1} | worst gap {:.1} | load() {} calls {:.1} ms {} KB | frames {}{}",
        a.name,
        a.handler.unwrap_or(0.0),
        to_frame,
        f0.map_or(0.0, |t| t.app()),
        f0.map_or(0.0, |t| t.layout()),
        f0.map_or(0.0, |t| t.render()),
        settled,
        slow.len(),
        worst,
        gaps,
        n,
        m,
        b / 1024,
        a.ticks.len(),
        if a.notes.is_empty() { String::new() } else { format!(" | {}", a.notes.join("; ")) },
    ));
    for (i, t) in a.ticks.iter().enumerate().filter(|(_, t)| t.busy() >= QUIET_MS) {
        log(&format!("bench frame {} #{i} at +{:.1}: app {:.1}, layout {:.1}, render {:.1}", a.name, ms(a.t0, t.start), t.app(), t.layout(), t.render()));
    }
}

/// A frame's own work in parts: logged when the whole took a frame or more (`bench split`).
pub(crate) struct Split {
    t0: Instant,
    last: Instant,
    parts: Vec<(&'static str, f64)>,
}

impl Split {
    pub(crate) fn new() -> Option<Split> {
        on().then(|| {
            let now = Instant::now();
            Split { t0: now, last: now, parts: Vec::new() }
        })
    }

    pub(crate) fn at(this: &mut Option<Split>, what: &'static str) {
        if let Some(s) = this {
            let now = Instant::now();
            s.parts.push((what, ms(s.last, now)));
            s.last = now;
        }
    }
}

impl Drop for Split {
    fn drop(&mut self) {
        let total = ms(self.t0, Instant::now());
        if total >= 12.0 {
            let parts: Vec<String> = self.parts.iter().filter(|p| p.1 >= 0.5).map(|(w, m)| format!("{w} {m:.1}")).collect();
            log(&format!("bench split frame {total:.1} ms: {}", parts.join(", ")));
        }
    }
}

/// A part of the frame's work that took 4 ms or more (`bench slow`).
pub(crate) fn slow(what: &str, since: Instant) {
    if on() {
        let took = ms(since, Instant::now());
        if took >= 4.0 {
            log(&format!("bench slow {what}: {took:.1} ms"));
        }
    }
}

thread_local! {
    /// dev: the `untiladvisor` step holds the next steps until the Advisor's check is done
    /// (at most a minute).
    static HOLD: std::cell::Cell<Option<Instant>> = const { std::cell::Cell::new(None) };
}

pub(crate) fn hold_for_advisor() {
    HOLD.with(|h| h.set(Some(Instant::now())));
}

/// Whether the steps wait for the Advisor still (`running`: its check is not done).
pub(crate) fn holding(running: bool) -> bool {
    HOLD.with(|h| match h.get() {
        Some(t) if running && t.elapsed().as_secs() < 60 => true,
        Some(t) => {
            h.set(None);
            log(&format!("bench note advisor wait over after {:.1} ms{}", ms(t, Instant::now()), if running { " (gave up)" } else { "" }));
            false
        }
        None => false,
    })
}

/// A line of its own in the timing log.
pub(crate) fn line(text: &str) {
    if on() {
        log(text);
    }
}
