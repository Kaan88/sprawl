//! Scan history on the map: saving a snapshot after each scan, and the History panel whose
//! slider steps through earlier snapshots. Tiles take the size they had then and are coloured
//! by what changed since the snapshot before (red grew, blue shrank, cyan new). At "Now" the
//! colours show what changed since the previous scan. Play steps through them all.

use std::collections::HashMap;
use std::rc::Rc;
use std::time::{Duration, Instant, SystemTime};

use sprawl_advisor::history::{self, Entry};
use sprawl_core::{local_time, Tree, ROOT};

use crate::bindings::Microsoft::UI::Xaml::Controls::Primitives::RangeBase;
use crate::bindings::Microsoft::UI::Xaml::{UIElement, Visibility};
use crate::state::{find, text, State};

const STEP: Duration = Duration::from_millis(1400);

pub(crate) struct Hist {
    pub entries: Vec<Entry>,
    /// 0..entries.len(); entries.len() is "now".
    pub pos: usize,
    cache: HashMap<usize, Rc<Vec<u64>>>,
    /// Sizes shown (None: as scanned) and the baseline the colours compare against.
    pub display: Option<Rc<Vec<u64>>>,
    pub base: Option<Rc<Vec<u64>>>,
    pub playing: bool,
    next: Instant,
    /// `State::tree_gen` it was opened for.
    gen: u64,
    /// Snapshots being read and laid over the tree on a worker: (index, sizes) as each is done.
    /// Reading one is seconds on a big drive, which froze the window on every slider step.
    job: Option<std::sync::mpsc::Receiver<(usize, Vec<u64>)>>,
    /// The position on screen (`pos` may be ahead of it while its snapshot is read).
    shown: Option<usize>,
}

/// How many snapshots' sizes are kept (each is 8 bytes a node: about 25 MB for a full C:).
const KEEP: usize = 6;

pub(crate) fn xaml() -> &'static str {
    r##"
    <Border x:Name="HistPanel" HorizontalAlignment="Left" VerticalAlignment="Bottom" Margin="14" Width="440" CornerRadius="10" Padding="16,12,16,12" BorderBrush="#26FFFFFF" BorderThickness="1" Visibility="Collapsed">
      <Border.Background><AcrylicBrush TintColor="#FF141620" TintOpacity="0.55" TintLuminosityOpacity="0.7" FallbackColor="#E8161822"/></Border.Background>
      <StackPanel Spacing="4">
        <Grid>
          <StackPanel Orientation="Horizontal" Spacing="8"><FontIcon Glyph="&#xE81C;" FontSize="14" Foreground="#FF8FB4FF"/><TextBlock Text="History" FontSize="13" FontWeight="SemiBold" VerticalAlignment="Center"/></StackPanel>
          <Button x:Name="HistClose" AutomationProperties.Name="Close History" HorizontalAlignment="Right" Padding="6" Background="Transparent" BorderThickness="0"><FontIcon Glyph="&#xE711;" FontSize="12"/></Button>
        </Grid>
        <TextBlock x:Name="HistWhen" Text="Now" FontSize="18" FontWeight="SemiBold"/>
        <TextBlock x:Name="HistInfo" Text=" " FontSize="12" Foreground="#FFB9C2D6" TextWrapping="Wrap"/>
        <Grid ColumnSpacing="10" Margin="0,4,0,0">
          <Grid.ColumnDefinitions><ColumnDefinition Width="Auto"/><ColumnDefinition Width="*"/></Grid.ColumnDefinitions>
          <Button x:Name="HistPlay" AutomationProperties.Name="Play the history" VerticalAlignment="Center" ToolTipService.ToolTip="Play through the history"><FontIcon x:Name="HistPlayIcon" Glyph="&#xE768;" FontSize="13"/></Button>
          <Slider x:Name="HistSlider" Grid.Column="1" Minimum="0" Maximum="1" Value="1" StepFrequency="1" TickFrequency="1" TickPlacement="Outside" SnapsTo="Ticks" VerticalAlignment="Center"/>
        </Grid>
        <TextBlock x:Name="HistGrew" Text=" " FontSize="12" Foreground="#FFE8EDF7" TextWrapping="Wrap"/>
        <HyperlinkButton x:Name="HistGrowth" Content="What grew, and when the drive fills: Advisor &gt; Growth" FontSize="12" Padding="0"/>
        <StackPanel Orientation="Horizontal" Spacing="14" Margin="0,4,0,0">
          <StackPanel Orientation="Horizontal" Spacing="5"><Border Width="10" Height="10" CornerRadius="2" Background="#FFFF5A45"/><TextBlock Text="grew" FontSize="12" Foreground="#FF9AA3B8"/></StackPanel>
          <StackPanel Orientation="Horizontal" Spacing="5"><Border Width="10" Height="10" CornerRadius="2" Background="#FF4A8CFF"/><TextBlock Text="shrank" FontSize="12" Foreground="#FF9AA3B8"/></StackPanel>
          <StackPanel Orientation="Horizontal" Spacing="5"><Border Width="10" Height="10" CornerRadius="2" Background="#FF2AD9FF"/><TextBlock Text="new" FontSize="12" Foreground="#FF9AA3B8"/></StackPanel>
          <StackPanel Orientation="Horizontal" Spacing="5"><Border Width="10" Height="10" CornerRadius="2" Background="#FF3A3F4C"/><TextBlock Text="unchanged" FontSize="12" Foreground="#FF9AA3B8"/></StackPanel>
        </StackPanel>
      </StackPanel>
    </Border>"##
}

