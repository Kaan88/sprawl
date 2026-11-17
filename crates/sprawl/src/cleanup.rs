//! Clean-up inside the Advisor: tick findings, review them, and Sprawl removes them itself,
//! showing each item's progress. The tree, the map and the findings update in place afterwards.

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Receiver};
use std::sync::Arc;
use std::time::Instant;

use sprawl_advisor::clean::{self, Action, Outcome};
use sprawl_advisor::cleaners::Closed;
use sprawl_advisor::{Finding, Remedy, Risk, Source};
use sprawl_core::Tree;

use crate::state::State;
use crate::ui;

/// The ticks and the clean-up's state, kept in the Advisor.
#[derive(Default)]
pub(crate) struct Cleanup {
    /// Ticked findings, by lower-case path.
    pub ticked: HashSet<String>,
    /// The report generation the default ticks were set for.
    pub ticked_for: Option<u64>,
    /// Ticks carried over a restart as administrator, for the first report.
    pub pending: Option<HashSet<String>>,
    /// The review panel is open.
    pub review: bool,
    /// "These are my own files and I want them gone" is ticked.
    pub own_ok: bool,
    /// The ticks are the ones Sprawl chose.
    pub auto: bool,
    /// The review is for this one finding (its row's Clean up...), not for the ticks.
    pub single: Option<String>,
    pub running: Option<Running>,
    /// The last clean-up's result, shown until dismissed.
    pub done: Option<Done>,
}

pub(crate) struct Item {
    pub path: PathBuf,
    pub title: String,
    pub bytes: u64,
    pub state: ItemState,
}

pub(crate) enum ItemState {
    Waiting,
    Running(u64),
    Done(Outcome),
}

pub(crate) struct Running {
    pub items: Vec<Item>,
    rx: Receiver<Msg>,
    pub stop: Arc<AtomicBool>,

    /// What each item's folder holds now: None when it is gone.
    rescans: Vec<(PathBuf, Option<Tree>)>,
    /// Ticked items that didn't pass the checks again, and why.
    held: Vec<String>,
}

pub(crate) struct Done {
    pub freed: u64,
    /// Items that freed something.
    pub items: usize,
    /// Left because they were new since the scan, or kept by the user.
    pub kept_new: u64,
    pub kept_chosen: u64,
    pub problems: Vec<String>,
    pub stopped: bool,
    pub output: Vec<String>,
}

enum Msg {
    Start(usize),
    Progress(usize, u64),
    /// (item, how it went, a scan of what is there now: None when it is gone, or not read)
    Finished(usize, Outcome, Option<Option<Tree>>),
    End,
}

/// One folder read again after it was cleaned. A plain walk: exact and quick for one folder,
/// where the volume's MFT (when elevated) would read the whole drive, maybe not yet seeing
/// what was just deleted.
pub(crate) fn rescan(p: &std::path::Path) -> std::io::Result<Tree> {
    sprawl_core::walk::walk(p, &sprawl_core::Progress::new())
}

pub(crate) fn key(p: &std::path::Path) -> String {
    p.to_string_lossy().to_lowercase()
}

/// Why a part whose application must be closed can't be cleaned now, or None when it can:
/// one of `exes` runs, a program has one of its files open, or Windows can't say.
fn still_open(path: &std::path::Path, app: &str, exes: &[String]) -> Option<String> {
    let now = sprawl_advisor::procs::running();
    match sprawl_advisor::procs::any_running(exes, now.as_ref()) {
        Some(false) => {}
        Some(true) => return Some(format!("{}: left alone, {app} is open", path.display())),
        None => return Some(format!("{}: left alone, Windows couldn't say whether {app} is open", path.display())),
    }
    let files = sprawl_advisor::inuse::files_on_disk(path, sprawl_advisor::inuse::MAX_FILES);
    if files.is_empty() {
        return None;
    }
    match sprawl_advisor::inuse::in_use(&files) {
        Some(names) if names.is_empty() => None,
        Some(names) => Some(format!("{}: left alone, in use by {}", path.display(), names.join(", "))),
        None => Some(format!("{}: left alone, Windows couldn't say which programs use it", path.display())),
    }
}

/// Why a finding can't be ticked, or None when it can.
pub(crate) fn untickable(f: &Finding, counted: bool) -> Option<&'static str> {
    if !matches!(f.remedy, Remedy::Delete | Remedy::Command) || f.risk == Risk::Red {
        return Some("");
    }
    let Some((action, admin)) = clean::action(f) else { return Some("") };
    if !counted {
        return Some("Inside another item");
    }
    if admin && !sprawl_core::is_elevated() {
        return Some("Needs administrator");
    }
    // a tool's own clean-up acts on the account running it: only the user's own folders
    if matches!(action, Action::Tool { .. }) {
        let mine = std::env::var_os("USERPROFILE").is_some_and(|home| PathBuf::from(key(&f.path)).starts_with(PathBuf::from(home.to_string_lossy().to_lowercase())));
        if !mine {
            return Some("Another account's");
        }
    }
    None
}

