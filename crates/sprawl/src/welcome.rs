//! What the window shows before anything is scanned: the drives, with how full they are and
//! when each was last scanned, the folders scanned recently, and a way to pick a folder.

use std::path::PathBuf;
use std::time::SystemTime;

use windows_core::Interface;

use crate::bindings::Microsoft::UI::Xaml::Controls::Border;
use crate::bindings::Microsoft::UI::Xaml::Controls::Primitives::ButtonBase;
use crate::bindings::Microsoft::UI::Xaml::{FrameworkElement, UIElement, Visibility};
use crate::state::{find, keep, load, with, State};
use crate::ui;

/// The welcome overlay over the map, and the hints in the two empty tables.
pub(crate) fn xaml() -> &'static str {
    r##"<Border x:Name="Welcome" Visibility="Collapsed"/>"##
}

pub(crate) fn tree_hint() -> &'static str {
    r##"<TextBlock x:Name="TreeHint" Grid.Row="1" Visibility="Collapsed" IsHitTestVisible="False" HorizontalAlignment="Center" VerticalAlignment="Center" TextAlignment="Center" FontSize="13" Foreground="#FF6E7789" Text="Folders show up here, biggest first.&#x0a;Click one to find it on the map below."/>"##
}

pub(crate) fn types_hint() -> &'static str {
    r##"<TextBlock x:Name="TypesHint" Grid.Row="1" Visibility="Collapsed" IsHitTestVisible="False" HorizontalAlignment="Center" VerticalAlignment="Center" TextAlignment="Center" FontSize="13" Foreground="#FF6E7789" Text="File types show up here.&#x0a;Click one to light up its files."/>"##
}

/// Roots of what was scanned before, newest first: (root, when, bytes).
fn recent() -> Vec<(PathBuf, SystemTime, u64)> {
    let Some(base) = sprawl_advisor::history::base_dir() else { return Vec::new() };
    let mut all = sprawl_advisor::history::list(&base);
    all.sort_by(|a, b| b.taken.cmp(&a.taken));
    let mut out: Vec<(PathBuf, SystemTime, u64)> = Vec::new();
    for e in all {
        let key = e.root.to_string_lossy().to_lowercase();
        if !out.iter().any(|(r, _, _)| r.to_string_lossy().to_lowercase() == key) {
            out.push((e.root, e.taken, e.bytes));
        }
    }
    out
}

fn ago(t: SystemTime) -> String {
    let s = t.elapsed().map(|d| d.as_secs()).unwrap_or(0);
    match s {
        0..=5399 => "within the hour".into(),
        5400..=86_399 => format!("{} hours ago", (s + 1800) / 3600),
        86_400..=172_799 => "yesterday".into(),
        _ => format!("{} days ago", (s + 43_200) / 86_400),
    }
}

/// Which drive a path is on, as "C:\".
fn drive_of(p: &std::path::Path) -> String {
    let s = p.to_string_lossy();
    s.get(..3).unwrap_or("").to_uppercase()
}

impl State {
    /// Shows the welcome while nothing is scanned or scanning, and hides it after.
    pub(crate) fn welcome_frame(&mut self) {
        let want = self.tree.is_none() && self.job.is_none();
        if self.welcome == Some(want) {
            return;
        }
        self.welcome = Some(want);
        if want {
            self.welcome_build();
            // the header's usage bar is sized before the layout knows its width: size it again
            if let Some(d) = self.drives.first().cloned() {
                self.update_header(std::path::Path::new(&d));
            }
        }
        for n in ["Welcome", "TreeHint", "TypesHint"] {
            if let Ok(e) = find::<UIElement>(&self.root, n) {
                let _ = e.SetVisibility(if want { Visibility::Visible } else { Visibility::Collapsed });
            }
        }
    }