/// "+1.2 GB" / "−300 MB"
fn signed(v: i128) -> String {
    let s = crate::format::size(v.unsigned_abs() as u64);
    if v >= 0 {
        format!("+{s}")
    } else {
        format!("\u{2212}{s}")
    }
}

fn ago(t: SystemTime) -> String {
    let s = SystemTime::now().duration_since(t).map(|d| d.as_secs()).unwrap_or(0);
    match s {
        0..=89 => "just now".into(),
        90..=5399 => format!("{} minutes ago", (s + 30) / 60),
        5400..=129_599 => format!("{} hours ago", (s + 1800) / 3600),
        _ => format!("{} days ago", (s + 43_200) / 86_400),
    }
}

/// Colour for a tile that went from `then` to `now` bytes.
pub(crate) fn change_colour(then: u64, now: u64) -> [f32; 3] {
    if then == 0 && now > 0 {
        return [0.12, 0.80, 1.0];
    }
    if now == 0 && then > 0 {
        return [0.25, 0.35, 0.75];
    }
    let g = ((now.max(1) as f64) / (then.max(1) as f64)).ln();
    let t = (g.abs() / 4f64.ln()).min(1.0).sqrt() as f32;
    if g > 0.005 {
        [0.42 + 0.58 * t, 0.18 + 0.12 * t, 0.16 + 0.06 * t]
    } else if g < -0.005 {
        [0.14 + 0.06 * t, 0.24 + 0.24 * t, 0.48 + 0.52 * t]
    } else {
        [0.21, 0.23, 0.28]
    }
}

impl State {
    /// After a scan: encode its snapshots on this thread (fast) and write them on another.
    pub(crate) fn save_snapshot(&mut self) {
        if self.shot.is_some() || self.settings.bool("snapshots") != Some(true) {
            return;
        }
        let Some(t) = &self.tree else { return };
        if t.method == sprawl_core::ScanMethod::Csv || t.nodes[ROOT].name == sprawl_core::MULTIPLE || self.full_tree.is_some() {
            return;
        }
        let Some(base) = history::base_dir() else { return };
        let cap = (self.settings.num("snapshot_cap_mb").unwrap_or(1024.0) * 1024.0 * 1024.0) as u64;
        // encoded on the worker too: a big scan takes seconds to encode
        let (t, taken) = (t.clone(), self.scan_taken);
        std::thread::spawn(move || {
            let pending = history::prepare(&t, taken, true);
            drop(t);
            if let Err(e) = pending.save(&base, cap) {
                eprintln!("snapshot: {e}");
            }
        });
    }

    pub(crate) fn hist_toggle(&mut self) {
        if self.hist.is_some() {
            self.hist_close();
            return;
        }
        // the history plays on the treemap
        if self.map.views.kind.is_some() {
            self.set_view_kind(None);
            self.command("ModeMap");
        }
        let Some(t) = &self.tree else {
            text(&self.root, "FootLeft", "Scan something first: the history shows how it changed between scans.");
            return;
        };
        let Some(base) = history::base_dir() else { return };
        let me = self.scan_taken - Duration::from_secs(2);
        let entries: Vec<Entry> = history::covering(&base, &t.path_of(ROOT)).into_iter().filter(|e| e.taken < me).collect();
        if entries.is_empty() {
            let off = self.settings.bool("snapshots") != Some(true);
            text(
                &self.root,
                "FootLeft",
                if off {
                    "Scan history is off (Options > Settings > History)."
                } else {
                    "No earlier scans of this folder yet. Each scan is saved, so the next one can be compared with this one."
                },
            );
            return;
        }
        let n = entries.len();
        self.hist = Some(Hist { entries, pos: n, cache: HashMap::new(), display: None, base: None, playing: false, next: Instant::now(), gen: self.tree_gen, job: None, shown: None });
        if let Ok(s) = find::<RangeBase>(&self.root, "HistSlider") {
            let _ = s.SetMaximum(n as f64);
            let _ = s.SetValue(n as f64);
        }
        if let Ok(p) = find::<UIElement>(&self.root, "HistPanel") {
            let _ = p.SetVisibility(Visibility::Visible);
        }
        self.hist_seek(n);
    }

