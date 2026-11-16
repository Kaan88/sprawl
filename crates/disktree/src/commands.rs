//! Menu items and buttons, routed here by their x:Name.

use std::collections::HashSet;
use std::path::PathBuf;

use disktree_core::ROOT;

use crate::bindings::Microsoft::UI::Xaml::Controls::Primitives::Selector;
use crate::bindings::Microsoft::UI::Xaml::Controls::{ColumnDefinition, RowDefinition, ToggleMenuFlyoutItem};
use crate::bindings::Microsoft::UI::Xaml::{GridLength, GridUnitType, UIElement, Visibility};
use crate::format::{file_type, fmt_time, stamp, user_dir};
use crate::state::{find, load, text, State, Units, View};
use crate::{ui, win32};

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
                if let Some(p) = win32::pick_folder(self.hwnd) {
                    self.start_scan(PathBuf::from(p));
                }
            }
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
                self.map.capture = Some(user_dir("Pictures").join(format!("DiskTree treemap {}.png", stamp())));
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
                let t = self.tree.as_ref().unwrap();
                let mut s = String::from("File Name,Size,Allocated,Modified,Attributes,Files,Folders\r\n");
                for (i, n) in t.nodes.iter().enumerate() {
                    let mut name = t.path_of(i).display().to_string();
                    if n.is_dir && !name.ends_with('\\') {
                        name.push('\\');
                    }
                    s += &format!("\"{}\",{},{},{},{},{},{}\r\n", name.replace('"', "\"\""), n.size, self.alloc[i], n.modified.map(fmt_time).unwrap_or_default(), if n.is_dir { "D" } else { "" }, n.files, n.folders);
                }
                self.save_text("DiskTree export", &s);
            }
            "mExportTypes" if has_tree => {
                let mut s = String::from("Extension,File Type,Percent,Size,Allocated,Files\r\n");
                for e in &self.exts {
                    s += &format!("\"{}\",\"{}\",{:.1},{},{},{}\r\n", e.ext, file_type(&e.ext), e.percent, e.size, e.alloc, e.files);
                }
                self.save_text("DiskTree file types", &s);
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
            "oAlloc" | "oCushion" | "oNames" | "oTilt" => {
                let on = find::<ToggleMenuFlyoutItem>(&self.root, name).and_then(|t| t.IsChecked()).unwrap_or(true);
                match name {
                    "oAlloc" => self.map.by_alloc = on,
                    "oCushion" => self.map.scene.cushion = if on { 1.0 } else { 0.0 },
                    "oNames" => self.map.show_labels = on,
                    _ => self.map.scene.tilt_on = on,
                }
                self.render_map();
            }
            "TabTree" => self.set_view(View::Tree),
            "TabFiles" => self.set_view(View::Files),
            "TabAbout" => self.set_view(View::About),
            _ => {}
        }
    }

    pub(crate) fn save_text(&self, base: &str, s: &str) {
        let path = user_dir("Documents").join(format!("{base} {}.csv", stamp()));
        let msg = match std::fs::write(&path, s) {
            Ok(()) => format!("Saved {}", path.display()),
            Err(e) => format!("Couldn't save: {e}"),
        };
        text(&self.root, "FootLeft", &msg);
    }

    pub(crate) fn delete(&mut self, permanent: bool) {
        let Some(n) = self.selected.filter(|&n| n != ROOT) else { return };
        let Some(path) = self.selected_path() else { return };
        let what = if permanent { "Delete this permanently? It can't be brought back." } else { "Move this to the Recycle Bin?" };
        if !win32::confirm(self.hwnd, &format!("{what}\n\n{}", path.display()), "DiskTree") {
            return;
        }
        if !win32::delete_path(self.hwnd, &path, permanent) {
            win32::info(self.hwnd, "It couldn't be deleted (it may be in use or protected).", "DiskTree");
            return;
        }
        let t = self.tree.as_mut().unwrap();
        let (size, files, folders, is_dir, alloc) = (t.nodes[n].size, t.nodes[n].files, t.nodes[n].folders, t.nodes[n].is_dir, self.alloc[n]);
        let parent = t.nodes[n].parent;
        if let Some(p) = parent {
            t.nodes[p].children.retain(|&c| c != n);
        }
        let mut p = parent;
        while let Some(i) = p {
            let node = &mut t.nodes[i];
            node.size -= size;
            node.files -= files;
            node.folders -= folders + u64::from(is_dir);
            self.alloc[i] -= alloc;
            p = node.parent;
        }
        // Detach the whole branch so file-type totals forget it.
        let mut stack = vec![n];
        while let Some(i) = stack.pop() {
            stack.extend(t.nodes[i].children.clone());
            t.nodes[i].is_dir = true;
            t.nodes[i].size = 0;
            t.nodes[i].children.clear();
            self.alloc[i] = 0;
        }
        self.selected = parent;
        self.files.retain(|&f| f != n && !self.tree.as_ref().unwrap().nodes[f].is_dir);
        self.rebuild_exts();
        self.refresh_rows();
        self.refresh_exts();
        self.render_map();
    }
}
