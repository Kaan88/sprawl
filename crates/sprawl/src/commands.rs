//! Menu items and buttons, routed here by their x:Name.

use std::collections::HashSet;
use std::path::PathBuf;

use sprawl_core::ROOT;

use crate::bindings::Microsoft::UI::Xaml::Controls::Primitives::Selector;
use crate::bindings::Microsoft::UI::Xaml::Controls::Primitives::RangeBase;
use crate::bindings::Microsoft::UI::Xaml::Controls::{ColumnDefinition, RowDefinition, ToggleMenuFlyoutItem, ToggleSwitch};
use crate::bindings::Microsoft::UI::Xaml::{FocusState, GridLength, GridUnitType, UIElement, Visibility};
use crate::format::{file_type, stamp, user_dir};
use crate::state::{find, load, text, State, Units, View};

/// Options menu switches that are remembered between runs.
const TOGGLES: &[&str] = &["oAuto", "oHeader", "oTypes", "oTreemap", "oAlloc", "oNames", "oTilt", "oCushion", "oFree", "oMirror", "oPulses", "oSearchDrives", "oAlwaysAdmin", "oUpdates"];
const UNITS: &[&str] = &["uDyn", "uB", "uKB", "uMB", "uGB", "uTB"];
use crate::state::Source;
use crate::{dialogs, paint, ui, win32};

impl State {
    // ------------------------------------------------------------ commands

    pub(crate) fn set_view(&mut self, v: View) {
        self.view = v;
        let vis = |b: bool| if b { Visibility::Visible } else { Visibility::Collapsed };
        if let Ok(a) = find::<UIElement>(&self.root, "AboutPanel") {
            let _ = a.SetVisibility(vis(v == View::About));
        }
        if let Ok(a) = find::<UIElement>(&self.root, "AdvisorPanel") {
            let _ = a.SetVisibility(vis(v == View::Advisor));
        }
        let table = matches!(v, View::Tree | View::Files | View::Search);
        for (name, on) in [("TreeScroll", table), ("TreeHeader", table), ("SearchBar", v == View::Search), ("SearchMore", false), ("SearchEmpty", false), ("SearchSuggest", false)] {
            if let Ok(e) = find::<UIElement>(&self.root, name) {
                let _ = e.SetVisibility(vis(on));
            }
        }
        self.paint_tabs();
        // the Advisor takes the whole width; the types pane comes back as the user left it
        let types = v != View::Advisor && find::<ToggleMenuFlyoutItem>(&self.root, "oTypes").and_then(|t| t.IsChecked()).unwrap_or(true);
        if let Ok(e) = find::<UIElement>(&self.root, "TypesPanel") {
            let _ = e.SetVisibility(vis(types));
        }
        if let Ok(c) = find::<ColumnDefinition>(&self.root, "TypesCol") {
            let _ = c.SetWidth(GridLength { Value: if types { 1.0 } else { 0.0 }, GridUnitType: GridUnitType::Star });
        }
        // and the whole height: the map folds away while the Advisor or Cleaners is open
        self.map_room();
        if self.welcome == Some(true) {
            crate::welcome::tree_hint_shown(self, matches!(v, View::Tree | View::Files));
        }
        if table {
            self.refresh_rows();
        }
        if v == View::Advisor {
            self.render_advisor();
        }
        self.map.drawn_once = false;
    }

    /// The map and its bar: shown when the Treemap option is on, folded away while the Advisor
    /// or Cleaners tab is open (their lists take the height).
    pub(crate) fn map_room(&self) {
        let map_on = find::<ToggleMenuFlyoutItem>(&self.root, "oTreemap").and_then(|t| t.IsChecked()).unwrap_or(true);
        let shown = map_on && self.view != View::Advisor;
        // the bar (the map's modes) stays when only the map is switched off, as before
        for (n, on) in [("MapBorder", shown), ("MapPath", map_on), ("MapBar", self.view != View::Advisor)] {
            if let Ok(e) = find::<UIElement>(&self.root, n) {
                let _ = e.SetVisibility(if on { Visibility::Visible } else { Visibility::Collapsed });
            }
        }
        if let Ok(r) = find::<RowDefinition>(&self.root, "MapRow") {
            let _ = r.SetHeight(GridLength { Value: if shown { 1.25 } else { 0.0 }, GridUnitType: GridUnitType::Star });
        }
    }

