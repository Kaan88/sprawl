//! The Advisor's Cleaners section: the applications on this PC and their parts, BleachBit-like.
//! A part's matches are findings of the report, so a part's tick is the same tick as its rows in
//! Findings, and cleaning goes through the same review and executor. Nothing here deletes.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use sprawl_advisor::cleaners::{CleanerDef, Class, Closed, Presence, GROUPS, SHAPE};
use sprawl_advisor::{Finding, Risk, Source};

use crate::advisor::Report;
use crate::cleanup::{key, untickable};
use crate::state::State;
use crate::ui;

/// What the Cleaners section has open.
#[derive(Default)]
pub(crate) struct ClState {
    /// Open cleaners, by id.
    pub(crate) open: HashSet<String>,
    /// The part whose files are shown (and lit on the map), by its key.
    pub(crate) show: Option<String>,
    /// The map lens for the shown part: (tree generation, a code per node).
    pub(crate) lens: Option<(u64, Arc<Vec<u8>>)>,
    /// Cleaners whose definition is shown.
    pub(crate) def: HashSet<String>,
    /// The programs running in this session, and when that was read (`None` inside: unknown).
    pub(crate) procs: Option<(std::time::Instant, Option<HashSet<String>>)>,
}

/// The "keep the last N days" choices offered for a part whose definition keeps `def`.
fn day_choices(def: u32) -> Vec<u32> {
    let mut v: Vec<u32> = [def, 1, 3, 7, 30, 90].into_iter().filter(|&d| d >= def).collect();
    v.sort_unstable();
    v.dedup();
    v
}

/// One cleaner as listed.
pub(crate) struct ClRow {
    pub(crate) id: String,
    pub(crate) name: String,
    pub(crate) group: &'static str,
    /// Index into the report's catalogue (None: the "found by shape" list).
    pub(crate) def: Option<usize>,
    pub(crate) presence: Presence,
    pub(crate) parts: Vec<PartRow>,
}

pub(crate) struct PartRow {
    pub(crate) key: String,
    pub(crate) title: String,
    pub(crate) risk: Risk,
    pub(crate) class: Class,
    pub(crate) closed: Closed,
    pub(crate) cost: String,
    pub(crate) why: String,
    pub(crate) info: bool,
    /// It clears a folder (so "keep the last N days" applies).
    pub(crate) clears: bool,
    pub(crate) keep_days: Option<u32>,
    /// The findings that are this part (indices into the report).
    pub(crate) findings: Vec<usize>,
}

impl PartRow {
    pub(crate) fn bytes(&self, r: &Report) -> u64 {
        self.findings.iter().filter(|&&i| r.counted[i]).map(|&i| r.findings[i].bytes).sum()
    }

    /// The findings that can be ticked.
    fn tickable<'a>(&'a self, r: &'a Report) -> impl Iterator<Item = usize> + 'a {
        self.findings.iter().copied().filter(move |&i| untickable(&r.findings[i], r.counted[i]).is_none())
    }
}

/// Where in the group order a group goes.
fn group_of(g: &str) -> &'static str {
    GROUPS.iter().copied().find(|x| *x == g).unwrap_or("Other")
}

