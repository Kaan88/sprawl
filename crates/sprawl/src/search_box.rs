//! The Search box's surroundings in the window: the hint line under it (what the box could not
//! understand), the "?" help and the empty state, built from [`crate::search_query`]. Every
//! control here writes into the box or reads from it; the box always holds the real query.

use sprawl_core::query::Query;

use crate::bindings::Microsoft::UI::Xaml::Controls::Primitives::{ButtonBase, FlyoutBase};
use crate::bindings::Microsoft::UI::Xaml::Controls::{Border, TextBox};
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
        self.search_box_state();
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
        self.search_changed(t.to_string());
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
    }
}

use windows_core::Interface;