impl State {
    /// Ticks for a new report: the user's own ticks that still apply, else none. Sprawl never
    /// ticks a delete by itself; "Tick the safe ones" does it on request.
    pub(crate) fn cleanup_defaults(&mut self) {
        let Some(r) = self.advice() else { return };
        if self.advisor.cleanup.ticked_for == Some(r.tree) {
            return;
        }
        let gen = r.tree;
        if let Some(p) = self.advisor.cleanup.pending.as_ref() {
            let ticked: HashSet<String> = r.findings.iter().zip(&r.counted).filter(|(f, c)| untickable(f, **c).is_none()).map(|(f, _)| key(&f.path)).filter(|k| p.contains(k)).collect();
            let c = &mut self.advisor.cleanup;
            c.ticked = ticked;
            c.ticked_for = Some(gen);
            c.auto = false;
            c.pending = None;
            return;
        }
        // ticks the user chose stay, for findings that are still there and can be ticked
        if !self.advisor.cleanup.auto && self.advisor.cleanup.ticked_for.is_some() {
            let old = &self.advisor.cleanup.ticked;
            let kept: HashSet<String> = r.findings.iter().zip(&r.counted).filter(|(f, c)| untickable(f, **c).is_none()).map(|(f, _)| key(&f.path)).filter(|k| old.contains(k)).collect();
            let c = &mut self.advisor.cleanup;
            c.ticked = kept;
            c.ticked_for = Some(gen);
            c.review = false;
            c.single = None;
            return;
        }
        let c = &mut self.advisor.cleanup;
        c.ticked.clear();
        c.ticked_for = Some(gen);
        c.review = false;
        c.own_ok = false;
        c.single = None;
        c.auto = false;
    }

    /// What "Tick the safe ones" ticks: what comes back by itself, from Sprawl's own rules (not a
    /// guess by shape), inside the user's own profile, 1 MB or more.
    fn safe_ticks(&self) -> HashSet<String> {
        let Some(r) = self.advice() else { return HashSet::new() };
        r.findings
            .iter()
            .zip(&r.counted)
            .filter(|(f, c)| untickable(f, **c).is_none() && f.risk == Risk::Green && f.builtin && !f.heuristic() && f.in_profile() && !matches!(f.source, Source::Model(_)) && f.bytes >= 1 << 20)
            .map(|(f, _)| key(&f.path))
            .collect()
    }

    /// The ticked findings of the report on screen, in list order (index, finding).
    pub(crate) fn ticked_findings(&self) -> Vec<(usize, Finding)> {
        let Some(r) = self.advice() else { return Vec::new() };
        r.findings
            .iter()
            .enumerate()
            .filter(|(i, f)| {
                let k = key(&f.path);
                let chosen = match &self.advisor.cleanup.single {
                    Some(one) => *one == k,
                    None => self.advisor.cleanup.ticked.contains(&k),
                };
                chosen && untickable(f, r.counted[*i]).is_none()
            })
            .map(|(i, f)| (i, f.clone()))
            .collect()
    }

    pub(crate) fn cleanup_toggle(&mut self, i: usize) {
        let Some(f) = self.advice().and_then(|r| r.findings.get(i)).map(|f| key(&f.path)) else { return };
        let c = &mut self.advisor.cleanup;
        if !c.ticked.remove(&f) {
            c.ticked.insert(f);
        }
        // the ticks are what counts now, and a changed set is confirmed again
        c.single = None;
        c.own_ok = false;
        c.auto = false;
    }

    /// Tick all of `keys` (a cleaner's part), or untick them when they are all ticked already.
    pub(crate) fn cleanup_tick_many(&mut self, keys: Vec<String>) {
        if keys.is_empty() {
            return;
        }
        let c = &mut self.advisor.cleanup;
        if keys.iter().all(|k| c.ticked.contains(k)) {
            for k in &keys {
                c.ticked.remove(k);
            }
        } else {
            c.ticked.extend(keys);
        }
        c.single = None;
        c.own_ok = false;
        c.auto = false;
    }

    /// Tick what comes back by itself (on the user's request).
    pub(crate) fn cleanup_select_safe(&mut self) {
        self.cleanup_defaults();
        let safe = self.safe_ticks();
        let c = &mut self.advisor.cleanup;
        c.ticked = safe;
        c.single = None;
        c.own_ok = false;
        c.auto = true;
    }

