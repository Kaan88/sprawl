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
}

pub(crate) fn xaml() -> &'static str {
    r##"
    <Border x:Name="HistPanel" HorizontalAlignment="Left" VerticalAlignment="Bottom" Margin="14" Width="440" CornerRadius="10" Padding="16,12,16,12" BorderBrush="#26FFFFFF" BorderThickness="1" Visibility="Collapsed">
      <Border.Background><AcrylicBrush TintColor="#FF141620" TintOpacity="0.55" TintLuminosityOpacity="0.7" FallbackColor="#E8161822"/></Border.Background>
      <StackPanel Spacing="4">
        <Grid>
          <StackPanel Orientation="Horizontal" Spacing="8"><FontIcon Glyph="&#xE81C;" FontSize="14" Foreground="#FF8FB4FF"/><TextBlock Text="History" FontSize="13" FontWeight="SemiBold" VerticalAlignment="Center"/></StackPanel>
          <Button x:Name="HistClose" HorizontalAlignment="Right" Padding="6" Background="Transparent" BorderThickness="0"><FontIcon Glyph="&#xE711;" FontSize="11"/></Button>
        </Grid>
        <TextBlock x:Name="HistWhen" Text="Now" FontSize="18" FontWeight="SemiBold"/>
        <TextBlock x:Name="HistInfo" Text=" " FontSize="12" Foreground="#FFB9C2D6" TextWrapping="Wrap"/>
        <Grid ColumnSpacing="10" Margin="0,4,0,0">
          <Grid.ColumnDefinitions><ColumnDefinition Width="Auto"/><ColumnDefinition Width="*"/></Grid.ColumnDefinitions>
          <Button x:Name="HistPlay" Padding="10,6" CornerRadius="6" VerticalAlignment="Center" ToolTipService.ToolTip="Play through the history"><FontIcon x:Name="HistPlayIcon" Glyph="&#xE768;" FontSize="13"/></Button>
          <Slider x:Name="HistSlider" Grid.Column="1" Minimum="0" Maximum="1" Value="1" StepFrequency="1" TickFrequency="1" TickPlacement="Outside" SnapsTo="Ticks" VerticalAlignment="Center"/>
        </Grid>
        <TextBlock x:Name="HistGrew" Text=" " FontSize="12" Foreground="#FFE8EDF7" TextWrapping="Wrap"/>
        <HyperlinkButton x:Name="HistGrowth" Content="What grew, and when the drive fills: Advisor &gt; Growth" FontSize="12" Padding="0"/>
        <StackPanel Orientation="Horizontal" Spacing="14" Margin="0,4,0,0">
          <StackPanel Orientation="Horizontal" Spacing="5"><Border Width="10" Height="10" CornerRadius="2" Background="#FFFF5A45"/><TextBlock Text="grew" FontSize="11" Foreground="#FF9AA3B8"/></StackPanel>
          <StackPanel Orientation="Horizontal" Spacing="5"><Border Width="10" Height="10" CornerRadius="2" Background="#FF4A8CFF"/><TextBlock Text="shrank" FontSize="11" Foreground="#FF9AA3B8"/></StackPanel>
          <StackPanel Orientation="Horizontal" Spacing="5"><Border Width="10" Height="10" CornerRadius="2" Background="#FF2AD9FF"/><TextBlock Text="new" FontSize="11" Foreground="#FF9AA3B8"/></StackPanel>
          <StackPanel Orientation="Horizontal" Spacing="5"><Border Width="10" Height="10" CornerRadius="2" Background="#FF3A3F4C"/><TextBlock Text="unchanged" FontSize="11" Foreground="#FF9AA3B8"/></StackPanel>
        </StackPanel>
      </StackPanel>
    </Border>"##
}

/// "+1.2 GB" / "−300 MB"
fn signed(v: i128) -> String {
    let s = sprawl_core::format_size(v.unsigned_abs() as u64);
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
        if self.shot.is_some() || self.settings.bool("snapshots") == Some(false) {
            return;
        }
        let Some(t) = &self.tree else { return };
        if t.method == sprawl_core::ScanMethod::Csv || t.nodes[ROOT].name == sprawl_core::MULTIPLE || self.full_tree.is_some() {
            return;
        }
        let Some(base) = history::base_dir() else { return };
        let cap = (self.settings.num("snapshot_cap_mb").unwrap_or(1024.0) * 1024.0 * 1024.0) as u64;
        let pending = history::prepare(t, self.scan_taken, true);
        std::thread::spawn(move || {
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
            let off = self.settings.bool("snapshots") == Some(false);
            text(
                &self.root,
                "FootLeft",
                if off {
                    "Scan history is off (Options > Setup Wizard > History)."
                } else {
                    "No earlier scans of this folder yet. Each scan is saved, so the next one can be compared with this one."
                },
            );
            return;
        }
        let n = entries.len();
        self.hist = Some(Hist { entries, pos: n, cache: HashMap::new(), display: None, base: None, playing: false, next: Instant::now(), gen: self.tree_gen });
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

    /// Sizes of every current node at entry `i` (or now), cached.
    fn hist_weights(&mut self, i: usize) -> Option<Rc<Vec<u64>>> {
        let t = self.tree.as_ref()?;
        let h = self.hist.as_mut()?;
        if i >= h.entries.len() {
            return None;
        }
        if let Some(w) = h.cache.get(&i) {
            return Some(w.clone());
        }
        let old = match history::load(&h.entries[i]) {
            Ok(o) => o,
            Err(e) => {
                eprintln!("history: {e}");
                return Some(Rc::new(vec![0; t.nodes.len()]));
            }
        };
        let w = Rc::new(history::project(t, &old));
        if h.cache.len() >= 6 {
            // keep the neighbours of where we are
            let far = *h.cache.keys().max_by_key(|&&k| k.abs_diff(i)).unwrap();
            h.cache.remove(&far);
        }
        h.cache.insert(i, w.clone());
        Some(w)
    }

    pub(crate) fn hist_seek(&mut self, pos: usize) {
        let Some(h) = &self.hist else { return };
        if self.tree.is_none() || self.tree_gen != h.gen {
            self.hist_close();
            return;
        }
        let n = h.entries.len();
        let pos = pos.min(n);
        let display = self.hist_weights(pos);
        let base = if pos > 0 { self.hist_weights(pos - 1) } else { None };
        let h = self.hist.as_mut().unwrap();
        h.pos = pos;
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

    fn hist_set_slider(&mut self, pos: usize) {
        if let Ok(s) = find::<RangeBase>(&self.root, "HistSlider") {
            let _ = s.SetValue(pos as f64); // ValueChanged seeks
        }
    }

    /// Called each frame: advance the time-lapse.
    pub(crate) fn hist_tick(&mut self) {
        let Some(h) = &mut self.hist else { return };
        if !h.playing || Instant::now() < h.next {
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
        return if shrank > 0 { format!("Shrank overall by {}", sprawl_core::format_size(shrank as u64)) } else { "No change in size".into() };
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
