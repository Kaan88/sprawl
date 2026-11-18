//! Sprawl's own question and message boxes, in place of `MessageBoxW`: a card over the window
//! (the `DlgHost` overlay), in the app's look. A question's default is always the safe answer:
//! "No" has the focus, Esc and Enter on it say no, and clicking outside the card does nothing.
//!
//! They don't block: what happens on "Yes" is a callback, run later on the UI thread. Anything
//! that acts on the scan must check again in the callback that the scan is still the one the
//! question was about (see `commands.rs` `delete`).

use windows_core::Interface;

use crate::bindings::Microsoft::UI::Xaml::Controls::{Border, Control};
use crate::bindings::Microsoft::UI::Xaml::Controls::Primitives::ButtonBase;
use crate::bindings::Microsoft::UI::Xaml::{FocusState, UIElement, Visibility};
use crate::state::{find, keep, load, with, State};
use crate::ui;

/// What runs on "Yes".
pub(crate) type Then = Box<dyn FnOnce(&mut State)>;

pub(crate) struct Pending {
    title: String,
    text: String,
    /// The "Yes" button's words; `None` for a message with only "OK".
    yes: Option<String>,
    /// The "Yes" button in the warning colour (deleting for good).
    danger: bool,
    then: Option<Then>,
    /// A body laid out in XAML instead of `text` (the keyboard shortcuts), shown wider.
    rich: Option<String>,
}

/// The questions waiting: the first is on screen, the rest follow in turn.
#[derive(Default)]
pub(crate) struct Dialogs {
    queue: std::collections::VecDeque<Pending>,
    /// The keyboard is on "Yes" (it starts on "No" for every question).
    on_yes: bool,
    /// The window was in front at the last frame: when it comes to the front again, the focus
    /// goes back to the question (a focus given while the window was behind doesn't hold, and
    /// without it no key reaches the window at all).
    front: bool,
}

impl Dialogs {
    pub(crate) fn open(&self) -> bool {
        // (also: menu commands and shortcuts wait while a question is open, see State::command)
        !self.queue.is_empty()
    }
}

impl State {
    /// Ask a yes-or-no question; `then` runs only on "Yes". "No" is the default.
    pub(crate) fn ask(&mut self, title: &str, text: &str, yes: &str, danger: bool, then: impl FnOnce(&mut State) + 'static) {
        self.dialog_push(Pending { title: title.into(), text: text.into(), yes: Some(yes.into()), danger, then: Some(Box::new(then)), rich: None });
    }

    /// A message with an OK button.
    pub(crate) fn tell(&mut self, title: &str, text: &str) {
        self.dialog_push(Pending { title: title.into(), text: text.into(), yes: None, danger: false, then: None, rich: None });
    }

    /// A message laid out in XAML (`body`: elements, the namespaces are on the card).
    pub(crate) fn tell_rich(&mut self, title: &str, body: String) {
        self.dialog_push(Pending { title: title.into(), text: String::new(), yes: None, danger: false, then: None, rich: Some(body) });
    }

    fn dialog_push(&mut self, p: Pending) {
        self.dialogs.queue.push_back(p);
        if self.dialogs.queue.len() == 1 {
            self.dialog_show();
        }
    }