impl State {
    /// The cleaners to list for report `r`, in order: by group, then the biggest first. Cleaners
    /// whose application isn't on this PC are left out.
    pub(crate) fn cleaner_layout(&self, r: &Report) -> Vec<ClRow> {
        let mut by_rule: HashMap<&str, Vec<usize>> = HashMap::new();
        for (i, f) in r.findings.iter().enumerate() {
            if let Source::Rule(id) = &f.source {
                by_rule.entry(id.as_str()).or_default().push(i);
            }
        }
        let mut rows: Vec<ClRow> = Vec::new();
        for (k, d) in r.catalogue.iter().enumerate() {
            let presence = r.presence.get(k).cloned().unwrap_or(Presence::Absent);
            if presence == Presence::Absent {
                continue;
            }
            let parts = d
                .parts
                .iter()
                .map(|p| PartRow {
                    key: p.rule.clone(),
                    title: p.title.clone(),
                    risk: p.risk,
                    class: p.class,
                    closed: p.closed,
                    cost: p.cost.clone(),
                    why: p.why.clone(),
                    info: p.info,
                    clears: p.clears,
                    keep_days: p.keep_days,
                    findings: by_rule.get(p.rule.as_str()).cloned().unwrap_or_default(),
                })
                .collect();
            rows.push(ClRow { id: d.id.clone(), name: d.name.clone(), group: group_of(&d.group), def: Some(k), presence, parts });
        }
        // caches of programs no cleaner knows, one part each
        let shape: Vec<PartRow> = by_rule
            .get(SHAPE)
            .into_iter()
            .flatten()
            .map(|&i| {
                let f = &r.findings[i];
                PartRow {
                    key: key(&f.path),
                    title: f.title.clone(),
                    risk: f.risk,
                    class: Class::Space,
                    closed: Closed::Warn,
                    cost: "The program downloads or makes these again; it may be slower once.".into(),
                    why: f.evidence.first().cloned().unwrap_or_default(),
                    info: false,
                    clears: true,
                    keep_days: None,
                    findings: vec![i],
                }
            })
            .collect();
        if !shape.is_empty() {
            rows.push(ClRow { id: SHAPE.into(), name: "Other apps (found by shape)".into(), group: "Other", def: None, presence: Presence::InScan, parts: shape });
        }
        let size = |c: &ClRow| -> u64 { c.parts.iter().map(|p| p.bytes(r)).sum() };
        rows.sort_by_key(|c| (GROUPS.iter().position(|g| *g == c.group).unwrap_or(GROUPS.len()), c.id == SHAPE, std::cmp::Reverse(size(c))));
        rows
    }

    /// How many cleaners and parts are listed (for wiring their buttons).
    pub(crate) fn cleaner_counts(&self) -> (usize, usize) {
        let Some(r) = self.advice() else { return (0, 0) };
        let rows = self.cleaner_layout(r);
        (rows.len(), rows.iter().map(|c| c.parts.len()).sum())
    }