    fn welcome_build(&mut self) {
        let recent = recent();
        let drives = crate::win32::drives();
        let mut x = format!(r##"<ScrollViewer {} HorizontalScrollBarVisibility="Disabled"><StackPanel Spacing="18" Padding="28,24,28,24" MaxWidth="1100" HorizontalAlignment="Left">"##, ui::NS);
        x += r##"<StackPanel Spacing="4"><TextBlock Text="Where did the space go?" FontSize="24" FontWeight="SemiBold"/><TextBlock Text="Pick a drive or a folder: Sprawl shows what uses the space, as a tree, a table of file types and a map you can fly through." FontSize="13" Foreground="#FFB9C2D6" TextWrapping="Wrap"/></StackPanel>"##;

        // the drives, as cards
        x += r##"<VariableSizedWrapGrid Orientation="Horizontal" ItemWidth="260" ItemHeight="128">"##;
        for (i, d) in drives.iter().enumerate().take(12) {
            let (total, free) = crate::win32::disk_space(std::path::Path::new(&d.root)).unwrap_or((0, 0));
            let used = total.saturating_sub(free);
            let pct = if total == 0 { 0.0 } else { used as f64 / total as f64 };
            let colour = if pct > 0.9 { "#FFE5534B" } else if pct > 0.75 { "#FFE8A33D" } else { "#FF4C8DF6" };
            let last = recent.iter().find(|(r, _, _)| r.to_string_lossy().len() <= 3 && drive_of(r) == d.root.to_uppercase());
            let last = match last {
                Some((_, t, b)) => format!("Last scanned {}: {}", ago(*t), self.size(*b)),
                None => "Not scanned yet".into(),
            };
            let letter = d.root.trim_end_matches('\\');
            let name = if d.label.is_empty() { letter.to_string() } else { format!("{} ({letter})", d.label) };
            x += &format!(
                r##"<Button x:Name="WelD{i}" Margin="0,0,12,12" Padding="16,12" HorizontalAlignment="Stretch" VerticalAlignment="Stretch" HorizontalContentAlignment="Stretch" VerticalContentAlignment="Top" CornerRadius="10" Background="#14FFFFFF" BorderBrush="#22FFFFFF" ToolTipService.ToolTip="Scan {root}">
<StackPanel Spacing="6">
<StackPanel Orientation="Horizontal" Spacing="10"><FontIcon Glyph="&#xEDA2;" FontSize="18" Foreground="{colour}"/><TextBlock Text="{name}" FontSize="15" FontWeight="SemiBold" TextTrimming="CharacterEllipsis" MaxWidth="180"/></StackPanel>
<Grid Height="6" CornerRadius="3" Background="#22FFFFFF"><Border HorizontalAlignment="Left" Width="{w:.0}" CornerRadius="3" Background="{colour}"/></Grid>
<TextBlock FontSize="12" Foreground="#FFD5DBE8" Text="{free} free of {total}"/>
<TextBlock FontSize="11" Foreground="#FF8E97AA" Text="{last}" TextTrimming="CharacterEllipsis"/>
</StackPanel></Button>"##,
                root = ui::esc(&d.root),
                name = ui::esc(&name),
                w = (pct * 226.0).max(6.0),
                free = self.size(free),
                total = self.size(total),
                last = ui::esc(&last),
            );
        }
        x += r##"<Button x:Name="WelFolder" Margin="0,0,12,12" Padding="16,12" HorizontalAlignment="Stretch" VerticalAlignment="Stretch" HorizontalContentAlignment="Left" VerticalContentAlignment="Top" CornerRadius="10" Background="Transparent" BorderBrush="#33FFFFFF" BorderThickness="1">
<StackPanel Spacing="6"><StackPanel Orientation="Horizontal" Spacing="10"><FontIcon Glyph="&#xE8B7;" FontSize="18" Foreground="#FFB9C2D6"/><TextBlock Text="A folder..." FontSize="15" FontWeight="SemiBold"/></StackPanel>
<TextBlock FontSize="12" Foreground="#FF8E97AA" TextWrapping="Wrap" Text="Any folder, or several at once (Shift+F3). A WizTree or Sprawl CSV opens with Ctrl+O."/></StackPanel></Button>"##;
        x += "</VariableSizedWrapGrid>";

        // folders scanned before (drives are on their cards)
        let folders: Vec<&(PathBuf, SystemTime, u64)> = recent.iter().filter(|(r, _, _)| r.to_string_lossy().len() > 3 && r.exists()).take(5).collect();
        self.welcome_recent = folders.iter().map(|(r, _, _)| r.clone()).collect();
        if !folders.is_empty() {
            x += r##"<StackPanel Spacing="2"><TextBlock Text="Scanned before" FontSize="13" FontWeight="SemiBold" Margin="0,0,0,4"/>"##;
            for (i, (r, t, b)) in folders.iter().enumerate() {
                x += &format!(
                    r##"<Button x:Name="WelR{i}" Padding="10,5" HorizontalAlignment="Left" Background="Transparent" BorderThickness="0" CornerRadius="5"><StackPanel Orientation="Horizontal" Spacing="12"><FontIcon Glyph="&#xE8B7;" FontSize="13" Foreground="#FF8E97AA"/><TextBlock Text="{}" FontSize="12" Width="520" TextTrimming="CharacterEllipsis"/><TextBlock Text="{}" FontSize="12" Width="80" TextAlignment="Right" Foreground="#FFD5DBE8"/><TextBlock Text="{}" FontSize="12" Foreground="#FF8E97AA"/></StackPanel></Button>"##,
                    ui::esc(&r.display().to_string()),
                    self.size(*b),
                    ago(*t)
                );
            }
            x += "</StackPanel>";
        }

        // the fast path, when it isn't on yet
        if !sprawl_core::is_elevated() {
            x += r##"<StackPanel Orientation="Horizontal" Spacing="10"><FontIcon Glyph="&#xE945;" FontSize="14" Foreground="#FFE8A33D"/><TextBlock VerticalAlignment="Center" FontSize="12" Foreground="#FFB9C2D6" TextWrapping="Wrap" MaxWidth="640" Text="As administrator, Sprawl reads a whole NTFS drive's file table directly: a full drive in seconds instead of minutes."/><Button x:Name="WelAdmin" Content="Restart as administrator" Padding="10,3" FontSize="12" CornerRadius="5"/></StackPanel>"##;
        }
        x += "</StackPanel></ScrollViewer>";

        let Ok(host) = find::<Border>(&self.root, "Welcome") else { return };
        let Ok(el) = load::<UIElement>(&x) else {
            eprintln!("welcome failed to load");
            return;
        };
        let _ = host.SetChild(&el);
        let Ok(fe) = el.cast::<FrameworkElement>() else { return };
        let mut names: Vec<String> = (0..drives.len().min(12)).map(|i| format!("WelD{i}")).collect();
        names.extend((0..folders.len()).map(|i| format!("WelR{i}")));
        names.extend(["WelFolder".to_string(), "WelAdmin".to_string()]);
        self.welcome_drives = drives.into_iter().map(|d| d.root).collect();
        for n in names {
            if let Ok(b) = find::<ButtonBase>(&fe, &n) {
                if let Ok(t) = b.Click(move |_, _| {
                    let n = n.clone();
                    with(move |s| s.welcome_command(&n));
                }) {
                    keep(t);
                }
            }
        }
    }

    fn welcome_command(&mut self, name: &str) {
        if let Some(i) = name.strip_prefix("WelD").and_then(|i| i.parse::<usize>().ok()) {
            if let Some(d) = self.welcome_drives.get(i).cloned() {
                self.start_scan(PathBuf::from(d));
            }
        } else if let Some(i) = name.strip_prefix("WelR").and_then(|i| i.parse::<usize>().ok()) {
            if let Some(r) = self.welcome_recent.get(i).cloned() {
                self.start_scan(r);
            }
        } else if name == "WelFolder" {
            self.command("mSelectFolder");
        } else if name == "WelAdmin" {
            self.command("mAdmin");
        }
    }
}
