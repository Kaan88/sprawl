//! The Search box's surroundings in the window: the hint line under it (what the box could not
//! understand), the "?" help and the empty state, built from [`crate::search_query`]. Every
//! control here writes into the box or reads from it; the box always holds the real query.

use sprawl_core::query::Query;

use crate::bindings::Microsoft::UI::Xaml::Controls::Primitives::{ButtonBase, FlyoutBase};
use crate::bindings::Microsoft::UI::Xaml::Controls::{Border, MenuFlyout, MenuFlyoutItem, TextBox};
use crate::bindings::Microsoft::UI::Xaml::{FocusState, FrameworkElement, UIElement, Visibility};
use crate::search_query as sq;
use crate::state::{find, keep, load, text, with, State, View};
use crate::ui;

/// The hint line when nothing is wrong.
const TIP: &str = "Enter goes to the results: Enter there opens, Ctrl+Enter shows it in the tree, Esc comes back here.   ? lists everything the box understands.";

/// The help list as XAML: each example a button named `{prefix}{line}_{example}`.
fn help_xaml(prefix: &str, intro: &str) -> String {
    let mut x = format!(r##"<StackPanel {} Spacing="6">"##, ui::NS);
    if !intro.is_empty() {
        x += &format!(r##"<TextBlock Text="{}" FontSize="13" Foreground="#FFB9C2D6" TextWrapping="Wrap" Margin="0,0,0,4"/>"##, ui::esc(intro));
    }
    for (i, (examples, what)) in sq::help().iter().enumerate() {
        x += r##"<Grid ColumnSpacing="12"><Grid.ColumnDefinitions><ColumnDefinition Width="230"/><ColumnDefinition Width="*"/></Grid.ColumnDefinitions><StackPanel Orientation="Horizontal" Spacing="4">"##;
        for (j, e) in examples.iter().enumerate() {
            x += &format!(
                r##"<Button x:Name="{prefix}{i}_{j}" Padding="6,1" FontSize="12" FontFamily="Consolas" Background="#1AFFFFFF" BorderThickness="0" ToolTipService.ToolTip="Put it in the search box"><TextBlock Text="{}"/></Button>"##,
                ui::esc(e)
            );
        }
        x += &format!(
            r##"</StackPanel><TextBlock Grid.Column="1" Text="{}" FontSize="12" Foreground="#FF8E97AA" TextWrapping="Wrap" VerticalAlignment="Center"/></Grid>"##,
            ui::esc(what)
        );
    }
    x + "</StackPanel>"
}

impl State {
    /// The parts of the Search bar made at run time, once: the help, the empty state.
    pub(crate) fn search_ui_build(&mut self) {
        if self.search.ui_built {
            return;
        }
        self.search.ui_built = true;
        let help = sq::help();
        for (host, prefix, intro) in [
            ("SearchHelpHost", "SxHelp", ""),
            ("SearchEmptyHost", "SxEmpty", "Type a name to find it in this scan (and on the drives switched on below). The filters under the box write their words into it. Or start from an example:"),
        ] {
            let Ok(h) = find::<Border>(&self.root, host) else { continue };
            let Ok(content) = load::<UIElement>(&help_xaml(prefix, intro)) else { continue };
            let _ = h.SetChild(&content);
            let Ok(fe) = content.cast::<FrameworkElement>() else { continue };
            for (i, (examples, _)) in help.iter().enumerate() {
                for (j, e) in examples.iter().enumerate() {
                    let e: &'static str = e;
                    if let Ok(b) = find::<ButtonBase>(&fe, &format!("{prefix}{i}_{j}")) {
                        if let Ok(t) = b.Click(move |_, _| {
                            with(|s| s.search_example(e));
                        }) {
                            keep(t);
                        }
                    }
                }
            }
        }
        self.search_chips_build();
        self.search_box_state();
    }

    /// The filter chips under the box: a button that shows the chip (and takes its words out
    /// of the box when lit) and a menu of options that write words into the box.
    fn search_chips_build(&mut self) {
        let Ok(host) = find::<Border>(&self.root, "SearchChips") else { return };
        let today = sq::today();
        let mut x = format!(r##"<StackPanel {} Orientation="Horizontal" Spacing="6">"##, ui::NS);
        for (k, chip) in sq::CHIPS.iter().enumerate() {
            let items: String = chip
                .options(&today)
                .iter()
                .enumerate()
                .map(|(i, (label, _))| format!(r##"<MenuFlyoutItem x:Name="Ck{k}_{i}" Text="{}"/>"##, ui::esc(label)))
                .collect();
            x += &format!(
                r##"<StackPanel Orientation="Horizontal"><FlyoutBase.AttachedFlyout><MenuFlyout x:Name="ChipFly{k}" Placement="Bottom">{items}</MenuFlyout></FlyoutBase.AttachedFlyout><Button x:Name="Chip{k}" {off} Padding="10,2" CornerRadius="4,0,0,4" ToolTipService.ToolTip="Pick a filter: its words go into the search box. Lit: click to take them out again."><TextBlock x:Name="ChipText{k}" Text="{t}" MaxWidth="280" TextTrimming="CharacterEllipsis"/></Button><Button x:Name="ChipMenu{k}" {off} Padding="6,2" CornerRadius="0,4,4,0" ToolTipService.ToolTip="Choose"><FontIcon Glyph="&#xE70D;" FontSize="10"/></Button></StackPanel>"##,
                t = ui::esc(chip.title()),
                off = crate::look::pick(crate::look::Pick::Chip, false),
            );
        }
        x += "</StackPanel>";
        let Ok(content) = load::<UIElement>(&x) else { return };
        let _ = host.SetChild(&content);
        let Ok(fe) = content.cast::<FrameworkElement>() else { return };
        for (k, chip) in sq::CHIPS.iter().enumerate() {
            for (name, open) in [(format!("Chip{k}"), false), (format!("ChipMenu{k}"), true)] {
                if let Ok(b) = find::<ButtonBase>(&fe, &name) {
                    if let Ok(t) = b.Click(move |_, _| {
                        with(|s| s.search_chip_click(k, open));
                    }) {
                        keep(t);
                    }
                }
            }
            for i in 0..chip.options(&today).len() {
                if let Ok(m) = find::<MenuFlyoutItem>(&fe, &format!("Ck{k}_{i}")) {
                    if let Ok(t) = m.Click(move |_, _| {
                        with(|s| s.search_chip_pick(k, i));
                    }) {
                        keep(t);
                    }
                }
            }
        }
        self.search.chips = Some(fe);
    }

    /// A chip was clicked: lit, its words come out of the box; otherwise (or on its arrow) its
    /// options show.
    pub(crate) fn search_chip_click(&mut self, k: usize, open: bool) {
        let Some(&chip) = sq::CHIPS.get(k) else { return };
        let lit = sq::lit(&self.search.query)[k].is_some();
        if lit && !open {
            let t = sq::remove(&self.search.query, chip);
            let at = t.len();
            self.search_set_box(&t, at);
            return;
        }
        let Some(fe) = self.search.chips.clone() else { return };
        if let (Ok(m), Ok(b)) = (find::<MenuFlyout>(&fe, &format!("ChipFly{k}")), find::<FrameworkElement>(&fe, &format!("Chip{k}"))) {
            let _ = m.ShowAt(&b);
        }
    }

    /// Option `i` of chip `k`: its word goes into the box (replacing the chip's word).
    pub(crate) fn search_chip_pick(&mut self, k: usize, i: usize) {
        let Some(&chip) = sq::CHIPS.get(k) else { return };
        let Some((_, pick)) = chip.options(&sq::today()).into_iter().nth(i) else { return };
        let q = self.search.query.clone();
        let (t, at) = match pick {
            sq::Pick::Term(term) => sq::put(&q, chip, &term),
            sq::Pick::Prompt(kw) => sq::put(&q, chip, kw),
            sq::Pick::Modifier(m) => sq::add_modifier(&q, m),
            sq::Pick::Folder => {
                let Some(dir) = crate::dialogs::pick_folders(self.hwnd, false).pop() else { return };
                sq::put(&q, chip, &sq::folder_term(&dir.display().to_string()))
            }
        };
        self.search_set_box(&t, at);
    }

    /// The chips lit from the words in the box.
    fn search_chips_state(&self) {
        let Some(fe) = &self.search.chips else { return };
        let lit = sq::lit(&self.search.query);
        for (k, chip) in sq::CHIPS.iter().enumerate() {
            let label = match &lit[k] {
                Some(v) => format!("{}: {v}   \u{2715}", chip.title()),
                None => chip.title().to_string(),
            };
            text(fe, &format!("ChipText{k}"), &label);
            for name in [format!("Chip{k}"), format!("ChipMenu{k}")] {
                crate::look::paint(fe, &name, crate::look::Pick::Chip, lit[k].is_some());
            }
        }
    }

    /// An example from the help: added to what is typed.
    fn search_example(&mut self, e: &str) {
        if let Ok(f) = find::<FlyoutBase>(&self.root, "SearchHelpFly") {
            let _ = f.Hide();
        }
        let (t, at) = sq::append(&self.search.query, e);
        self.search_set_box(&t, at);
    }

    /// Put `t` in the box with the caret at byte `at`, and search for it.
    pub(crate) fn search_set_box(&mut self, t: &str, at: usize) {
        if let Ok(b) = find::<TextBox>(&self.root, "SearchBox") {
            let _ = b.SetText(&t.into());
            let _ = b.cast::<UIElement>().and_then(|u| u.Focus(FocusState::Programmatic));
            let _ = b.Select(sq::utf16_at(t, at) as i32, 0);
        }
        // the box's own TextChanged may come later (or while the state is busy): not waited for
        self.search_typed_at(t.to_string(), at);
    }

    /// The box changed (its TextChanged), the caret at `caret16` (UTF-16 units).
    pub(crate) fn search_typed(&mut self, t: String, caret16: usize) {
        let at = sq::byte_at(&t, caret16);
        self.search_typed_at(t, at);
    }

    fn search_typed_at(&mut self, t: String, at: usize) {
        self.search_changed(t);
        let exts: Vec<String> = self.exts.iter().take(60).map(|e| e.ext.clone()).collect();
        let s = sq::suggest(&self.search.query, at, &exts);
        if s != self.search.suggest {
            self.search.suggest = s;
            self.search.highlight = None;
            self.search_suggest_render();
        }
    }

    /// The suggestion list under the box (hidden when there is none).
    fn search_suggest_render(&mut self) {
        let Ok(host) = find::<Border>(&self.root, "SearchSuggest") else { return };
        let shown = self.view == View::Search && self.search.suggest.is_some();
        let _ = host.cast::<UIElement>().and_then(|u| u.SetVisibility(if shown { Visibility::Visible } else { Visibility::Collapsed }));
        let Some(s) = self.search.suggest.clone().filter(|_| shown) else { return };
        let mut x = format!(r##"<StackPanel {} Spacing="1">"##, ui::NS);
        for (i, (label, _)) in s.items.iter().enumerate() {
            let bg = if self.search.highlight == Some(i) { "#FF2C3B63" } else { "Transparent" };
            x += &format!(
                r##"<Button x:Name="SxSug{i}" HorizontalAlignment="Stretch" HorizontalContentAlignment="Left" Padding="8,3" FontSize="12" BorderThickness="0" Background="{bg}" IsTabStop="False"><TextBlock Text="{}" FontFamily="Consolas"/></Button>"##,
                ui::esc(label)
            );
        }
        x += r##"<TextBlock Text="Up and Down pick, Enter or Tab takes it, Esc closes" FontSize="11" Foreground="#FF7A8296" Margin="8,2,8,2"/></StackPanel>"##;
        let Ok(content) = load::<UIElement>(&x) else { return };
        let _ = host.SetChild(&content);
        let Ok(fe) = content.cast::<FrameworkElement>() else { return };
        for i in 0..s.items.len() {
            if let Ok(b) = find::<ButtonBase>(&fe, &format!("SxSug{i}")) {
                if let Ok(t) = b.Click(move |_, _| {
                    with(|st| st.search_suggest_take(i));
                }) {
                    keep(t);
                }
            }
        }
    }

    fn search_suggest_close(&mut self) {
        if self.search.suggest.is_some() {
            self.search.suggest = None;
            self.search.highlight = None;
            self.search_suggest_render();
        }
    }

    fn search_suggest_take(&mut self, i: usize) {
        let Some(s) = self.search.suggest.take() else { return };
        self.search.highlight = None;
        self.search_suggest_render();
        if let Some((t, at)) = sq::take(&self.search.query, &s, i) {
            self.search_set_box(&t, at);
        }
    }

    /// A key in the search box, before the box sees it. True when it was used here: the
    /// suggestions never take a key from plain typing.
    pub(crate) fn search_box_key(&mut self, key: i32) -> bool {
        let open = self.search.suggest.as_ref().map_or(0, |s| s.items.len());
        match sq::box_key(key, open, self.search.highlight) {
            sq::BoxKey::Highlight(h) => {
                self.search.highlight = h;
                self.search_suggest_render();
                true
            }
            sq::BoxKey::Take(i) => {
                self.search_suggest_take(i);
                true
            }
            sq::BoxKey::Close => {
                self.search_suggest_close();
                true
            }
            sq::BoxKey::Results => {
                self.search_suggest_close();
                self.search_note();
                self.search_box_down();
                true
            }
            sq::BoxKey::Pass => false,
        }
    }

    /// The hint line and the empty state, for what is typed now.
    pub(crate) fn search_box_state(&mut self) {
        let q = Query::parse(&self.search.query);
        let errors = q.errors();
        let hint = if errors.is_empty() {
            TIP.to_string()
        } else {
            let n = errors.len();
            format!("{} {}: {}", if n == 1 { "This word" } else { "These words" }, "can't be understood, so nothing is searched", errors.join("   ·   "))
        };
        text(&self.root, "SearchHint", &hint);
        self.search_chips_state();
        if let Ok(t) = find::<crate::bindings::Microsoft::UI::Xaml::Controls::TextBlock>(&self.root, "SearchHint") {
            let colour = if errors.is_empty() { "#FF8E97AA" } else { "#FFFFB547" };
            if let Ok(b) = load::<crate::bindings::Microsoft::UI::Xaml::Media::Brush>(&format!(r##"<SolidColorBrush {} Color="{colour}"/>"##, ui::NS)) {
                let _ = t.SetForeground(&b);
            }
        }
        let empty = self.view == View::Search && self.search.query.trim().is_empty();
        if let Ok(e) = find::<UIElement>(&self.root, "SearchEmpty") {
            let _ = e.SetVisibility(if empty { Visibility::Visible } else { Visibility::Collapsed });
        }
        self.search_saved_state();
    }

    /// Keep a history of searches (on unless switched off).
    fn search_keeps_history(&self) -> bool {
        self.settings.bool("searchHistory") != Some(false)
    }

    /// A search was used (Enter to the results, a result opened or shown in the tree): it
    /// goes first among the recent ones, unless the history is off.
    pub(crate) fn search_note(&mut self) {
        if !self.search_keeps_history() || !Query::parse(&self.search.query).errors().is_empty() {
            return;
        }
        let h = self.search.history.get_or_insert_with(sq::history_load);
        if h.note(&self.search.query) {
            sq::history_save(h);
            self.search_saved_state();
        }
    }

    /// The saved and recent searches button at the right of the box: its menu is made again
    /// only when what it lists changes.
    fn search_saved_state(&mut self) {
        if !self.search.ui_built {
            return;
        }
        let Ok(host) = find::<Border>(&self.root, "SearchSaved") else { return };
        let keep_on = self.search_keeps_history();
        let h = self.search.history.get_or_insert_with(sq::history_load).clone();
        let q = self.search.query.trim().to_string();
        let pinned = h.is_pinned(&q);
        let shape = format!("{}|{}|{}|{keep_on}", h.to_text(), q.is_empty(), pinned);
        if shape == self.search.saved_shape {
            return;
        }
        self.search.saved_shape = shape;
        // (label, what it does)
        let mut items: Vec<(String, SavedAct)> = Vec::new();
        let mut x = format!(
            r##"<Button {} x:Name="SxSaved" Height="32" Padding="10,3" ToolTipService.ToolTip="Saved and recent searches"><FontIcon Glyph="&#xE81C;" FontSize="14"/><Button.Flyout><MenuFlyout x:Name="SxSavedFly" Placement="BottomEdgeAlignedRight">"##,
            ui::NS
        );
        let mut item = |x: &mut String, label: String, act: SavedAct, icon: &str, enabled: bool| {
            let i = items.len();
            let icon = if icon.is_empty() { String::new() } else { format!(r##"<MenuFlyoutItem.Icon><FontIcon Glyph="{icon}"/></MenuFlyoutItem.Icon>"##) };
            *x += &format!(r##"<MenuFlyoutItem x:Name="SxSv{i}" Text="{}" IsEnabled="{}">{icon}</MenuFlyoutItem>"##, ui::esc(&label), if enabled { "True" } else { "False" });
            items.push((label, act));
        };
        for p in &h.pinned {
            item(&mut x, p.clone(), SavedAct::Use(p.clone()), "&#xE718;", true);
        }
        if !h.pinned.is_empty() && !h.recent.is_empty() {
            x += "<MenuFlyoutSeparator/>";
        }
        for r in &h.recent {
            item(&mut x, r.clone(), SavedAct::Use(r.clone()), "&#xE81C;", true);
        }
        if h.pinned.is_empty() && h.recent.is_empty() {
            let none = if keep_on { "No saved or recent searches yet" } else { "No saved searches" };
            item(&mut x, none.into(), SavedAct::Nothing, "", false);
        }
        x += "<MenuFlyoutSeparator/>";
        if pinned {
            item(&mut x, "Unpin this search".into(), SavedAct::Unpin(q.clone()), "&#xE77A;", true);
        } else {
            item(&mut x, "Pin this search".into(), SavedAct::Pin(q.clone()), "&#xE718;", !q.is_empty());
        }
        item(&mut x, "Clear recent searches".into(), SavedAct::Clear, "&#xE74D;", !h.recent.is_empty());
        let i = items.len();
        x += &format!(
            r##"<ToggleMenuFlyoutItem x:Name="SxSv{i}" Text="Keep a history of searches" IsChecked="{}"/>"##,
            if keep_on { "True" } else { "False" }
        );
        items.push((String::new(), SavedAct::Keep));
        x += "</MenuFlyout></Button.Flyout></Button>";
        let Ok(content) = load::<UIElement>(&x) else { return };
        let _ = host.SetChild(&content);
        let Ok(fe) = content.cast::<FrameworkElement>() else { return };
        for (i, (_, act)) in items.into_iter().enumerate() {
            if let Ok(m) = find::<MenuFlyoutItem>(&fe, &format!("SxSv{i}")) {
                if let Ok(t) = m.Click(move |_, _| {
                    let act = act.clone();
                    with(|s| s.search_saved_pick(act));
                }) {
                    keep(t);
                }
            }
        }
    }

    /// Open the saved searches menu (a dev step for screenshots).
    pub(crate) fn search_saved_open(&mut self) {
        eprintln!("saved searches: {:?}", self.search.history);
        let Ok(host) = find::<Border>(&self.root, "SearchSaved") else { return };
        let Some(fe) = host.Child().ok().and_then(|c| c.cast::<FrameworkElement>().ok()) else { return };
        if let (Ok(m), Ok(b)) = (find::<MenuFlyout>(&fe, "SxSavedFly"), find::<FrameworkElement>(&fe, "SxSaved")) {
            let _ = m.ShowAt(&b);
        }
    }

    fn search_saved_pick(&mut self, act: SavedAct) {
        let h = self.search.history.get_or_insert_with(sq::history_load);
        match act {
            SavedAct::Use(q) => {
                let at = q.len();
                self.search_set_box(&q, at);
                return;
            }
            SavedAct::Pin(q) => h.pin(&q),
            SavedAct::Unpin(q) => h.unpin(&q),
            SavedAct::Clear => h.recent.clear(),
            SavedAct::Keep => {
                let on = !self.search_keeps_history();
                self.settings.set_bool("searchHistory", on);
                self.settings.save();
                // switched off: what was kept goes (pinned searches stay)
                if !on {
                    let h = self.search.history.get_or_insert_with(sq::history_load);
                    h.recent.clear();
                }
            }
            SavedAct::Nothing => return,
        }
        if let Some(h) = &self.search.history {
            sq::history_save(h);
        }
        self.search_saved_state();
    }
}

/// What an item of the saved searches menu does.
#[derive(Clone)]
enum SavedAct {
    Use(String),
    Pin(String),
    Unpin(String),
    Clear,
    Keep,
    Nothing,
}

use windows_core::Interface;