    /// Tick enough for `need` bytes: the plan's choice (green first, then amber).
    pub(crate) fn cleanup_select_enough(&mut self, need: u64) -> Option<u64> {
        let r = self.advice()?;
        let plan = sprawl_advisor::plan::plan(&r.findings, &r.counted, need);
        let ticked: HashSet<String> = plan.picked.iter().filter(|&&i| untickable(&r.findings[i], r.counted[i]).is_none()).map(|&i| key(&r.findings[i].path)).collect();
        let bytes = r.findings.iter().filter(|f| ticked.contains(&key(&f.path))).map(|f| f.bytes).sum();
        self.advisor.cleanup.ticked = ticked;
        self.advisor.cleanup.auto = false;
        Some(bytes)
    }

    /// Start cleaning what is ticked: each item is checked again first.
    pub(crate) fn cleanup_start(&mut self) {
        if self.advisor.cleanup.running.is_some() {
            return;
        }
        // checked against the whole scan, not a filtered view of it
        let Some(t) = self.full_tree.clone().or_else(|| self.tree.clone()) else { return };
        let picked = self.ticked_findings();
        if picked.is_empty() {
            return;
        }
        let needs_ok = picked.iter().any(|(_, f)| f.risk >= Risk::Orange);
        if needs_ok && !self.advisor.cleanup.own_ok {
            self.adv_say("Tick \"These are my own files and I want them gone\" first, or untick your own files.");
            return;
        }
        let never = sprawl_advisor::safety::Never::default_list();
        // what the run leaves: anything new since the scan began, and what the user keeps
        let spare = clean::Spare { since: Some(self.scan_began), keep: self.advisor.memory.kept_paths() };
        self.cleaner_procs(true);
        self.remember_usual(&picked.iter().map(|(_, f)| f.clone()).collect::<Vec<_>>());
        let mut work: Vec<(PathBuf, Action, u64, Option<(String, Vec<String>)>)> = Vec::new();
        let mut items = Vec::new();
        let mut held = Vec::new();
        for (_, f) in picked {
            if spare.keeps(&f.path) {
                held.push(format!("{}: not cleaned, you chose to keep it", f.title));
                continue;
            }
            // the same checks as when it was found, against this scan
            let f = match sprawl_advisor::safety::check(&f, &t, &never) {
                Ok(f) => f,
                Err(why) => {
                    held.push(format!("{}: not cleaned, {why}", f.title));
                    continue;
                }
            };
            let Some((action, _)) = clean::action(&f) else {
                held.push(format!("{}: not cleaned, its command is not one Sprawl can run", f.title));
                continue;
            };
            // its application must be closed: checked now, and again just before it runs
            let gate = self.part_gate(&f);
            if let Some((app, Closed::Must)) = self.app_open(&f) {
                held.push(format!("{}: not cleaned, {app} is open. Close it, then clean up again.", f.title));
                continue;
            }
            work.push((f.path.clone(), action, f.bytes, gate));
            items.push(Item { path: f.path.clone(), title: f.title.clone(), bytes: f.bytes, state: ItemState::Waiting });
        }
        if items.is_empty() {
            self.adv_say("Nothing was cleaned: the items no longer pass the safety checks. Scan again.");
            return;
        }
        let (tx, rx) = channel();
        let stop = Arc::new(AtomicBool::new(false));
        let stop2 = stop.clone();
        std::thread::spawn(move || {
            for (i, (path, action, before, gate)) in work.into_iter().enumerate() {
                if stop2.load(Ordering::Relaxed) {
                    break;
                }
                let _ = tx.send(Msg::Start(i));
                // an application that must be closed: the process list and the files' users,
                // just before; an answer Windows can't give counts as open
                if let Some((app, exes)) = &gate {
                    if let Some(why) = still_open(&path, app, exes) {
                        let mut out = Outcome::default();
                        out.problems.push(why);
                        let _ = tx.send(Msg::Finished(i, out, None));
                        continue;
                    }
                }
                let mut last = Instant::now();
                let mut out = clean::run(&path, &action, &spare, &stop2, &mut |freed| {
                    if last.elapsed().as_millis() >= 200 {
                        last = Instant::now();
                        let _ = tx.send(Msg::Progress(i, freed));
                    }
                });
                // what is there now, for the tree
                let now = match std::fs::symlink_metadata(&path) {
                    Ok(_) => rescan(&path).ok().map(Some),
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => Some(None),
                    Err(_) => None,
                };
                // a tool removes things itself: what it freed is what the folder lost
                if matches!(action, Action::Tool { .. }) {
                    if let Some(now) = &now {
                        out.freed = before.saturating_sub(now.as_ref().map_or(0, |t| t.nodes[sprawl_core::ROOT].size));
                    }
                }
                let _ = tx.send(Msg::Finished(i, out, now));
            }
            let _ = tx.send(Msg::End);
        });
        let at = std::time::Instant::now();
        self.own_deletes.extend(items.iter().map(|it| (it.path.to_string_lossy().trim_end_matches('\\').to_lowercase(), at)));
        let c = &mut self.advisor.cleanup;
        c.review = false;
        c.single = None;
        c.done = None;
        c.running = Some(Running { items, rx, stop, rescans: Vec::new(), held });
        self.advisor_redraw();
    }