    /// The lit tab: Cleaners is the Advisor showing only its Cleaners section.
    pub(crate) fn paint_tabs(&self) {
        let v = self.view;
        let cl = self.cleaners_tab();
        for (name, me) in [("TabTree", View::Tree), ("TabFiles", View::Files), ("TabSearch", View::Search), ("TabAdvisor", View::Advisor), ("TabCleaners", View::Advisor), ("TabAbout", View::About)] {
            let on = me == v && (me != View::Advisor || (name == "TabCleaners") == cl);
            if let Ok(b) = find::<crate::bindings::Microsoft::UI::Xaml::Controls::Control>(&self.root, name) {
                let brush: windows_core::Result<crate::bindings::Microsoft::UI::Xaml::Media::Brush> =
                    load(&format!(r##"<SolidColorBrush {} Color="{}"/>"##, ui::NS, if on { "#FF2C3550" } else { "Transparent" }));
                if let Ok(br) = brush {
                    let _ = b.SetBackground(&br);
                }
            }
        }
    }

    pub(crate) fn selected_path(&self) -> Option<PathBuf> {
        Some(self.tree.as_ref()?.path_of(self.selected?))
    }

    pub(crate) fn command(&mut self, name: &str) {
        let has_tree = self.tree.is_some();
        match name {
            // while scanning the button is Stop
            "ScanButton" if self.job.is_some() => self.stop_scan(),
            // F5 and File > Scan: the folder on screen again (the button scans what the box says)
            "mScan" if self.tree.as_ref().is_some_and(|t| t.method != sprawl_core::ScanMethod::Csv && t.nodes[ROOT].name != sprawl_core::MULTIPLE) => {
                if let Some(root) = self.tree.as_ref().map(|t| t.path_of(ROOT)) {
                    self.start_scan(root);
                }
            }
            "mScan" | "ScanButton" => {
                let idx = find::<Selector>(&self.root, "DriveBox").and_then(|c| c.SelectedIndex()).unwrap_or(0);
                if let Some(d) = self.drives.get(idx.max(0) as usize) {
                    self.start_scan(PathBuf::from(d));
                }
            }
            "mSelectFolder" => {
                if let Some(p) = dialogs::pick_folders(self.hwnd, false).pop() {
                    self.start_scan(p);
                }
            }
            "mSelectMultiple" => {
                let v = dialogs::pick_folders(self.hwnd, true);
                if !v.is_empty() {
                    self.start(Source::Folders(v));
                }
            }
            "mSelectCsv" => {
                if let Some(p) = dialogs::pick_csv(self.hwnd) {
                    self.start(Source::Csv(p));
                }
            }
            "mStop" => self.stop_scan(),
            "mReselectChild" if has_tree => {
                if let Some(c) = self.child_stack.pop().filter(|&c| c < self.tree.as_ref().unwrap().nodes.len()) {
                    self.reveal(c);
                }
            }
            "mFilter" => {
                if let Ok(b) = find::<UIElement>(&self.root, "FilterBox") {
                    let _ = b.Focus(FocusState::Programmatic);
                }
            }
            "mWizard" => {
                self.wizard_all = false;
                self.wizard_open();
            }
            "mSettings" => self.settings_open(),
            "mAbout" => self.command("TabAbout"),
            "mUpdate" => self.update_check(true),
            "UpdateNow" | "UpdateLater" | "UpdateSkip" => self.update_command(name),
            "mShortcuts" => {
                let mut s = String::from("Tabs: Ctrl+1 Tree View, Ctrl+2 File View, Ctrl+3 Advisor, Ctrl+5 Cleaners, Ctrl+F Search\nTables: arrow keys, Page Up/Down, Home/End; type a name to jump to it; Ctrl+click and Shift+click pick several\nMap: click selects, double-click goes in, wheel zooms, right-drag pans\n\n");
                for (k, what) in crate::ui::shortcuts() {
                    s += &format!("{k}\t{what}\n");
                }
                win32::info(self.hwnd, &s, "Keyboard shortcuts");
            }
            "ModeHist" | "mHistory" => self.hist_toggle(),
            "mTweaks" => {
                if let Ok(p) = find::<UIElement>(&self.root, "Tweaks") {
                    let shown = p.Visibility().map(|v| v == Visibility::Visible).unwrap_or(false);
                    let _ = p.SetVisibility(if shown { Visibility::Collapsed } else { Visibility::Visible });
                }
            }
            "mAdmin" if self.advisor.cleanup.running.is_some() => text(&self.root, "FootLeft", "A clean-up is running: restart as administrator when it has finished."),
            "mAdmin" => {
                let mut arg = self.tree.as_ref().map(|t| dialogs::quote(&t.root_path)).unwrap_or_default();
                // from the Advisor: the new copy opens it again with the same ticks
                if self.view == View::Advisor && !arg.is_empty() {
                    arg += " --advisor";
                    for k in self.advisor.cleanup.ticked.iter().take(500) {
                        arg += &format!(" --tick {}", dialogs::quote(std::path::Path::new(k)));
                    }
                }
                if sprawl_core::is_elevated() {
                    text(&self.root, "FootLeft", "Sprawl is already running as administrator.");
                } else if dialogs::restart_elevated(self.hwnd, &arg) {
                    std::process::exit(0);
                }
            }
            "oExplorer" => {
                let on = find::<ToggleMenuFlyoutItem>(&self.root, name).and_then(|t| t.IsChecked()).unwrap_or(false);
                let msg = match (dialogs::set_explorer_menu(on), on) {
                    (true, true) => "Added \"Scan with Sprawl\" to the right-click menu of folders and drives.",
                    (true, false) => "Removed \"Scan with Sprawl\" from Explorer.",
                    (false, _) => "Couldn't change Explorer's menu.",
                };
                text(&self.root, "FootLeft", msg);
            }
            "oAlwaysAdmin" => {}
            "mNewInstance" => {
                if let Ok(exe) = std::env::current_exe() {
                    let _ = std::process::Command::new(exe).spawn();
                }
            }
            "mReset" if has_tree => {
                self.expanded = HashSet::from([ROOT]);
                self.refresh_rows();
            }
            "mExpandAll" if has_tree => {
                let t = self.tree.as_ref().unwrap();
                let base = self.selected.unwrap_or(ROOT);
                let mut stack = vec![base];
                while let Some(n) = stack.pop() {
                    if t.nodes[n].is_dir {
                        self.expanded.insert(n);
                        stack.extend(&t.nodes[n].children);
                    }
                }
                self.refresh_rows();
            }
            "mParent" if has_tree => {
                if let Some(p) = self.selected.and_then(|s| self.tree.as_ref().unwrap().nodes[s].parent) {
                    if let Some(s) = self.selected {
                        self.child_stack.push(s);
                    }
                    self.reveal(p);
                }
            }
            "mZoomIn" if has_tree => {
                let t = self.tree.as_ref().unwrap();
                let mut n = self.selected.unwrap_or(ROOT);
                if !t.nodes[n].is_dir {
                    n = t.nodes[n].parent.unwrap_or(ROOT);
                }
                self.map_root = n;
                self.render_map();
            }
            "mZoomOut" if has_tree => {
                if let Some(p) = self.tree.as_ref().unwrap().nodes[self.map_root].parent {
                    self.map_root = p;
                    self.render_map();
                }
            }
            "mSavePng" if has_tree => {
                // The next frame is read back from the GPU and written as a PNG.
                self.map.capture = Some(user_dir("Pictures").join(format!("Sprawl treemap {}.png", stamp())));
            }
            "mOpen" => {
                if let Some(p) = self.selected_path() {
                    let _ = std::process::Command::new("explorer.exe").arg(&p).spawn();
                }
            }
            "mInTree" if self.view == View::Search => {
                if let Some(n) = self.selected {
                    self.search_reveal(n);
                }
            }
            "mMapHere" if has_tree => {
                if let Some(n) = self.selected {
                    let t = self.tree.as_ref().unwrap();
                    self.map_root = if t.nodes[n].is_dir { n } else { t.nodes[n].parent.unwrap_or(ROOT) };
                    self.render_map();
                }
            }
            "mExplore" => {
                if let Some(p) = self.selected_path() {
                    let _ = std::process::Command::new("explorer.exe").arg(format!("/select,{}", p.display())).spawn();
                }
            }
            "mCmd" => {
                if let Some(mut p) = self.selected_path() {
                    if !p.is_dir() {
                        p.pop();
                    }
                    use std::os::windows::process::CommandExt;
                    let _ = std::process::Command::new("cmd.exe").arg("/K").current_dir(p).creation_flags(0x10).spawn();
                }
            }
            "mCopyPath" => {
                // in Search: every picked row of the scan, in the order shown; else a row from
                // another drive that is picked
                let picked = if self.view == View::Search { crate::search::picked_in_order(&self.rows, &self.marked, self.selected) } else { Vec::new() };
                if let (Some(t), true) = (&self.tree, picked.len() > 1) {
                    let all: Vec<String> = picked.iter().map(|&n| t.path_of(n).display().to_string()).collect();
                    win32::copy_text(self.hwnd, &all.join("\r\n"));
                    text(&self.root, "SearchInfo", &format!("{} paths copied.", all.len()));
                } else if let Some(p) = self.selected_path() {
                    win32::copy_text(self.hwnd, &p.display().to_string());
                } else if let (View::Search, Some((k, id))) = (self.view, self.search.more.sel) {
                    self.more_command('C', k, id);
                }
            }
            "mCopyInfo" if has_tree => {
                let t = self.tree.as_ref().unwrap();
                let mut s = String::from("Name\tSize\tAllocated\tItems\tFiles\r\n");
                for &(n, d) in &self.rows {
                    if !self.scan_row(&(n, d)) {
                        continue;
                    }
                    let node = &t.nodes[n];
                    s += &format!("{}\t{}\t{}\t{}\t{}\r\n", t.path_of(n).display(), self.size(node.size), self.size(self.alloc[n]), node.items(), node.files);
                }
                win32::copy_text(self.hwnd, &s);
                text(&self.root, "FootLeft", "Copied the visible rows to the clipboard");
            }
            // only from the tables: the Advisor removes things only through its own review
            "mDelete" | "mDeleteForever" if matches!(self.view, View::Tree | View::Files | View::Search) => self.delete(name == "mDeleteForever"),
            "mDelete" | "mDeleteForever" => text(&self.root, "FootLeft", "Delete works in Tree View, File View and Search (on rows of this scan)"),
            "mExportCsv" if has_tree => {
                // WizTree's format, so the file loads back here or in WizTree.
                let path = user_dir("Documents").join(format!("Sprawl export {}.csv", stamp()));
                let res = std::fs::File::create(&path).and_then(|f| {
                    let mut w = std::io::BufWriter::new(f);
                    sprawl_core::export_csv(self.tree.as_ref().unwrap(), &mut w)?;
                    std::io::Write::flush(&mut w)
                });
                let msg = match res {
                    Ok(()) => format!("Saved {}", path.display()),
                    Err(e) => format!("Couldn't save: {e}"),
                };
                text(&self.root, "FootLeft", &msg);
            }
            "mExportTypes" if has_tree => {
                let mut s = String::from("Extension,File Type,Percent,Size,Allocated,Files\r\n");
                for e in &self.exts {
                    s += &format!("\"{}\",\"{}\",{:.1},{},{},{}\r\n", e.ext, file_type(&e.ext), e.percent, e.size, e.alloc, e.files);
                }
                self.save_text("Sprawl file types", &s);
            }
            "mExit" => {
                if self.advisor.cleanup.running.is_some() && !win32::confirm(self.hwnd, "A clean-up is still running. Close Sprawl anyway?

What is already deleted stays deleted; the rest is left as it was.", "Sprawl") {
                    return;
                }
                if let Some(p) = win32::window_place(self.hwnd) {
                    self.settings.set("window", p);
                    self.settings.save();
                }
                std::process::exit(0);
            }
            "uDyn" | "uB" | "uKB" | "uMB" | "uGB" | "uTB" => {
                self.units = match name {
                    "uB" => Units::Bytes,
                    "uKB" => Units::KB,
                    "uMB" => Units::MB,
                    "uGB" => Units::GB,
                    "uTB" => Units::TB,
                    _ => Units::Dynamic,
                };
                if has_tree {
                    self.refresh_rows();
                    self.refresh_exts();
                    let p = self.tree.as_ref().unwrap().root_path.clone();
                    self.update_header(&p);
                }
            }
            "oHeader" | "oTypes" | "oTreemap" => {
                let on = find::<ToggleMenuFlyoutItem>(&self.root, name).and_then(|t| t.IsChecked()).unwrap_or(true);
                let vis = if on { Visibility::Visible } else { Visibility::Collapsed };
                match name {
                    "oHeader" => {
                        if let Ok(e) = find::<UIElement>(&self.root, "Header") {
                            let _ = e.SetVisibility(vis);
                        }
                    }
                    "oTypes" => {
                        if let Ok(e) = find::<UIElement>(&self.root, "TypesPanel") {
                            let _ = e.SetVisibility(vis);
                        }
                        if let Ok(c) = find::<ColumnDefinition>(&self.root, "TypesCol") {
                            let _ = c.SetWidth(GridLength { Value: if on { 1.0 } else { 0.0 }, GridUnitType: GridUnitType::Star });
                        }
                    }
                    _ => self.map_room(),
                }
            }
            "oAlloc" | "oCushion" | "oNames" | "oTilt" | "oFree" | "oMirror" => {
                let on = find::<ToggleMenuFlyoutItem>(&self.root, name).and_then(|t| t.IsChecked()).unwrap_or(true);
                match name {
                    "oAlloc" => self.map.by_alloc = on,
                    "oFree" => self.map.show_free = on,
                    "oMirror" => {
                        self.map.mirror = on;
                        self.map.scene.mirror = on;
                    }
                    "oCushion" => self.map.scene.cushion = if on { self.map.cushion_strength } else { 0.0 },
                    "oNames" => self.map.show_labels = on,
                    _ => self.map.scene.tilt_on = on,
                }
                self.render_map();
            }
            "aaOff" | "aaLow" | "aaMedium" | "aaHigh" => {
                self.map.aa = match name {
                    "aaOff" => 1,
                    "aaLow" => 2,
                    "aaHigh" => 8,
                    _ => 4,
                };
                if let Some(g) = &mut self.map.gpu {
                    if let Err(e) = g.set_samples(self.map.aa) {
                        eprintln!("antialiasing: {e}");
                    }
                }
                self.render_map();
                self.settings.set("aa", name);
                self.settings.save();
            }
            "oSearchDrives" => {
                let on = find::<ToggleMenuFlyoutItem>(&self.root, name).and_then(|t| t.IsChecked()).unwrap_or(true);
                self.settings.set_bool(name, on);
                if on && self.view == View::Search {
                    self.more_open();
                }
            }
            "oPulses" => {
                self.pulses_on = find::<ToggleMenuFlyoutItem>(&self.root, name).and_then(|t| t.IsChecked()).unwrap_or(true);
                self.watch();
            }
            v if v.starts_with("View") && crate::views::ALL.iter().any(|a| a.1 == v) => {
                let k = crate::views::ALL.iter().find(|a| a.1 == v).map(|a| a.0);
                self.set_view_kind(k);
            }
            p if p.starts_with("Paint") && crate::views::PAINTS.iter().any(|a| a.1 == p) => {
                if let Some(x) = crate::views::PAINTS.iter().find(|a| a.1 == p) {
                    self.set_paint(x.0);
                }
            }
            "ViewOnly" => self.views_only_toggle(),
            "SetupYes" | "SetupNo" => {
                if let Ok(e) = find::<UIElement>(&self.root, "SetupOffer") {
                    let _ = e.SetVisibility(crate::bindings::Microsoft::UI::Xaml::Visibility::Collapsed);
                }
                if name == "SetupYes" {
                    self.wizard_all = false;
                    self.wizard_open();
                } else {
                    self.settings.set_bool("wizard_done", true);
                    self.settings.save();
                    text(&self.root, "FootLeft", "You can set things up any time under Options > Settings.");
                }
            }
            "ModeMap" | "ModeCity" | "ModeGalaxy" => {
                let mode = match name {
                    "ModeCity" => crate::scene::Mode::City,
                    "ModeGalaxy" => crate::scene::Mode::Galaxy,
                    _ => crate::scene::Mode::Map,
                };
                self.set_mode(mode);
                self.settings.set("view", name);
                self.settings.save();
            }
            "TabTree" => self.set_view(View::Tree),
            "TabFiles" => self.set_view(View::Files),
            "TabSearch" | "mSearch" => self.search_focus(),
            "SearchToFilter" => self.search_to_filter(),
            d if d.len() == 4 && d.starts_with("SxD") => self.more_drive(d.chars().nth(3).unwrap_or('C')),
            "TabAdvisor" => {
                self.set_cleaners_tab(false);
                self.set_view(View::Advisor);
            }
            "TabCleaners" => {
                self.set_cleaners_tab(true);
                self.set_view(View::Advisor);
                // no scan that covers the cleaners: the tab reads their folders itself
                if self.own_active() && self.advisor.cl.own_reading.is_none() {
                    self.own_on_open();
                    self.render_advisor();
                }
            }
            "TabAbout" => self.set_view(View::About),
            _ => {}
        }
        if name.starts_with('o') || name.starts_with('u') {
            self.save_settings();
        }
    }

    // ------------------------------------------------------------ settings

    pub(crate) fn save_settings(&mut self) {
        for &t in TOGGLES {
            if let Ok(on) = find::<ToggleMenuFlyoutItem>(&self.root, t).and_then(|x| x.IsChecked()) {
                self.settings.set_bool(t, on);
            }
        }
        if let Some(u) = UNITS.iter().find(|u| find::<ToggleMenuFlyoutItem>(&self.root, u).and_then(|x| x.IsChecked()).unwrap_or(false)) {
            self.settings.set("units", u);
        }
        self.settings.set("palette", paint::palette());
        self.settings.set("lift", self.map.scene.lift_scale);
        self.settings.set("cushion", self.map.cushion_strength);
        self.settings.set("tilt", self.map.scene.tilt_amount);
        self.settings.set_bool("animate", self.map.scene.animate);
        self.settings.save();
    }

    /// Put the saved options back (at start-up).
    pub(crate) fn apply_settings(&mut self) {
        let s = &self.settings;
        let lift = s.num("lift").unwrap_or(1.0) as f32;
        let cushion = s.num("cushion").unwrap_or(1.0) as f32;
        let tilt = s.num("tilt").unwrap_or(0.44) as f32;
        let animate = s.bool("animate").unwrap_or(true);
        let palette = s.num("palette").unwrap_or(0.0) as usize;
        let units = s.get("units").map(str::to_string);
        let toggles: Vec<(&str, bool)> = TOGGLES.iter().filter_map(|&t| Some((t, s.bool(t)?))).collect();
        self.map.scene.lift_scale = lift;
        self.map.cushion_strength = cushion;
        self.map.scene.cushion = cushion;
        self.map.scene.tilt_amount = tilt;
        self.map.scene.animate = animate;
        paint::set_palette(palette);
        let set_range = |name: &str, v: f64| {
            if let Ok(r) = find::<RangeBase>(&self.root, name) {
                let _ = r.SetValue(v);
            }
        };
        set_range("LiftSlider", lift as f64 * 100.0);
        set_range("CushionSlider", cushion as f64 * 100.0);
        set_range("TiltSlider", tilt as f64 / 0.8 * 100.0);
        if let Ok(t) = find::<ToggleSwitch>(&self.root, "AnimSwitch") {
            let _ = t.SetIsOn(animate);
        }
        if let Ok(b) = find::<Selector>(&self.root, "PaletteBox") {
            let _ = b.SetSelectedIndex(palette as i32);
        }
        for (t, on) in toggles {
            if let Ok(x) = find::<ToggleMenuFlyoutItem>(&self.root, t) {
                if x.IsChecked().unwrap_or(on) != on {
                    let _ = x.SetIsChecked(on);
                    self.command(t);
                }
            }
        }
        if let Some(u) = units.filter(|u| UNITS.contains(&u.as_str())) {
            if let Ok(x) = find::<ToggleMenuFlyoutItem>(&self.root, &u) {
                let _ = x.SetIsChecked(true);
            }
            self.command(&u);
        }
        if let Some(a) = self.settings.get("aa").map(str::to_string).filter(|a| ["aaOff", "aaLow", "aaMedium", "aaHigh"].contains(&a.as_str())) {
            if let Ok(x) = find::<ToggleMenuFlyoutItem>(&self.root, &a) {
                let _ = x.SetIsChecked(true);
            }
            self.command(&a);
        }
        // only a view name: the settings file must never be able to run another command
        if let Some(v) = self.settings.get("view").filter(|v| matches!(*v, "ModeMap" | "ModeCity" | "ModeGalaxy") || crate::views::ALL.iter().any(|a| a.1 == *v)).map(str::to_string) {
            self.command(&v);
        }
        if let Some(p) = self.settings.get("paint").filter(|p| crate::views::PAINTS.iter().any(|a| a.1 == *p)).map(str::to_string) {
            self.command(&p);
        }
        if let Ok(x) = find::<ToggleMenuFlyoutItem>(&self.root, "oExplorer") {
            dialogs::migrate_explorer_menu();
            let _ = x.SetIsChecked(dialogs::explorer_menu_installed());
        }
    }

    /// A slider, switch or list in the Colours and Tweaks panel changed.
    pub(crate) fn tweak(&mut self, name: &str, v: f64) {
        match name {
            "LiftSlider" => self.map.scene.lift_scale = (v / 100.0) as f32,
            "CushionSlider" => {
                self.map.cushion_strength = (v / 100.0) as f32;
                let on = find::<ToggleMenuFlyoutItem>(&self.root, "oCushion").and_then(|t| t.IsChecked()).unwrap_or(true);
                self.map.scene.cushion = if on { self.map.cushion_strength } else { 0.0 };
            }
            "TiltSlider" => self.map.scene.tilt_amount = (v / 100.0 * 0.8) as f32,
            "AnimSwitch" => self.map.scene.animate = v > 0.5,
            "PaletteBox" => {
                paint::set_palette(v as usize);
                // Colours are baked into the tiles and the tables.
                self.map.scene.clear();
                self.refresh_rows();
                self.refresh_exts();
            }
            _ => return,
        }
        self.render_map();
        self.save_settings();
    }

    pub(crate) fn save_text(&self, base: &str, s: &str) {
        let path = user_dir("Documents").join(format!("{base} {}.csv", stamp()));
        let msg = match std::fs::write(&path, s) {
            Ok(()) => format!("Saved {}", path.display()),
            Err(e) => format!("Couldn't save: {e}"),
        };
        text(&self.root, "FootLeft", &msg);
    }

    /// Why the selected item may not be deleted from here, if it may not.
    fn delete_refusal(&self, n: usize, path: &std::path::Path) -> Option<String> {
        let t = self.tree.as_ref()?;
        if t.method == sprawl_core::ScanMethod::Csv {
            return Some("This view was loaded from a CSV file, so it may not match the disk. Scan the folder to delete from it.".into());
        }
        if t.nodes[ROOT].name == sprawl_core::MULTIPLE && t.nodes[n].parent == Some(ROOT) {
            return Some("That is one of the scanned folders itself.".into());
        }
        if self.full_tree.is_some() && t.nodes[n].is_dir {
            return Some("A filter is on, so this folder holds more than you can see. Clear the filter to delete a whole folder.".into());
        }
        let s = path.display().to_string();
        let bad_seg = s.split('\\').skip(1).any(|g| g == "." || g == ".." || g.ends_with('.') || g.ends_with(' '));
        if !path.is_absolute() || s.contains(['*', '?']) || bad_seg {
            return Some(format!("Sprawl won't delete a path written like this:\n{s}"));
        }
        let never = sprawl_advisor::safety::Never::default_list();
        if !never.complete() {
            return Some("Sprawl couldn't work out where Windows keeps its own folders, so it won't delete anything.".into());
        }
        if let Some(p) = never.hit(&s) {
            return Some(format!("{s}\n\nis protected (it matches {p}). Deleting it could break Windows, an app or your profile."));
        }
        if let Some((at, p)) = never.hit_inside(t, n) {
            return Some(format!("This folder holds\n{at}\n\nwhich is protected ({p}). Delete the things around it one by one instead."));
        }
        match std::fs::symlink_metadata(path) {
            Ok(m) if m.is_dir() == t.nodes[n].is_dir => None,
            // a junction or folder link: deleting it removes the link, not what it points to
            Ok(m) if m.file_type().is_symlink() && t.nodes[n].is_reparse_point() => None,
            Ok(_) => Some("It changed since the scan (a file became a folder or the other way round). Scan again first.".into()),
            Err(_) => Some("It's no longer there. Scan again to refresh.".into()),
        }
    }

    /// The picked rows (Ctrl/Shift) as top-most nodes: one inside another goes with it.
    pub(crate) fn picked_nodes(&self) -> Vec<usize> {
        let Some(t) = &self.tree else { return Vec::new() };
        let mut all: std::collections::BTreeSet<usize> = self.marked.clone();
        all.extend(self.selected);
        all.remove(&ROOT);
        all.iter()
            .copied()
            .filter(|&n| n < t.nodes.len())
            .filter(|&n| {
                let mut p = t.nodes[n].parent;
                while let Some(q) = p {
                    if all.contains(&q) {
                        return false;
                    }
                    p = t.nodes[q].parent;
                }
                true
            })
            .collect()
    }

    pub(crate) fn delete(&mut self, permanent: bool) {
        if self.marked.len() > 1 || (self.marked.len() == 1 && self.selected.is_some_and(|s| !self.marked.contains(&s))) {
            return self.delete_many(permanent);
        }
        let Some(n) = self.selected.filter(|&n| n != ROOT) else { return };
        let Some(path) = self.selected_path() else { return };
        if let Some(why) = self.delete_refusal(n, &path) {
            win32::info(self.hwnd, &why, "Sprawl");
            return;
        }
        let node = &self.tree.as_ref().unwrap().nodes[n];
        let what = match (node.is_dir, permanent) {
            (true, true) => format!("Delete this folder permanently? It can't be brought back.\n\n{} files in {} folders, {}", node.files, node.folders, self.size(node.size)),
            (true, false) => format!("Move this folder to the Recycle Bin?\n\n{} files in {} folders, {}", node.files, node.folders, self.size(node.size)),
            (false, true) => format!("Delete this file permanently? It can't be brought back.\n\n{}", self.size(node.size)),
            (false, false) => format!("Move this file to the Recycle Bin?\n\n{}", self.size(node.size)),
        };
        if !win32::confirm(self.hwnd, &format!("{what}\n\n{}", path.display()), "Sprawl") {
            return;
        }
        self.own_deletes.push((path.to_string_lossy().trim_end_matches('\\').to_lowercase(), std::time::Instant::now()));
        if !win32::delete_path(self.hwnd, &path, permanent) {
            win32::info(self.hwnd, "It couldn't be deleted (it may be in use or protected).", "Sprawl");
            return;
        }
        // The full tree behind a filter forgets it too, or clearing the filter brings it back.
        self.forget_in_full(&[path.clone()]);
        let t = std::sync::Arc::make_mut(self.tree.as_mut().unwrap());
        let parent = t.nodes[n].parent;
        detach(t, n, Some(&mut self.alloc));
        self.tree_gen += 1;
        // its pictures are of the tree before the delete
        self.hist_close();
        self.selected = parent;
        self.files.retain(|&f| f != n && !self.tree.as_ref().unwrap().nodes[f].is_dir);
        self.rebuild_exts();
        self.refresh_rows();
        self.refresh_exts();
        self.render_map();
        // what was found there may be gone, and the totals changed
        self.advisor_start();
    }

    /// Delete with several rows picked: every one is checked first, then one confirm for all.
    fn delete_many(&mut self, permanent: bool) {
        let nodes = self.picked_nodes();
        let Some(t) = self.tree.clone() else { return };
        if nodes.is_empty() {
            return;
        }
        for &n in &nodes {
            let path = t.path_of(n);
            if let Some(why) = self.delete_refusal(n, &path) {
                win32::info(self.hwnd, &format!("{why}\n\n{}\n\nNothing was deleted.", path.display()), "Sprawl");
                return;
            }
        }
        let total: u64 = nodes.iter().map(|&n| t.nodes[n].size).sum();
        let list: String = nodes.iter().take(12).map(|&n| format!("{}\n", t.path_of(n).display())).collect();
        let more = if nodes.len() > 12 { format!("...and {} more\n", nodes.len() - 12) } else { String::new() };
        let what = if permanent {
            format!("Delete these {} items permanently? They can't be brought back.\n\n{}", nodes.len(), self.size(total))
        } else {
            format!("Move these {} items to the Recycle Bin?\n\n{}", nodes.len(), self.size(total))
        };
        if !win32::confirm(self.hwnd, &format!("{what}\n\n{list}{more}"), "Sprawl") {
            return;
        }
        let mut failed = Vec::new();
        let mut gone = Vec::new();
        for &n in &nodes {
            let path = t.path_of(n);
            self.own_deletes.push((path.to_string_lossy().trim_end_matches('\\').to_lowercase(), std::time::Instant::now()));
            if win32::delete_path(self.hwnd, &path, permanent) {
                gone.push((n, path));
            } else {
                failed.push(path);
            }
        }
        self.forget_in_full(&gone.iter().map(|(_, p)| p.clone()).collect::<Vec<_>>());
        for (n, _) in &gone {
            let tm = std::sync::Arc::make_mut(self.tree.as_mut().unwrap());
            detach(tm, *n, Some(&mut self.alloc));
        }
        if !gone.is_empty() {
            self.tree_gen += 1;
            self.hist_close();
            let tr = self.tree.as_ref().unwrap();
            self.selected = gone.first().and_then(|(n, _)| tr.nodes[*n].parent);
            self.files.retain(|&f| !tr.nodes[f].is_dir);
        }
        self.marked.clear();
        self.rebuild_exts();
        self.refresh_rows();
        self.refresh_exts();
        self.render_map();
        if !failed.is_empty() {
            let list: String = failed.iter().take(8).map(|p| format!("{}\n", p.display())).collect();
            win32::info(self.hwnd, &format!("{} of {} couldn't be deleted (in use or protected):\n\n{list}", failed.len(), nodes.len()), "Sprawl");
        }
        if !gone.is_empty() {
            self.advisor_start();
        }
    }
}

/// Take node `n` out of the tree after it was deleted: its totals come off every parent and the
/// whole branch is emptied so file-type totals forget it.
fn detach(t: &mut sprawl_core::Tree, n: usize, mut alloc: Option<&mut Vec<u64>>) {
    let (size, files, folders, is_dir) = (t.nodes[n].size, t.nodes[n].files, t.nodes[n].folders, t.nodes[n].is_dir);
    let own_alloc = alloc.as_ref().map_or(t.nodes[n].allocated, |a| a[n]);
    let parent = t.nodes[n].parent;
    if let Some(p) = parent {
        t.nodes[p].children.retain(|&c| c != n);
    }
    let mut p = parent;
    while let Some(i) = p {
        let node = &mut t.nodes[i];
        node.size = node.size.saturating_sub(size);
        node.files = node.files.saturating_sub(files);
        node.folders = node.folders.saturating_sub(folders + u64::from(is_dir));
        node.allocated = node.allocated.saturating_sub(own_alloc);
        if let Some(a) = alloc.as_deref_mut() {
            a[i] = a[i].saturating_sub(own_alloc);
        }
        p = node.parent;
    }
    let mut stack = vec![n];
    while let Some(i) = stack.pop() {
        stack.extend(t.nodes[i].children.clone());
        t.nodes[i].is_dir = true;
        t.nodes[i].size = 0;
        t.nodes[i].allocated = 0;
        t.nodes[i].children.clear();
        if let Some(a) = alloc.as_deref_mut() {
            a[i] = 0;
        }
    }
}

/// Put `sub` (a fresh scan of node `n`'s path) in place of what the tree had below `n`: the old
/// branch is taken out, the new one added at the end, and the totals of every parent follow.
pub(crate) fn replace_branch(t: &mut sprawl_core::Tree, n: usize, sub: &sprawl_core::Tree, mut alloc: Option<&mut Vec<u64>>) {
    for c in t.nodes[n].children.clone() {
        detach(t, c, alloc.as_deref_mut());
    }
    let new = &sub.nodes[sprawl_core::ROOT];
    // what is left on n now (its own size, for a file) against what the scan found
    let old_alloc = alloc.as_ref().map_or(t.nodes[n].allocated, |a| a[n]);
    let d_size = new.size as i128 - t.nodes[n].size as i128;
    let d_alloc = new.allocated as i128 - old_alloc as i128;
    let d_files = new.files as i128 - t.nodes[n].files as i128;
    let d_folders = new.folders as i128 - t.nodes[n].folders as i128;
    let add = |v: u64, d: i128| (v as i128 + d).max(0) as u64;
    let mut p = Some(n);
    while let Some(i) = p {
        let node = &mut t.nodes[i];
        node.size = add(node.size, d_size);
        node.allocated = add(node.allocated, d_alloc);
        node.files = add(node.files, d_files);
        node.folders = add(node.folders, d_folders);
        if let Some(a) = alloc.as_deref_mut() {
            a[i] = add(a[i], d_alloc);
        }
        p = node.parent;
    }
    t.nodes[n].is_dir = new.is_dir;
    t.nodes[n].modified = new.modified;
    t.nodes[n].attributes = new.attributes;
    // the scan's nodes after its root, renumbered to follow the tree's own
    let base = t.nodes.len();
    let map = |i: usize| if i == sprawl_core::ROOT { n } else { base + i - 1 };
    for node in &sub.nodes[1..] {
        let mut k = node.clone();
        k.parent = k.parent.map(map);
        k.children = k.children.iter().map(|&c| map(c)).collect();
        if let Some(a) = alloc.as_deref_mut() {
            a.push(k.allocated);
        }
        t.nodes.push(k);
    }
    t.nodes[n].children = new.children.iter().map(|&c| map(c)).collect();
}

/// In a filtered view, the files below `k` that a fresh scan of it (`sub`) no longer has are
/// taken out, so the rows don't show what a clean-up removed.
fn forget_missing(t: &mut sprawl_core::Tree, k: usize, sub: &sprawl_core::Tree, alloc: &mut Vec<u64>) {
    // every path below the fresh scan's root, relative to it, in lower case
    let mut there = std::collections::HashSet::new();
    let mut stack = vec![(ROOT, String::new())];
    while let Some((i, rel)) = stack.pop() {
        for &c in &sub.nodes[i].children {
            let r = format!("{rel}\\{}", sub.nodes[c].name.to_lowercase());
            stack.push((c, r.clone()));
            there.insert(r);
        }
    }
    let mut gone = Vec::new();
    let mut stack = vec![(k, String::new())];
    while let Some((i, rel)) = stack.pop() {
        for &c in &t.nodes[i].children {
            let r = format!("{rel}\\{}", t.nodes[c].name.to_lowercase());
            if !there.contains(&r) {
                gone.push(c);
            } else {
                stack.push((c, r));
            }
        }
    }
    for g in gone {
        detach(t, g, Some(alloc));
    }
}

/// Still in the tree: every step up is among its parent's children (a detached branch isn't).
pub(crate) fn alive(t: &sprawl_core::Tree, mut n: usize) -> bool {
    while let Some(p) = t.nodes.get(n).and_then(|x| x.parent) {
        if !t.nodes[p].children.contains(&n) {
            return false;
        }
        n = p;
    }
    n == sprawl_core::ROOT
}

impl State {
    /// Deleted paths leave the full tree behind a filter too, and its kept parts follow (as
    /// the tables of the tree on screen do after a delete).
    fn forget_in_full(&mut self, paths: &[PathBuf]) {
        let Some(full) = self.full_tree.as_mut().map(std::sync::Arc::make_mut) else { return };
        let mut parts = self.full_parts.as_mut();
        let mut any = false;
        for path in paths {
            if let Some(k) = sprawl_advisor::find_path(full, path).filter(|&k| k != ROOT) {
                detach(full, k, parts.as_mut().map(|p| &mut p.alloc));
                any = true;
            }
        }
        if let (true, Some(p)) = (any, parts) {
            p.files.retain(|&f| !full.nodes[f].is_dir);
            p.exts = crate::tables::ext_rows(full, &p.alloc);
            crate::tables::sort_ext_rows(&mut p.exts, p.sorts.exts);
        }
    }

    /// After a clean-up: each path as it is now (a fresh scan of it, or None when it is gone) goes
    /// into the tree, and the tables, the map and the findings follow.
    pub(crate) fn apply_rescans(&mut self, now: Vec<(PathBuf, Option<sprawl_core::Tree>)>) {
        if self.tree.is_none() || now.is_empty() {
            return;
        }
        let filtered = self.full_tree.is_some();
        let mut changed = false;
        for (path, sub) in &now {
            if let Some(full) = self.full_tree.as_mut().map(std::sync::Arc::make_mut) {
                let alloc = self.full_parts.as_mut().map(|p| &mut p.alloc);
                if let Some(k) = sprawl_advisor::find_path(full, path).filter(|&k| k != ROOT) {
                    changed = true;
                    match sub {
                        Some(s) => replace_branch(full, k, s, alloc),
                        None => detach(full, k, alloc),
                    }
                }
            }
            if filtered {
                // the filtered view on screen loses what is gone at once; it is made again
                // from the full tree below
                let t = std::sync::Arc::make_mut(self.tree.as_mut().unwrap());
                if let Some(k) = sprawl_advisor::find_path(t, path).filter(|&k| k != ROOT) {
                    match sub {
                        None => detach(t, k, Some(&mut self.alloc)),
                        Some(s) => forget_missing(t, k, s, &mut self.alloc),
                    }
                }
                continue;
            }
            let t = std::sync::Arc::make_mut(self.tree.as_mut().unwrap());
            if let Some(k) = sprawl_advisor::find_path(t, path) {
                changed = true;
                match sub {
                    Some(s) => replace_branch(t, k, s, Some(&mut self.alloc)),
                    None if k != ROOT => detach(t, k, Some(&mut self.alloc)),
                    None => {}
                }
            }
        }
        if !changed {
            return;
        }
        if filtered {
            // the full tree's parts follow it; the filtered view is made again from it
            if let (Some(full), Some(p)) = (self.full_tree.as_mut().map(std::sync::Arc::make_mut), self.full_parts.as_mut()) {
                p.redo(full);
            }
            self.tree_gen += 1;
            self.files.retain(|&f| !self.tree.as_ref().unwrap().nodes[f].is_dir);
            self.rebuild_exts();
            self.sort_exts();
            self.refresh_rows();
            self.refresh_exts();
            self.render_map();
            self.filter_again(true);
            return;
        }
        self.sort_tree();
        let t = self.tree.as_ref().unwrap();
        // what was selected or zoomed into may be gone: its nearest part still there
        let up = |mut n: usize| {
            while !alive(t, n) {
                n = t.nodes[n].parent.unwrap_or(ROOT);
            }
            n
        };
        self.selected = self.selected.map(up);
        self.map_root = up(self.map_root);
        self.expanded.retain(|&n| alive(t, n));
        let files = sprawl_core::largest_files(t, usize::MAX);
        let root = t.path_of(ROOT);
        self.files = files;
        self.tree_gen += 1;
        self.hist_close();
        self.sort_files();
        self.rebuild_exts();
        self.sort_exts();
        self.refresh_rows();
        self.refresh_exts();
        self.render_map();
        self.update_header(&root);
        if let Some(t) = self.tree.as_ref() {
            let n = &t.nodes[ROOT];
            let info = format!("{} files, {} folders, {} unreadable", crate::format::thousands(n.files), crate::format::thousands(n.folders), crate::format::thousands(t.errors));
            text(&self.root, "ScanInfo", &info);
        }
        self.advisor_start();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_filtered_view_forgets_what_a_clean_up_removed() {
        let base = std::env::temp_dir().join(format!("sprawl-forget-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(base.join("cache").join("sub")).unwrap();
        std::fs::write(base.join("cache").join("a.dll"), vec![0u8; 100]).unwrap();
        std::fs::write(base.join("cache").join("sub").join("b.dll"), vec![0u8; 50]).unwrap();
        std::fs::write(base.join("cache").join("sub").join("c.dll"), vec![0u8; 20]).unwrap();
        let full = sprawl_core::scan(&base, |_| {}).unwrap();
        // the view on screen: filtered to *.dll
        let mut shown = sprawl_core::filter_tree(&full, "*.dll");
        let mut alloc: Vec<u64> = shown.nodes.iter().map(|n| n.allocated).collect();
        // the clean-up took b.dll; the folder is scanned again
        std::fs::remove_file(base.join("cache").join("sub").join("b.dll")).unwrap();
        let sub = sprawl_core::scan(&base.join("cache"), |_| {}).unwrap();
        let k = sprawl_advisor::find_path(&shown, &base.join("cache")).unwrap();
        forget_missing(&mut shown, k, &sub, &mut alloc);
        assert!(sprawl_advisor::find_path(&shown, &base.join("cache").join("sub").join("b.dll")).is_none());
        assert!(sprawl_advisor::find_path(&shown, &base.join("cache").join("sub").join("c.dll")).is_some());
        assert!(sprawl_advisor::find_path(&shown, &base.join("cache").join("a.dll")).is_some());
        assert_eq!(shown.nodes[ROOT].size, 120);
        assert_eq!(shown.nodes[ROOT].files, 2);
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn a_branch_is_replaced_and_totals_follow() {
        let base = std::env::temp_dir().join(format!("sprawl-splice-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(base.join("cache").join("sub")).unwrap();
        std::fs::create_dir_all(base.join("other")).unwrap();
        std::fs::write(base.join("cache").join("a.bin"), vec![0u8; 1000]).unwrap();
        std::fs::write(base.join("cache").join("sub").join("b.bin"), vec![0u8; 500]).unwrap();
        std::fs::write(base.join("other").join("c.bin"), vec![0u8; 300]).unwrap();
        let mut t = sprawl_core::scan(&base, |_| {}).unwrap();
        let mut alloc: Vec<u64> = t.nodes.iter().map(|n| n.allocated).collect();
        assert_eq!(t.nodes[ROOT].size, 1800);
        // the cache is cleaned to one small file
        std::fs::remove_dir_all(base.join("cache").join("sub")).unwrap();
        std::fs::write(base.join("cache").join("a.bin"), vec![0u8; 10]).unwrap();
        let sub = sprawl_core::scan(&base.join("cache"), |_| {}).unwrap();
        let k = sprawl_advisor::find_path(&t, &base.join("cache")).unwrap();
        replace_branch(&mut t, k, &sub, Some(&mut alloc));
        assert_eq!(t.nodes[ROOT].size, 310);
        assert_eq!(t.nodes[ROOT].files, 2);
        assert_eq!(t.nodes[k].folders, 0);
        assert_eq!(alloc.len(), t.nodes.len());
        let a = sprawl_advisor::find_path(&t, &base.join("cache").join("a.bin")).unwrap();
        assert_eq!((t.nodes[a].size, t.nodes[a].parent), (10, Some(k)));
        assert!(sprawl_advisor::find_path(&t, &base.join("cache").join("sub")).is_none());
        // and a path that is gone comes out
        let o = sprawl_advisor::find_path(&t, &base.join("other")).unwrap();
        detach(&mut t, o, Some(&mut alloc));
        assert_eq!(t.nodes[ROOT].size, 10);
        assert!(!alive(&t, o) && alive(&t, a));
        let _ = std::fs::remove_dir_all(&base);
    }
}
