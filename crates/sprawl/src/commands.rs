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
const TOGGLES: &[&str] = &["oAuto", "oHeader", "oTypes", "oTreemap", "oAlloc", "oNames", "oTilt", "oCushion", "oFree", "oMirror", "oPulses", "oAlwaysAdmin"];
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
        for (name, on) in [("TreeScroll", v != View::About), ("TreeHeader", v != View::About)] {
            if let Ok(e) = find::<UIElement>(&self.root, name) {
                let _ = e.SetVisibility(vis(on));
            }
        }
        for (name, me) in [("TabTree", View::Tree), ("TabFiles", View::Files), ("TabAbout", View::About)] {
            if let Ok(b) = find::<crate::bindings::Microsoft::UI::Xaml::Controls::Control>(&self.root, name) {
                let brush: windows_core::Result<crate::bindings::Microsoft::UI::Xaml::Media::Brush> =
                    load(&format!(r##"<SolidColorBrush {} Color="{}"/>"##, ui::NS, if me == v { "#FF2C3550" } else { "Transparent" }));
                if let Ok(br) = brush {
                    let _ = b.SetBackground(&br);
                }
            }
        }
        if v != View::About {
            self.refresh_rows();
        }
    }

    pub(crate) fn selected_path(&self) -> Option<PathBuf> {
        Some(self.tree.as_ref()?.path_of(self.selected?))
    }

    pub(crate) fn command(&mut self, name: &str) {
        let has_tree = self.tree.is_some();
        match name {
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
            "mWizard" => self.wizard_open(),
            "ModeHist" | "mHistory" => self.hist_toggle(),
            "mTweaks" => {
                if let Ok(p) = find::<UIElement>(&self.root, "Tweaks") {
                    let shown = p.Visibility().map(|v| v == Visibility::Visible).unwrap_or(false);
                    let _ = p.SetVisibility(if shown { Visibility::Collapsed } else { Visibility::Visible });
                }
            }
            "mAdmin" => {
                let arg = self.tree.as_ref().map(|t| dialogs::quote(&t.root_path)).unwrap_or_default();
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
                if let Some(p) = self.selected_path() {
                    win32::copy_text(self.hwnd, &p.display().to_string());
                }
            }
            "mCopyInfo" if has_tree => {
                let t = self.tree.as_ref().unwrap();
                let mut s = String::from("Name\tSize\tAllocated\tItems\tFiles\r\n");
                for &(n, _) in &self.rows {
                    let node = &t.nodes[n];
                    s += &format!("{}\t{}\t{}\t{}\t{}\r\n", t.path_of(n).display(), self.size(node.size), self.size(self.alloc[n]), node.items(), node.files);
                }
                win32::copy_text(self.hwnd, &s);
                text(&self.root, "FootLeft", "Copied the visible rows to the clipboard");
            }
            "mDelete" | "mDeleteForever" => self.delete(name == "mDeleteForever"),
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
            "mExit" => std::process::exit(0),
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
                    _ => {
                        for n in ["MapBorder", "MapPath"] {
                            if let Ok(e) = find::<UIElement>(&self.root, n) {
                                let _ = e.SetVisibility(vis);
                            }
                        }
                        if let Ok(r) = find::<RowDefinition>(&self.root, "MapRow") {
                            let _ = r.SetHeight(GridLength { Value: if on { 1.0 } else { 0.0 }, GridUnitType: GridUnitType::Star });
                        }
                    }
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
            "oPulses" => {
                self.pulses_on = find::<ToggleMenuFlyoutItem>(&self.root, name).and_then(|t| t.IsChecked()).unwrap_or(true);
                self.watch();
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
        if let Some(v) = self.settings.get("view").filter(|v| matches!(*v, "ModeMap" | "ModeCity" | "ModeGalaxy")).map(str::to_string) {
            self.command(&v);
        }
        if let Ok(x) = find::<ToggleMenuFlyoutItem>(&self.root, "oExplorer") {
            dialogs::migrate_explorer_menu();
            let _ = x.SetIsChecked(dialogs::explorer_menu_installed());
        }
    }

    /// A slider, switch or list in the Colors and Tweaks panel changed.
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

    pub(crate) fn delete(&mut self, permanent: bool) {
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
        if !win32::delete_path(self.hwnd, &path, permanent) {
            win32::info(self.hwnd, "It couldn't be deleted (it may be in use or protected).", "Sprawl");
            return;
        }
        // The full tree behind a filter forgets it too, or clearing the filter brings it back.
        if let Some(full) = self.full_tree.as_mut() {
            if let Some(k) = sprawl_advisor::find_path(full, &path) {
                detach(full, k, None);
            }
        }
        let t = self.tree.as_mut().unwrap();
        let parent = t.nodes[n].parent;
        detach(t, n, Some(&mut self.alloc));
        self.selected = parent;
        self.files.retain(|&f| f != n && !self.tree.as_ref().unwrap().nodes[f].is_dir);
        self.rebuild_exts();
        self.refresh_rows();
        self.refresh_exts();
        self.render_map();
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