    /// Progress from the clean-up thread; at the end, the tree is updated in place.
    pub(crate) fn cleanup_poll(&mut self) {
        let Some(run) = self.advisor.cleanup.running.as_mut() else { return };
        let mut redraw = false;
        let mut ended = false;
        for m in run.rx.try_iter().collect::<Vec<_>>() {
            match m {
                Msg::Start(i) => {
                    if let Some(it) = run.items.get_mut(i) {
                        it.state = ItemState::Running(0);
                    }
                    redraw = true;
                }
                Msg::Progress(i, freed) => {
                    if let Some(it) = run.items.get_mut(i) {
                        it.state = ItemState::Running(freed);
                        if let Some(p) = &self.advisor.foot {
                            crate::state::text(p, &format!("AdvCS{i}"), &format!("Removing... {}", sprawl_core::format_size(freed)));
                        }
                    }
                }
                Msg::Finished(i, out, now) => {
                    if let Some(it) = run.items.get_mut(i) {
                        if let Some(now) = now {
                            run.rescans.push((it.path.clone(), now));
                        }
                        it.state = ItemState::Done(out);
                    }
                    redraw = true;
                }
                Msg::End => ended = true,
            }
        }
        if ended {
            self.cleanup_finish();
        } else if redraw {
            self.advisor_redraw();
        }
    }

    fn cleanup_finish(&mut self) {
        let Some(run) = self.advisor.cleanup.running.take() else { return };
        log_run(&run.items, &run.held, run.stop.load(Ordering::Relaxed));
        let mut done = Done { freed: 0, items: 0, kept_new: 0, kept_chosen: 0, problems: run.held.clone(), stopped: run.stop.load(Ordering::Relaxed), output: Vec::new() };
        for it in &run.items {
            if let ItemState::Done(o) = &it.state {
                if o.freed > 0 {
                    done.items += 1;
                }
                done.freed += o.freed;
                done.kept_new += o.kept_new;
                done.kept_chosen += o.kept_chosen;
                done.problems.extend(o.problems.iter().take(3).cloned());
                if o.more_problems > 0 || o.problems.len() > 3 {
                    done.problems.push(format!("{}: and {} more", it.title, o.more_problems + o.problems.len().saturating_sub(3) as u64));
                }
                done.output.extend(o.output.iter().cloned());
                // remembered as done when most of it went, never over the user's own Keep
                let kept = self.advisor.memory.hides(&it.path).is_some_and(|d| d.verdict == sprawl_advisor::memory::Verdict::Keep);
                if o.freed >= it.bytes / 2 && !kept {
                    self.advisor.memory.decide(&it.path, &it.title, sprawl_advisor::memory::Verdict::Done);
                }
            }
        }
        self.save_memory();
        self.advisor.cleanup.done = Some(done);
        // what went no longer counts as ticked
        self.advisor.cleanup.ticked.clear();
        self.advisor.cleanup.ticked_for = None;
        // the tree as it is now: each item's folder scanned again, or taken out when gone
        self.apply_rescans(run.rescans);
    }