    pub(crate) fn hist_close(&mut self) {
        if self.hist.take().is_some() {
            if let Ok(p) = find::<UIElement>(&self.root, "HistPanel") {
                let _ = p.SetVisibility(Visibility::Collapsed);
            }
            self.render_map();
        }
    }

    /// The snapshots position `pos` needs (its own, and the one before for the colours), and
    /// while playing the next one too, so the step after is ready when it is due.
    fn hist_needs(h: &Hist, pos: usize) -> Vec<usize> {
        let n = h.entries.len();
        let mut v = Vec::new();
        if pos < n {
            v.push(pos);
        }
        if pos > 0 && pos - 1 < n {
            v.push(pos - 1);
        }
        if h.playing && pos + 1 < n {
            v.push(pos + 1);
        }
        v
    }

    /// Start reading what `pos` needs and the cache lacks, unless a read is running (its end
    /// starts the next for wherever the slider is by then: a drag reads only where it stops).
    fn hist_fetch(&mut self) {
        let Some(t) = self.tree.clone() else { return };
        let Some(h) = self.hist.as_mut() else { return };
        if h.job.is_some() {
            return;
        }
        let want: Vec<(usize, Entry)> = Self::hist_needs(h, h.pos).into_iter().filter(|i| !h.cache.contains_key(i)).map(|i| (i, h.entries[i].clone())).collect();
        if want.is_empty() {
            return;
        }
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            for (i, e) in want {
                let w = match history::load(&e) {
                    Ok(old) => history::project(&t, &old),
                    Err(err) => {
                        eprintln!("history: {err}");
                        vec![0; t.nodes.len()]
                    }
                };
                if tx.send((i, w)).is_err() {
                    return;
                }
            }
        });
        h.job = Some(rx);
    }

    /// Each frame: snapshots read on the worker go into the cache, and the position wanted is
    /// shown as soon as what it needs is there.
    fn hist_poll(&mut self) {
        let Some(h) = self.hist.as_mut() else { return };
        if let Some(rx) = &h.job {
            loop {
                match rx.try_recv() {
                    Ok((i, w)) => {
                        h.cache.insert(i, Rc::new(w));
                        // keep the neighbours of where we are (what it needs is within one step)
                        let pos = h.pos;
                        while h.cache.len() > KEEP {
                            let Some(&far) = h.cache.keys().max_by_key(|&&k| k.abs_diff(pos)) else { break };
                            h.cache.remove(&far);
                        }
                    }
                    Err(std::sync::mpsc::TryRecvError::Empty) => break,
                    Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                        h.job = None;
                        break;
                    }
                }
            }
        }
        let pos = h.pos;
        if h.shown != Some(pos) && Self::hist_needs(h, pos).iter().take(2).all(|i| h.cache.contains_key(i)) {
            self.hist_show(pos);
        }
        self.hist_fetch();
    }

    /// Go to position `pos` (0..=entries, the last is now). Shown at once when its snapshots
    /// are read already; otherwise the map stays on the last one shown until they are.
    pub(crate) fn hist_seek(&mut self, pos: usize) {
        let Some(h) = &mut self.hist else { return };
        if self.tree.is_none() || self.tree_gen != h.gen {
            self.hist_close();
            return;
        }
        let n = h.entries.len();
        let pos = pos.min(n);
        h.pos = pos;
        let ready = Self::hist_needs(h, pos).iter().take(2).all(|i| h.cache.contains_key(i));
        if ready {
            self.hist_show(pos);
        } else {
            let when = format!("{} ({})", ago(h.entries[pos.min(n - 1)].taken), local_time(h.entries[pos.min(n - 1)].taken));
            text(&self.root, "HistWhen", &when);
            text(&self.root, "HistInfo", "Reading that scan from History...");
            if let Ok(s) = find::<RangeBase>(&self.root, "HistSlider") {
                if s.Value().map(|v| v.round() as usize) != Ok(pos) {
                    let _ = s.SetValue(pos as f64);
                }
            }
        }
        self.hist_fetch();
    }

    /// Show position `pos`, whose snapshots are in the cache.
    fn hist_show(&mut self, pos: usize) {
        let Some(h) = &mut self.hist else { return };
        let n = h.entries.len();
        let display = if pos < n { h.cache.get(&pos).cloned() } else { None };
        let base = if pos > 0 { h.cache.get(&(pos - 1)).cloned() } else { None };
        h.shown = Some(pos);
        crate::state::timing(&format!("History shows {pos} of {n}"), std::time::Instant::now());
        // the next step of a time-lapse counts from when this one is on screen
        h.next = Instant::now() + STEP;
        h.display = display.clone();
        h.base = base.clone();
        let when = if pos == n { "Now".to_string() } else { format!("{} ({})", ago(h.entries[pos].taken), local_time(h.entries[pos].taken)) };
        let prev = (pos > 0).then(|| h.entries[pos - 1].taken);
        text(&self.root, "HistWhen", &when);
        if let Ok(s) = find::<RangeBase>(&self.root, "HistSlider") {
            if s.Value().map(|v| v.round() as usize) != Ok(pos) {
                let _ = s.SetValue(pos as f64); // ValueChanged sees pos unchanged and does nothing
            }
        }

        let t = self.tree.as_ref().unwrap();
        let now_of = |k: usize| display.as_ref().map_or(t.nodes[k].size, |d| d[k]);
        let info = match (&base, prev) {
            (Some(b), Some(p)) => {
                let d = now_of(ROOT) as i128 - b[ROOT] as i128;
                format!("{} since the scan {} · {} snapshot{} in all", signed(d), ago(p), n, if n == 1 { "" } else { "s" })
            }
            _ => format!("The oldest snapshot. {} in all; move right or press play.", n),
        };
        text(&self.root, "HistInfo", &info);
        text(&self.root, "HistGrew", &base.as_ref().map(|b| grew_most(t, self.map_root, &now_of, b)).unwrap_or_default());
        self.render_map();
    }

    pub(crate) fn hist_play(&mut self) {
        let Some(h) = &mut self.hist else { return };
        h.playing = !h.playing;
        let restart = h.playing && h.pos >= h.entries.len();
        h.next = Instant::now() + STEP;
        let playing = h.playing;
        text(&self.root, "HistPlayIcon", "");
        if let Ok(i) = find::<crate::bindings::Microsoft::UI::Xaml::Controls::FontIcon>(&self.root, "HistPlayIcon") {
            let _ = i.SetGlyph(&(if playing { "\u{E769}" } else { "\u{E768}" }).into());
        }
        if restart {
            self.hist_set_slider(0);
        }
    }

    /// Go to `pos` from code (Play). Not through the slider's ValueChanged: from inside a
    /// handler or a frame that never arrives (the state is borrowed already), which is why Play
    /// moved one step and stopped.
    fn hist_set_slider(&mut self, pos: usize) {
        self.hist_seek(pos);
    }

    /// Called each frame: snapshots read meanwhile, and the time-lapse's next step once the
    /// one before has been on screen for a while.
    pub(crate) fn hist_tick(&mut self) {
        self.hist_poll();
        let Some(h) = &mut self.hist else { return };
        if !h.playing || h.shown != Some(h.pos) || Instant::now() < h.next {
            return;
        }
        h.next = Instant::now() + STEP;
        let next = h.pos + 1;
        let end = h.entries.len();
        if next >= end {
            h.playing = false;
            if let Ok(i) = find::<crate::bindings::Microsoft::UI::Xaml::Controls::FontIcon>(&self.root, "HistPlayIcon") {
                let _ = i.SetGlyph(&"\u{E768}".into());
            }
        }
        self.hist_set_slider(next.min(end));
    }

    /// Weight and colour overrides for the layout, when the panel is open.
    pub(crate) fn hist_view(&self) -> Option<(Option<Rc<Vec<u64>>>, Option<Rc<Vec<u64>>>)> {
        self.hist.as_ref().map(|h| (h.display.clone(), h.base.clone()))
    }
}

/// Follow the growth down from `root` while one folder holds most of it.
fn grew_most(t: &Tree, root: usize, now: &dyn Fn(usize) -> u64, base: &[u64]) -> String {
    let delta = |k: usize| now(k) as i128 - base[k] as i128;
    let total = delta(root);
    if total <= 0 {
        let shrank: i128 = -total;
        return if shrank > 0 { format!("Shrank overall by {}", crate::format::size(shrank as u64)) } else { "No change in size".into() };
    }
    let mut n = root;
    loop {
        let best = t.nodes[n].children.iter().copied().filter(|&c| t.nodes[c].is_dir).max_by_key(|&c| delta(c));
        match best {
            Some(c) if delta(c) * 10 >= delta(n) * 6 && delta(c) > 0 => n = c,
            _ => break,
        }
    }
    if n == root {
        return format!("Grew {} spread over many folders", signed(total));
    }
    let rel = t.path_of(n);
    let base_path = t.path_of(root);
    let shown = rel.strip_prefix(&base_path).map(|p| p.display().to_string()).unwrap_or_else(|_| rel.display().to_string());
    format!("Grew most: {} ({} of {})", shown, signed(delta(n)), signed(total))
}