    /// The Cleaners section.
    pub(crate) fn cleaners_section(&self, r: &Report) -> String {
        let rows = self.cleaner_layout(r);
        let mut x = String::from(
            r##"<TextBlock TextWrapping="Wrap" FontSize="12" Foreground="#FF8E97AA" Text="The applications on this PC, and what each keeps that can go. A part's tick is the same as its row in Findings: one bar, one review, and nothing goes before you press Clean up."/>"##,
        );
        if !self.usual_keys(r).is_empty() {
            x += r##"<StackPanel Orientation="Horizontal" Spacing="8"><Button x:Name="AdvClUsual" Content="Tick my usual ones" Padding="10,3" FontSize="12" CornerRadius="5" ToolTipService.ToolTip="The parts that take time to get back which you cleaned last time"/></StackPanel>"##;
        }
        if rows.is_empty() {
            x += r##"<TextBlock Text="No application Sprawl knows is in this scan." TextWrapping="Wrap" Foreground="#FFB9C2D6"/>"##;
            return x;
        }
        let running = self.advisor.cleanup.running.is_some();
        let mut part_no = 0usize;
        let mut last_group = "";
        for (ci, c) in rows.iter().enumerate() {
            if c.group != last_group {
                last_group = c.group;
                x += &format!(r##"<TextBlock Text="{}" FontSize="12" FontWeight="SemiBold" Foreground="#FFB9C2D6" Margin="0,6,0,0"/>"##, ui::esc(c.group));
            }
            let open = self.advisor.cl.open.contains(&c.id);
            let bytes: u64 = c.parts.iter().map(|p| p.bytes(r)).sum();
            let ticked = c.parts.iter().filter(|p| p.findings.iter().any(|&i| self.advisor.cleanup.ticked.contains(&key(&r.findings[i].path)))).count();
            let sub = match &c.presence {
                Presence::Elsewhere(_) => "Not in this scan".to_string(),
                _ if bytes == 0 => "Nothing to clean now".to_string(),
                _ => {
                    let n = c.parts.iter().filter(|p| p.bytes(r) > 0).count();
                    let mut s = format!("{n} {}", if n == 1 { "part" } else { "parts" });
                    if ticked > 0 {
                        s += &format!(" · {ticked} ticked");
                    }
                    s
                }
            };
            let scan = match &c.presence {
                Presence::Elsewhere(drive) => format!(
                    r##"<Button x:Name="AdvClScan{ci}" Content="Scan {}" Padding="10,3" FontSize="12" CornerRadius="5" Margin="8,0,0,0"/>"##,
                    ui::esc(drive.display().to_string().trim_end_matches('\\'))
                ),
                _ => String::new(),
            };
            x += &format!(
                r##"<Grid ColumnSpacing="6"><Grid.ColumnDefinitions><ColumnDefinition Width="*"/><ColumnDefinition Width="Auto"/></Grid.ColumnDefinitions><Button x:Name="AdvClO{ci}" HorizontalAlignment="Stretch" HorizontalContentAlignment="Stretch" Padding="10,7" CornerRadius="7" BorderThickness="0" Background="{bg}" AutomationProperties.Name="{name}">
<Grid ColumnSpacing="10"><Grid.ColumnDefinitions><ColumnDefinition Width="14"/><ColumnDefinition Width="*"/><ColumnDefinition Width="90"/></Grid.ColumnDefinitions>
<FontIcon Glyph="{chev}" FontSize="10" Foreground="#FF8E97AA" VerticalAlignment="Center"/>
<StackPanel Grid.Column="1" Spacing="1"><TextBlock Text="{name}" FontSize="13" TextTrimming="CharacterEllipsis"/><TextBlock Text="{sub}" FontSize="12" Foreground="#FF8E97AA" TextTrimming="CharacterEllipsis"/></StackPanel>
<TextBlock Grid.Column="2" Text="{size}" FontSize="13" HorizontalAlignment="Right" VerticalAlignment="Center" Opacity="{op}"/>
</Grid></Button>{scan}</Grid>"##,
                bg = if open { "#FF262C3D" } else { "#0CFFFFFF" },
                name = ui::esc(&c.name),
                chev = if open { "&#xE70D;" } else { "&#xE76C;" },
                sub = ui::esc(&sub),
                size = if bytes > 0 { self.size(bytes) } else { String::new() },
                op = if bytes > 0 { "1" } else { "0.5" },
            );
            if open {
                x += r##"<StackPanel Spacing="2" Margin="22,0,0,4">"##;
                for p in &c.parts {
                    x += &self.part_row(r, c, p, part_no, running);
                    part_no += 1;
                }
                if let Some(k) = c.def {
                    let d = &r.catalogue[k];
                    let showing = self.advisor.cl.def.contains(&c.id);
                    x += &format!(
                        r##"<StackPanel Orientation="Horizontal" Spacing="8" Margin="0,4,0,0"><TextBlock Text="{}" FontSize="12" Foreground="#FF8E97AA" VerticalAlignment="Center"/><Button x:Name="AdvClDef{ci}" Content="{}" Padding="8,1" FontSize="12" CornerRadius="4" BorderThickness="0" Background="Transparent"/></StackPanel>"##,
                        ui::esc(&format!("From {}", d.pack)),
                        if showing { "Hide the definition" } else { "Show the definition" }
                    );
                    if showing {
                        x += &format!(
                            r##"<Border Padding="10,8" CornerRadius="5" Background="#33000000"><TextBlock FontFamily="Consolas" FontSize="12" TextWrapping="Wrap" IsTextSelectionEnabled="True" Text="{}"/></Border>"##,
                            d.text.lines().map(ui::esc).collect::<Vec<_>>().join("&#10;")
                        );
                    }
                }
                x += "</StackPanel>";
            } else {
                part_no += c.parts.len();
            }
        }
        x
    }

    fn part_row(&self, r: &Report, c: &ClRow, p: &PartRow, n: usize, running: bool) -> String {
        let bytes = p.bytes(r);
        let tickable: Vec<usize> = p.tickable(r).collect();
        let all_ticked = !tickable.is_empty() && tickable.iter().all(|&i| self.advisor.cleanup.ticked.contains(&key(&r.findings[i].path)));
        // why a part with something in it can't be ticked: the first finding's reason
        let why_not = p.findings.first().and_then(|&i| untickable(&r.findings[i], r.counted[i])).filter(|w| !w.is_empty());
        let tick = if !tickable.is_empty() {
            format!(
                r##"<CheckBox x:Name="AdvClT{n}" MinWidth="0" Padding="0" VerticalAlignment="Center" IsChecked="{}" IsEnabled="{}" AutomationProperties.Name="Clean up {}"/>"##,
                if all_ticked { "True" } else { "False" },
                if running { "False" } else { "True" },
                ui::esc(&format!("{}: {}", c.name, p.title))
            )
        } else if let (Some(w), true) = (why_not, bytes > 0) {
            format!(r##"<FontIcon Glyph="&#xE72E;" FontSize="13" Foreground="#FFA0A8B8" VerticalAlignment="Center" ToolTipService.ToolTip="Can't be ticked: {0}" AutomationProperties.Name="Can't be ticked: {0}"/>"##, ui::esc(w))
        } else {
            String::new()
        };
        let label = if p.info {
            "Tip".to_string()
        } else {
            match p.class {
                Class::Privacy => "Privacy".to_string(),
                Class::Space => crate::advisor::risk_label(p.risk).to_string(),
            }
        };
        let mut sub = label;
        let days = self.part_days(p);
        if days > 0 && !p.clears {
            sub += &format!(" · keeps the last {days} days");
        }
        if p.closed == Closed::Must {
            sub += " · only with the app closed";
        }
        if !p.cost.is_empty() {
            sub += &format!(" · {}", p.cost);
        }
        // the application being open
        let open = p.findings.first().and_then(|&i| self.app_open(&r.findings[i]));
        let warn = match &open {
            Some((app, Closed::Must)) => format!(r##"<TextBlock Text="Close {} first: this can't be cleaned while it is open." FontSize="12" Foreground="#FFF5B342" TextWrapping="Wrap"/>"##, ui::esc(app)),
            Some((app, Closed::Warn)) => format!(r##"<TextBlock Text="{} is open: files it is using will be skipped." FontSize="12" Foreground="#FFB9A36A" TextWrapping="Wrap"/>"##, ui::esc(app)),
            _ => String::new(),
        };
        let keep = if p.clears && !p.info && bytes > 0 {
            format!(
                r##"<Button x:Name="AdvClK{n}" Content="{}" Padding="8,1" FontSize="12" CornerRadius="4" BorderThickness="0" Background="Transparent" Foreground="#FF8FB4FF" ToolTipService.ToolTip="What changed lately stays. Press to keep more days." Margin="-8,0,0,0"/>"##,
                if days == 0 { "Keep the newest files?".to_string() } else { format!("Keeps the last {days} days (more)") }
            )
        } else {
            String::new()
        };
        let shown = self.advisor.cl.show.as_deref() == Some(p.key.as_str());
        let mut x = format!(
            r##"<Grid ColumnSpacing="6" Opacity="{op}"><Grid.ColumnDefinitions><ColumnDefinition Width="34"/><ColumnDefinition Width="*"/><ColumnDefinition Width="Auto"/><ColumnDefinition Width="80"/></Grid.ColumnDefinitions>{tick}
<StackPanel Grid.Column="1" Orientation="Horizontal" Spacing="8" Margin="0,4,0,4"><Ellipse Width="9" Height="9" Fill="{dot}" VerticalAlignment="Top" Margin="0,5,0,0"/><StackPanel Spacing="1"><TextBlock Text="{title}" FontSize="13" TextTrimming="CharacterEllipsis"/><TextBlock Text="{sub}" FontSize="12" Foreground="#FF8E97AA" TextWrapping="Wrap" MaxWidth="560"/>{warn}{keep}</StackPanel></StackPanel>
{files}<TextBlock Grid.Column="3" Text="{size}" FontSize="13" HorizontalAlignment="Right" VerticalAlignment="Center"/></Grid>"##,
            op = if bytes > 0 { "1" } else { "0.55" },
            dot = if p.info { "#FF8E97AA" } else { crate::advisor::risk_hex(p.risk) },
            title = ui::esc(&p.title),
            sub = ui::esc(&sub),
            files = if bytes > 0 {
                format!(
                    r##"<Button x:Name="AdvClS{n}" Grid.Column="2" Content="{}" Padding="8,1" FontSize="12" CornerRadius="4" VerticalAlignment="Center" ToolTipService.ToolTip="What would go, and where it is on the map"/>"##,
                    if shown { "Hide files" } else { "Show files" }
                )
            } else {
                String::new()
            },
            size = if bytes > 0 { self.size(bytes) } else { "-".into() },
        );
        if shown {
            x += &self.part_files(r, p);
        }
        x
    }

    /// The biggest files a part would remove, and where they are.
    fn part_files(&self, r: &Report, p: &PartRow) -> String {
        let Some(t) = &self.tree else { return String::new() };
        let mut files: Vec<usize> = Vec::new();
        let mut count = 0u64;
        for &i in &p.findings {
            let Some(n) = r.nodes.get(i).copied().flatten() else { continue };
            let mut stack = vec![n];
            while let Some(k) = stack.pop() {
                let node = &t.nodes[k];
                if node.is_dir {
                    stack.extend_from_slice(&node.children);
                } else {
                    count += 1;
                    files.push(k);
                }
            }
        }
        files.sort_by_key(|&k| std::cmp::Reverse(t.nodes[k].size));
        let mut x = String::from(r##"<Border Margin="34,0,0,4" Padding="12,8" CornerRadius="7" Background="#0AFFFFFF"><StackPanel Spacing="2">"##);
        x += &format!(r##"<TextBlock Text="{}" FontSize="12" Foreground="#FFB9C2D6" TextWrapping="Wrap"/>"##, ui::esc(&p.why));
        for &i in &p.findings {
            x += &format!(r##"<TextBlock Text="{}" FontSize="12" Foreground="#FF8E97AA" IsTextSelectionEnabled="True" TextTrimming="CharacterEllipsis"/>"##, ui::esc(&r.findings[i].path.display().to_string()));
        }
        x += &format!(r##"<TextBlock Text="{} files, the biggest:" FontSize="12" Foreground="#FF8E97AA" Margin="0,4,0,0"/>"##, crate::format::thousands(count));
        for &k in files.iter().take(12) {
            let name = t.nodes[k].name.clone();
            x += &format!(
                r##"<Grid><Grid.ColumnDefinitions><ColumnDefinition Width="*"/><ColumnDefinition Width="80"/></Grid.ColumnDefinitions><TextBlock Text="{}" FontSize="12" FontFamily="Consolas" Foreground="#FFB9C2D6" TextTrimming="CharacterEllipsis"/><TextBlock Grid.Column="1" Text="{}" FontSize="12" HorizontalAlignment="Right" Foreground="#FFB9C2D6"/></Grid>"##,
                ui::esc(&name),
                self.size(t.nodes[k].size)
            );
        }
        x += "</StackPanel></Border>";
        x
    }

    /// A Cleaners button: open a cleaner, tick a part, show a part's files, scan a drive.
    pub(crate) fn cleaner_command(&mut self, kind: &str, i: usize) {
        let Some(r) = self.advice() else { return };
        let rows = self.cleaner_layout(r);
        match kind {
            "AdvClO" => {
                let Some(c) = rows.get(i) else { return };
                if !self.advisor.cl.open.remove(&c.id) {
                    self.advisor.cl.open.insert(c.id.clone());
                }
            }
            "AdvClDef" => {
                let Some(c) = rows.get(i) else { return };
                if !self.advisor.cl.def.remove(&c.id) {
                    self.advisor.cl.def.insert(c.id.clone());
                }
            }
            "AdvClScan" => {
                if let Some(Presence::Elsewhere(drive)) = rows.get(i).map(|c| c.presence.clone()) {
                    self.start_scan(drive);
                }
                return;
            }
            "AdvClT" => {
                let Some(p) = rows.iter().flat_map(|c| c.parts.iter()).nth(i) else { return };
                let keys: Vec<String> = p.tickable(r).map(|k| key(&r.findings[k].path)).collect();
                self.cleanup_tick_many(keys);
                self.advisor.focus_after = Some(format!("AdvClT{i}"));
            }
            "AdvClK" => {
                let Some(p) = rows.iter().flat_map(|c| c.parts.iter()).nth(i) else { return };
                let def = p.keep_days.unwrap_or(0);
                let cur = self.part_days(p);
                let choices = day_choices(def);
                let next = choices.iter().copied().find(|&d| d > cur).unwrap_or(def);
                let (rule, findings) = (p.key.clone(), p.findings.clone());
                if next == def {
                    self.advisor.memory.part_days.remove(&rule);
                } else {
                    self.advisor.memory.part_days.insert(rule, next);
                }
                self.save_memory();
                // the findings of this report act on the new number now
                if let Some(r) = self.advisor.report.as_mut() {
                    for k in findings {
                        if let Some(f) = r.findings.get_mut(k) {
                            if let Some(sprawl_advisor::clean::Action::Clear { .. }) = f.action {
                                let admin = f.admin;
                                f.set_action(sprawl_advisor::clean::Action::Clear { days: next }, admin);
                            }
                        }
                    }
                }
            }
            "AdvClS" => {
                let Some(p) = rows.iter().flat_map(|c| c.parts.iter()).nth(i) else { return };
                if self.advisor.cl.show.as_deref() == Some(p.key.as_str()) {
                    self.advisor.cl.show = None;
                    self.advisor.cl.lens = None;
                } else {
                    let nodes: Vec<usize> = p.findings.iter().filter_map(|&k| r.nodes.get(k).copied().flatten()).collect();
                    self.advisor.cl.show = Some(p.key.clone());
                    self.advisor.cl.lens = self.tree.as_ref().map(|t| {
                        let mut v = vec![0u8; t.nodes.len()];
                        let mut stack = nodes;
                        while let Some(k) = stack.pop() {
                            v[k] = crate::search::LENS_MATCH;
                            stack.extend_from_slice(&t.nodes[k].children);
                        }
                        (self.tree_gen, Arc::new(v))
                    });
                }
                self.map.drawn_once = false;
            }
            _ => return,
        }
        self.render_advisor();
    }

    /// The part's "keep the last N days": the user's choice, never below the definition's.
    pub(crate) fn part_days(&self, p: &PartRow) -> u32 {
        let def = p.keep_days.unwrap_or(0);
        self.advisor.memory.part_days.get(&p.key).copied().unwrap_or(def).max(def)
    }

    /// Read the running programs again when the last look is older than a few seconds.
    pub(crate) fn cleaner_procs(&mut self, force: bool) {
        let stale = self.advisor.cl.procs.as_ref().is_none_or(|(t, _)| t.elapsed().as_secs() >= 3);
        if force || stale {
            self.advisor.cl.procs = Some((std::time::Instant::now(), sprawl_advisor::procs::running()));
        }
    }

    /// The application of `f`'s cleaner part when it is open (or when that can't be told and
    /// the part must have it closed), with what that means for the part.
    pub(crate) fn app_open(&self, f: &Finding) -> Option<(String, Closed)> {
        let r = self.advice()?;
        let Source::Rule(id) = &f.source else { return None };
        let (d, p) = sprawl_advisor::cleaners::part_of(&r.catalogue, id)?;
        if d.running.is_empty() || p.closed == Closed::No {
            return None;
        }
        let now = self.advisor.cl.procs.as_ref().and_then(|(_, s)| s.as_ref());
        match sprawl_advisor::procs::any_running(&d.running, now) {
            Some(true) => Some((d.name.clone(), p.closed)),
            None if p.closed == Closed::Must => Some((d.name.clone(), Closed::Must)),
            _ => None,
        }
    }

    /// For a part that must have its application closed: the application and its programs, to
    /// check again just before the part is cleaned.
    pub(crate) fn part_gate(&self, f: &Finding) -> Option<(String, Vec<String>)> {
        let r = self.advice()?;
        let Source::Rule(id) = &f.source else { return None };
        let (d, p) = sprawl_advisor::cleaners::part_of(&r.catalogue, id)?;
        (p.closed == Closed::Must && !d.running.is_empty()).then(|| (d.name.clone(), d.running.clone()))
    }

    /// The paths "Tick my usual ones" would tick.
    pub(crate) fn usual_keys(&self, r: &Report) -> Vec<String> {
        let usual = &self.advisor.memory.usual;
        if usual.is_empty() {
            return Vec::new();
        }
        r.findings
            .iter()
            .enumerate()
            .filter(|(i, f)| matches!(&f.source, Source::Rule(id) if usual.contains(id)) && untickable(f, r.counted[*i]).is_none())
            .map(|(_, f)| key(&f.path))
            .filter(|k| !self.advisor.cleanup.ticked.contains(k))
            .collect()
    }

    /// Remember the amber space parts in a clean-up, for "Tick my usual ones" next time.
    pub(crate) fn remember_usual(&mut self, picked: &[Finding]) {
        let Some(r) = self.advice() else { return };
        let mut ids: Vec<String> = picked
            .iter()
            .filter(|f| f.risk == Risk::Amber)
            .filter_map(|f| if let Source::Rule(id) = &f.source { Some(id.clone()) } else { None })
            .filter(|id| sprawl_advisor::cleaners::part_of(&r.catalogue, id).is_some_and(|(_, p)| p.class == Class::Space))
            .collect();
        ids.sort();
        ids.dedup();
        if !ids.is_empty() && ids != self.advisor.memory.usual {
            self.advisor.memory.usual = ids;
            self.save_memory();
        }
    }

    /// The cleaners as text, for the read-only MCP tool.
    pub(crate) fn cleaners_text(&self, r: &Report) -> String {
        let rows: Vec<(String, Vec<(String, Risk, u64, Vec<String>)>)> = self
            .cleaner_layout(r)
            .into_iter()
            .filter(|c| c.presence == Presence::InScan)
            .map(|c| {
                let parts = c
                    .parts
                    .iter()
                    .filter(|p| p.bytes(r) > 0)
                    .map(|p| (p.title.clone(), p.risk, p.bytes(r), p.findings.iter().map(|&i| r.findings[i].path.display().to_string()).collect()))
                    .collect();
                (c.name, parts)
            })
            .collect();
        sprawl_advisor::cleaners::as_text(&rows)
    }

    /// The map lens for the part whose files are shown, while the Cleaners section is on.
    pub(crate) fn cleaner_preview(&self) -> Option<Arc<Vec<u8>>> {
        if self.view != crate::state::View::Advisor || !self.advisor.cleaners_on() || self.advisor.cl.show.is_none() {
            return None;
        }
        self.advisor.cl.lens.as_ref().filter(|(g, _)| *g == self.tree_gen).map(|(_, l)| l.clone())
    }
}

/// The cleaners and where each one's application is, for a report (off the UI thread).
pub(crate) fn presences(tree: &sprawl_core::Tree, catalogue: &[CleanerDef], findings: &[Finding]) -> Vec<Presence> {
    let found: HashSet<&str> = findings.iter().filter_map(|f| if let Source::Rule(id) = &f.source { Some(id.as_str()) } else { None }).collect();
    catalogue.iter().map(|d| sprawl_advisor::cleaners::presence(tree, d, d.parts.iter().any(|p| found.contains(p.rule.as_str())))).collect()
}