    /// The clean-up's panel: running, or the result.
    pub(crate) fn cleanup_status_xaml(&self) -> String {
        let c = &self.advisor.cleanup;
        let mut x = String::new();
        if let Some(run) = &c.running {
            let freed: u64 = run.items.iter().map(|it| match &it.state {
                ItemState::Running(f) => *f,
                ItemState::Done(o) => o.freed,
                ItemState::Waiting => 0,
            }).sum();
            x += &format!(
                r##"<Border Padding="14,12" CornerRadius="8" Background="#142F7BF6" BorderBrush="#552F7BF6" BorderThickness="1"><StackPanel Spacing="8"><StackPanel Orientation="Horizontal" Spacing="10"><ProgressRing IsActive="True" Width="18" Height="18"/><TextBlock Text="Cleaning up... {} freed so far" FontSize="15" FontWeight="SemiBold" VerticalAlignment="Center"/><Button x:Name="AdvCleanStop" Content="Stop" Padding="10,2" FontSize="12" CornerRadius="5"/></StackPanel>"##,
                self.size(freed)
            );
            for (i, it) in run.items.iter().enumerate() {
                let (glyph, colour, state) = match &it.state {
                    ItemState::Waiting => ("&#xE823;", "#FF8E97AA", "Waiting".to_string()),
                    ItemState::Running(f) => ("&#xE895;", "#FF8FB4FF", format!("Removing... {}", self.size(*f))),
                    ItemState::Done(o) if o.problems.is_empty() => ("&#xE73E;", "#FF4CC38A", format!("Freed {}", self.size(o.freed))),
                    ItemState::Done(o) => ("&#xE7BA;", "#FFF5B342", format!("Freed {}; some couldn't go", self.size(o.freed))),
                };
                x += &format!(
                    r##"<Grid ColumnSpacing="10"><Grid.ColumnDefinitions><ColumnDefinition Width="18"/><ColumnDefinition Width="*"/><ColumnDefinition Width="Auto"/></Grid.ColumnDefinitions><FontIcon Glyph="{glyph}" FontSize="13" Foreground="{colour}"/><TextBlock Grid.Column="1" Text="{}" FontSize="12" TextTrimming="CharacterEllipsis"/><TextBlock x:Name="AdvCS{i}" Grid.Column="2" Text="{}" FontSize="12" Foreground="#FFB9C2D6"/></Grid>"##,
                    ui::esc(&it.title),
                    ui::esc(&state)
                );
            }
            x += "</StackPanel></Border>";
        } else if let Some(d) = &c.done {
            let fine = d.problems.is_empty() && !d.stopped && d.freed > 0;
            let (bg, border, glyph, colour) = if fine { ("#144CC38A", "#554CC38A", "&#xE73E;", "#FF4CC38A") } else { ("#14F5B342", "#55F5B342", "&#xE7BA;", "#FFF5B342") };
            let items = if d.items == 1 { "1 item".to_string() } else { format!("{} items", d.items) };
            let head = match (d.stopped, d.items) {
                (true, 0) => "Stopped. Nothing was freed.".to_string(),
                (true, _) => format!("Stopped. Freed {} from {items}.", self.size(d.freed)),
                (false, 0) => "Nothing was freed.".to_string(),
                (false, _) => format!("Freed {} from {items}.", self.size(d.freed)),
            };
            x += &format!(
                r##"<Border Padding="14,12" CornerRadius="8" Background="{bg}" BorderBrush="{border}" BorderThickness="1"><StackPanel Spacing="6"><Grid><StackPanel Orientation="Horizontal" Spacing="10"><FontIcon Glyph="{glyph}" FontSize="16" Foreground="{colour}"/><TextBlock Text="{}" FontSize="15" FontWeight="SemiBold"/></StackPanel><Button x:Name="AdvCleanedOk" HorizontalAlignment="Right" Content="OK" Padding="12,2" FontSize="12" CornerRadius="5"/></Grid><TextBlock Text="They were deleted for good. The tree, the map and the findings are updated.{}" FontSize="12" Foreground="#FFB9C2D6" TextWrapping="Wrap"/><StackPanel Orientation="Horizontal" Spacing="8"><Button x:Name="AdvCleanMore" Content="Find more with AI" Padding="10,3" FontSize="12" CornerRadius="5"/><Button x:Name="AdvCleanLog" Content="See what was removed" Padding="10,3" FontSize="12" CornerRadius="5"/></StackPanel>"##,
                ui::esc(&head),
                self.tree.as_ref().and_then(|t| crate::win32::disk_space(&t.path_of(sprawl_core::ROOT))).map_or(String::new(), |(_, free)| ui::esc(&format!(" The disk now has {} free.", self.size(free))))
            );
            if !d.problems.is_empty() {
                x += r##"<TextBlock Text="Some things couldn't go (in use, or no access): they stay as they were." FontSize="12" Foreground="#FFF5B342"/>"##;
                for p in d.problems.iter().take(8) {
                    x += &format!(r##"<TextBlock Text="{}" FontSize="12" Foreground="#FF8E97AA" TextWrapping="Wrap" Margin="10,0,0,0"/>"##, ui::esc(p));
                }
            }
            if d.kept_new + d.kept_chosen > 0 {
                let mut parts = Vec::new();
                if d.kept_new > 0 {
                    parts.push(format!("{} new since the scan", d.kept_new));
                }
                if d.kept_chosen > 0 {
                    parts.push(format!("{} you chose to keep", d.kept_chosen));
                }
                x += &format!(r##"<TextBlock Text="Left alone: {}." FontSize="12" Foreground="#FFB9C2D6" TextWrapping="Wrap"/>"##, ui::esc(&parts.join(", ")));
            }
            if !d.output.is_empty() {
                x += &format!(
                    r##"<Border Padding="10,8" CornerRadius="5" Background="#33000000"><TextBlock FontFamily="Consolas" FontSize="12" TextWrapping="Wrap" IsTextSelectionEnabled="True" Text="{}"/></Border>"##,
                    d.output.iter().map(|l| ui::esc(l)).collect::<Vec<_>>().join("&#10;")
                );
            }
            x += "</StackPanel></Border>";
        }
        x
    }

    /// The bar above the findings: what is ticked, and the ways to tick and clean.
    pub(crate) fn cleanup_bar_xaml(&self, goal: &str) -> String {
        let picked = self.ticked_findings();
        let bytes: u64 = picked.iter().map(|(_, f)| f.bytes).sum();
        let admin_waiting = self.advice().is_some_and(|r| r.findings.iter().zip(&r.counted).any(|(f, c)| untickable(f, *c) == Some("Needs administrator")));
        let mut x = format!(
            r##"<Border Padding="12,10" CornerRadius="8" Background="#0FFFFFFF" BorderBrush="#22FFFFFF" BorderThickness="1"><StackPanel Spacing="8">
<Grid ColumnSpacing="10"><Grid.ColumnDefinitions><ColumnDefinition Width="*"/><ColumnDefinition Width="Auto"/></Grid.ColumnDefinitions>
<StackPanel VerticalAlignment="Center"><TextBlock Text="{}" FontSize="15" FontWeight="SemiBold"/><TextBlock Text="{}" FontSize="12" Foreground="#FFA0A8B8" TextWrapping="Wrap"/></StackPanel>
<Button x:Name="AdvClean" Grid.Column="1" Content="Clean up..." Padding="16,6" FontSize="13" CornerRadius="6" Background="#FF2F7BF6" Foreground="White" IsEnabled="{}"/></Grid>
<StackPanel Orientation="Horizontal" Spacing="6"><Button x:Name="AdvSelSafe" Content="Tick the safe ones" Padding="10,3" FontSize="12" CornerRadius="5"/><Button x:Name="AdvSelNone" Content="Untick all" Padding="10,3" FontSize="12" CornerRadius="5"/><TextBlock Text="or tick enough for" FontSize="12" VerticalAlignment="Center" Margin="8,0,0,0" Foreground="#FFB9C2D6"/><TextBox x:Name="AdvGoal" Width="64" PlaceholderText="{goal}"/><TextBlock Text="GB" FontSize="12" VerticalAlignment="Center" Foreground="#FFB9C2D6"/><Button x:Name="AdvSelGoal" Content="Tick" Padding="10,3" FontSize="12" CornerRadius="5"/></StackPanel>"##,
            if picked.is_empty() { "Nothing ticked".to_string() } else { format!("{} ticked · {}", picked.len(), self.size(bytes)) },
            if self.advisor.cleanup.auto && !picked.is_empty() { "The safe ones are ticked. Change the ticks, then Clean up." } else { "Tick what should go, then Clean up. Sprawl removes it itself and shows what was freed." },
            if picked.is_empty() { "False" } else { "True" },
        );
        if admin_waiting {
            x += r##"<StackPanel Orientation="Horizontal" Spacing="8"><FontIcon Glyph="&#xE7EF;" FontSize="13" Foreground="#FFF5B342"/><TextBlock Text="Some items need administrator rights." FontSize="12" Foreground="#FFB9C2D6" VerticalAlignment="Center"/><Button x:Name="AdvAdmin" Content="Restart as administrator" Padding="10,2" FontSize="12" CornerRadius="5"/></StackPanel>"##;
        }
        x += r##"<Button x:Name="AdvExport" Padding="0" FontSize="12" BorderThickness="0" Background="Transparent" Foreground="#FF8E97AA" Content="Save as a script instead..."/>"##;
        x += "</StackPanel></Border>";
        x
    }

    /// The review before anything goes.
    pub(crate) fn cleanup_review_xaml(&self) -> String {
        let picked = self.ticked_findings();
        let bytes: u64 = picked.iter().map(|(_, f)| f.bytes).sum();
        let own = picked.iter().any(|(_, f)| f.risk >= Risk::Orange);
        let mut x = format!(
            r##"<Border Padding="14,12" CornerRadius="8" Background="#0FFFFFFF" BorderBrush="#552F7BF6" BorderThickness="1"><StackPanel Spacing="8"><TextBlock Text="Clean up {}, about {}?" FontSize="16" FontWeight="SemiBold"/><TextBlock Text="Deleted for good, not sent to the Recycle Bin." FontSize="13" FontWeight="SemiBold" Foreground="#FFF5B342"/><TextBlock Text="{}" FontSize="12" Foreground="#FFB9C2D6" TextWrapping="Wrap"/>"##,
            if picked.len() == 1 { "1 item".to_string() } else { format!("{} items", picked.len()) },
            self.size(bytes),
            {
                let tools = picked.iter().filter(|(_, f)| matches!(clean::action(f), Some((Action::Tool { .. }, _)))).count();
                let own = format!(
                    "Sprawl removes them itself. Links inside are removed as links (what they point to stays), files a program has locked are skipped, and anything created or changed since the scan began ({}) is left alone, as is anything you chose to keep.",
                    sprawl_core::local_time(self.scan_began)
                );
                let tool = "Each item with a program of its own runs that program's clean-up command, shown below, without a window. The program decides what goes: files new since the scan, or ones you chose to keep, may go too.";
                if tools == picked.len() {
                    tool.to_string()
                } else if tools > 0 {
                    format!("{own} {tool}")
                } else {
                    own
                }
            }
        );
        // the rows scroll; the boxes and buttons under them stay in view
        x += r##"<ScrollViewer MaxHeight="190" VerticalScrollBarVisibility="Auto" HorizontalScrollBarVisibility="Disabled" Padding="0,0,12,0"><StackPanel Spacing="8">"##;
        for (_, f) in &picked {
            let what = crate::advisor::risk_words(f.risk);
            let ai = if matches!(f.source, Source::Model(_)) { " · suggested by the AI" } else { "" };
            x += &format!(
                r##"<Grid ColumnSpacing="10"><Grid.ColumnDefinitions><ColumnDefinition Width="10"/><ColumnDefinition Width="*"/><ColumnDefinition Width="Auto"/></Grid.ColumnDefinitions><Ellipse Width="8" Height="8" Fill="{}"/><StackPanel Grid.Column="1"><TextBlock Text="{}" FontSize="13" TextTrimming="CharacterEllipsis"/><TextBlock Text="{}{ai}" FontSize="12" Foreground="#FF8E97AA" TextTrimming="CharacterEllipsis"/></StackPanel><TextBlock Grid.Column="2" Text="{}" FontSize="13" VerticalAlignment="Center"/></Grid>"##,
                crate::advisor::risk_hex(f.risk),
                ui::esc(&f.title),
                ui::esc(&match clean::action(f) {
                    Some((Action::Tool { command }, _)) => format!("{what} · runs: {command}"),
                    _ => format!("{what} · {}", f.path.display()),
                }),
                self.size(f.bytes)
            );
        }
        x += "</StackPanel></ScrollViewer>";
        // what the user keeps inside a ticked folder stays: say which
        let kept: Vec<PathBuf> = self
            .advisor
            .memory
            .kept_paths()
            .into_iter()
            .filter(|k| {
                let k = PathBuf::from(key(k));
                picked.iter().any(|(_, f)| !matches!(clean::action(f), Some((Action::Tool { .. }, _))) && k.starts_with(PathBuf::from(key(&f.path))))
            })
            .collect();
        if !kept.is_empty() {
            x += r##"<StackPanel Spacing="2"><TextBlock Text="Kept, because you chose to keep them:" FontSize="12" Foreground="#FFB9C2D6"/>"##;
            for k in kept.iter().take(6) {
                x += &format!(r##"<TextBlock Text="{}" FontSize="12" Foreground="#FF8E97AA" TextTrimming="CharacterEllipsis" Margin="10,0,0,0"/>"##, ui::esc(&k.display().to_string()));
            }
            if kept.len() > 6 {
                x += &format!(r##"<TextBlock Text="and {} more" FontSize="12" Foreground="#FF8E97AA" Margin="10,0,0,0"/>"##, kept.len() - 6);
            }
            x += "</StackPanel>";
        }
        // an open application: a part that must have it closed stays, the rest skip what it holds
        let mut must: Vec<String> = Vec::new();
        let mut warn: Vec<String> = Vec::new();
        for (_, f) in &picked {
            match self.app_open(f) {
                Some((app, sprawl_advisor::cleaners::Closed::Must)) if !must.contains(&app) => must.push(app),
                Some((app, sprawl_advisor::cleaners::Closed::Warn)) if !warn.contains(&app) => warn.push(app),
                _ => {}
            }
        }
        if !must.is_empty() || !warn.is_empty() {
            x += r##"<StackPanel Spacing="4">"##;
            if !must.is_empty() {
                x += &format!(
                    r##"<TextBlock Text="{} {} open: close {} first, or its parts stay. Sprawl never closes a program." FontSize="12" Foreground="#FFF5B342" TextWrapping="Wrap"/>"##,
                    ui::esc(&must.join(", ")),
                    if must.len() == 1 { "is" } else { "are" },
                    if must.len() == 1 { "it" } else { "them" }
                );
            }
            if !warn.is_empty() {
                x += &format!(r##"<TextBlock Text="{} {} open: files in use are skipped." FontSize="12" Foreground="#FFB9A36A" TextWrapping="Wrap"/>"##, ui::esc(&warn.join(", ")), if warn.len() == 1 { "is" } else { "are" });
            }
            x += r##"<Button x:Name="AdvClCheck" Content="Check again" Padding="10,2" FontSize="12" CornerRadius="5"/></StackPanel>"##;
        }
        if own {
            x += &format!(
                r##"<CheckBox x:Name="AdvOwnOk" IsChecked="{}" Content="These are my own files, I want them gone, and I know they can't be brought back" FontSize="13"/>"##,
                if self.advisor.cleanup.own_ok { "True" } else { "False" }
            );
        }
        x += &format!(r##"<StackPanel Orientation="Horizontal" Spacing="8"><Button x:Name="AdvCleanNow" Content="Clean up now" Padding="16,6" FontSize="13" CornerRadius="6" Background="#FF2F7BF6" Foreground="White" IsEnabled="{}"/>"##, if own && !self.advisor.cleanup.own_ok { "False" } else { "True" });
        x += r##"<Button x:Name="AdvReviewBack" Content="Back" Padding="14,6" FontSize="13" CornerRadius="6"/></StackPanel>"##;
        x += "</StackPanel></Border>";
        x
    }
}

impl State {
    /// Draw the Advisor again when it is on screen.
    pub(crate) fn advisor_redraw(&mut self) {
        if self.view == crate::state::View::Advisor {
            self.render_advisor();
        }
    }
}

impl State {
    /// A message for the user, in the Advisor's foot next to the buttons (or the status line
    /// when the Advisor isn't on screen).
    pub(crate) fn adv_say(&self, msg: &str) {
        *self.advisor.msg.borrow_mut() = Some(msg.to_string());
        let shown = self.view == crate::state::View::Advisor
            && self.advisor.foot.as_ref().and_then(|f| crate::find::<crate::bindings::Microsoft::UI::Xaml::Controls::TextBlock>(f, "AdvMsg").ok()).is_some_and(|t| {
                let _ = t.SetText(&msg.into());
                let _ = t.SetVisibility(crate::bindings::Microsoft::UI::Xaml::Visibility::Visible);
                true
            });
        if !shown {
            crate::state::text(&self.root, "StatusText", msg);
        }
    }

    /// The Advisor's foot, pinned under the list: the message, then the clean-up's status, review
    /// or bar, whatever section is open.
    pub(crate) fn advisor_foot_xaml(&self) -> String {
        let mut x = format!(r##"<StackPanel {} Spacing="8" Padding="18,10,18,12" BorderBrush="#22FFFFFF" BorderThickness="0,1,0,0" Background="#FF1A1D27">"##, crate::ui::NS);
        let msg = self.advisor.msg.borrow().clone();
        x += &format!(
            r##"<TextBlock x:Name="AdvMsg" Text="{}" FontSize="12" Foreground="#FF8FB4FF" TextWrapping="Wrap" Visibility="{}"/>"##,
            ui::esc(msg.as_deref().unwrap_or("")),
            if msg.is_some() { "Visible" } else { "Collapsed" }
        );
        x += &self.exported_xaml();
        let c = &self.advisor.cleanup;
        x += &self.cleanup_status_xaml();
        if c.running.is_none() {
            if c.review {
                x += &self.cleanup_review_xaml();
            } else if self.advice().is_some_and(|r| r.findings.iter().zip(&r.counted).any(|(f, k)| !matches!(untickable(f, *k), Some("")))) {
                x += &self.cleanup_bar_xaml(crate::advisor::GOAL_EXAMPLE);
            }
        }
        x += "</StackPanel>";
        x
    }
}

/// `%LOCALAPPDATA%\Sprawl\cleanup-log.txt`: what each clean-up removed, kept for the user.
pub(crate) fn log_path() -> Option<std::path::PathBuf> {
    Some(std::path::PathBuf::from(std::env::var_os("LOCALAPPDATA")?).join("Sprawl").join("cleanup-log.txt"))
}

/// Add a clean-up to the log (time, each item's path, what it freed and what couldn't go).
fn log_run(items: &[Item], held: &[String], stopped: bool) {
    let Some(p) = log_path() else { return };
    if let Some(dir) = p.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let mut s = format!("\r\n== {} (local time){}\r\n", sprawl_core::local_time(std::time::SystemTime::now()), if stopped { " (stopped)" } else { "" });
    for it in items {
        match &it.state {
            ItemState::Done(o) => {
                s += &format!("{}\tfreed {} bytes, {} files\r\n", it.path.display(), o.freed, o.files);
                if o.kept_new > 0 {
                    s += &format!("\t  left {} items new since the scan\r\n", o.kept_new);
                }
                if o.kept_chosen > 0 {
                    s += &format!("\t  left {} items you chose to keep\r\n", o.kept_chosen);
                }
                for pr in &o.problems {
                    s += &format!("\t  not removed: {pr}\r\n");
                }
                if o.more_problems > 0 {
                    s += &format!("\t  and {} more not removed\r\n", o.more_problems);
                }
            }
            _ => s += &format!("{}\tnot started\r\n", it.path.display()),
        }
    }
    for h in held {
        s += &format!("held back: {h}\r\n");
    }
    use std::io::Write;
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(&p) {
        let _ = f.write_all(s.as_bytes());
    }
}