    fn dialog_show(&mut self) {
        let Ok(host) = find::<Border>(&self.root, "DlgHost") else { return };
        let Some(p) = self.dialogs.queue.front() else {
            let _ = host.SetVisibility(Visibility::Collapsed);
            let _ = host.SetChild(None::<&UIElement>);
            return;
        };
        let body: String = match &p.rich {
            Some(x) => x.clone(),
            None => p.text.split('\n').map(|l| format!(r##"<TextBlock Text="{}" TextWrapping="Wrap" FontSize="13" Foreground="#FFD5DBE8" IsTextSelectionEnabled="True"/>"##, ui::esc(l))).collect(),
        };
        let (max_w, max_h) = if p.rich.is_some() { (940, 720) } else { (560, 420) };
        let buttons = match &p.yes {
            Some(yes) => format!(
                r##"<Button x:Name="DlgYes" Content="{}" Padding="16,5" CornerRadius="5" Background="{}" Foreground="White"/><Button x:Name="DlgNo" Content="No" Padding="16,5" CornerRadius="5"/>"##,
                ui::esc(yes),
                if p.danger { "#FFC4314B" } else { "#FF2F7BF6" }
            ),
            None => r##"<Button x:Name="DlgNo" Content="OK" Padding="16,5" CornerRadius="5" Background="#FF2F7BF6" Foreground="White"/>"##.to_string(),
        };
        let xaml = format!(
            r##"<Grid {ns} Background="#B8080A10"><Border HorizontalAlignment="Center" VerticalAlignment="Center" MinWidth="380" MaxWidth="{max_w}" Margin="24" Padding="22,18" CornerRadius="10" Background="#FF1C1F2A" BorderBrush="{edge}" BorderThickness="1">
<StackPanel Spacing="14"><TextBlock Text="{title}" FontSize="17" FontWeight="SemiBold" TextWrapping="Wrap"/>
<ScrollViewer MaxHeight="{max_h}" VerticalScrollBarVisibility="Auto"><StackPanel Spacing="2">{body}</StackPanel></ScrollViewer>
<StackPanel Orientation="Horizontal" Spacing="8" HorizontalAlignment="Right">{buttons}</StackPanel></StackPanel></Border></Grid>"##,
            ns = ui::NS,
            edge = if p.danger { "#88C4314B" } else { "#552F7BF6" },
            title = ui::esc(&p.title),
        );
        let Ok(el) = load::<UIElement>(&xaml) else {
            // a card that can't be shown says no
            eprintln!("dialog failed to load");
            self.dialogs.queue.pop_front();
            return self.dialog_show();
        };
        let _ = host.SetChild(&el);
        let _ = host.SetVisibility(Visibility::Visible);
        self.dialogs.on_yes = false;
        let Ok(fe) = el.cast::<crate::bindings::Microsoft::UI::Xaml::FrameworkElement>() else { return };
        for (name, yes) in [("DlgYes", true), ("DlgNo", false)] {
            if let Ok(b) = find::<ButtonBase>(&fe, name) {
                if let Ok(t) = b.Click(move |_, _| {
                    with(|s| s.dialog_answer(yes));
                }) {
                    keep(t);
                }
            }
        }
        // the safe answer has the focus (again when the window next comes to the front)
        self.dialogs.front = false;
        self.dialog_frame();
    }

    /// Each frame: a question open and the window just come to the front takes the focus.
    pub(crate) fn dialog_frame(&mut self) {
        if !self.dialogs.open() {
            return;
        }
        let front = crate::win32::is_foreground(self.hwnd);
        if front && !self.dialogs.front {
            self.dialog_focus();
        }
        self.dialogs.front = front;
    }

    /// Focus on the button the keyboard is on.
    fn dialog_focus(&self) {
        let Ok(host) = find::<Border>(&self.root, "DlgHost") else { return };
        let Some(fe) = host.Child().ok().and_then(|c| c.cast::<crate::bindings::Microsoft::UI::Xaml::FrameworkElement>().ok()) else { return };
        if let Ok(c) = find::<Control>(&fe, if self.dialogs.on_yes { "DlgYes" } else { "DlgNo" }) {
            let _ = c.Focus(FocusState::Programmatic);
        }
    }

    /// Every key while a question is open (the window's PreviewKeyDown, before any control):
    /// Esc says no; Tab and the arrows move between the two buttons; Enter and Space press the
    /// one the keyboard is on ("No" until moved). Any other key is kept from the window behind,
    /// so nothing there acts while the question is open. True when the key was the dialog's.
    pub(crate) fn dialog_key(&mut self, key: i32) -> bool {
        let Some(p) = self.dialogs.queue.front() else { return false };
        let two = p.yes.is_some();
        match key {
            27 => self.dialog_answer(false),
            13 | 32 => {
                let yes = two && self.dialogs.on_yes;
                self.dialog_answer(yes);
            }
            // Tab, Left, Up, Right, Down
            9 | 37 | 38 | 39 | 40 => {
                if two {
                    self.dialogs.on_yes = !self.dialogs.on_yes;
                }
                self.dialog_focus();
            }
            _ => {}
        }
        true
    }

    fn dialog_answer(&mut self, yes: bool) {
        let Some(p) = self.dialogs.queue.pop_front() else { return };
        // the next question (or nothing) first, so a callback that asks again is queued after it
        self.dialog_show();
        if yes {
            if let Some(then) = p.then {
                then(self);
            }
        }
    }
}
