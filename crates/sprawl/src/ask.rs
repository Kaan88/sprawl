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

/// What runs on "Yes" of a question with a box to type in, given what was typed.
pub(crate) type ThenText = Box<dyn FnOnce(&mut State, String)>;

/// A question's box to type in.
pub(crate) struct Input {
    initial: String,
    /// What is selected at first: (start, length), in UTF-16 units as the box counts.
    select: (i32, i32),
    then: ThenText,
}

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
    /// A box to type in under the text (`then` is not used: `input` has its own).
    input: Option<Input>,
}

/// The questions waiting: the first is on screen, the rest follow in turn.
#[derive(Default)]
pub(crate) struct Dialogs {
    queue: std::collections::VecDeque<Pending>,
    /// The keyboard is on "Yes" (it starts on "No" for every question).
    on_yes: bool,
    /// The keyboard is in the question's box to type in (it starts there when there is one).
    in_box: bool,
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
        self.dialog_push(Pending { title: title.into(), text: text.into(), yes: Some(yes.into()), danger, then: Some(Box::new(then)), rich: None, input: None });
    }

    /// Ask for a line of text (a new name): `initial` is in the box with `select` selected;
    /// `then` runs with what was typed on `yes`. Enter says yes, Esc says no.
    pub(crate) fn ask_text(&mut self, title: &str, text: &str, initial: &str, select: (i32, i32), yes: &str, then: impl FnOnce(&mut State, String) + 'static) {
        let input = Input { initial: initial.into(), select, then: Box::new(then) };
        self.dialog_push(Pending { title: title.into(), text: text.into(), yes: Some(yes.into()), danger: false, then: None, rich: None, input: Some(input) });
    }

    /// A message with an OK button.
    pub(crate) fn tell(&mut self, title: &str, text: &str) {
        self.dialog_push(Pending { title: title.into(), text: text.into(), yes: None, danger: false, then: None, rich: None, input: None });
    }

    /// A message laid out in XAML (`body`: elements, the namespaces are on the card).
    pub(crate) fn tell_rich(&mut self, title: &str, body: String) {
        self.dialog_push(Pending { title: title.into(), text: String::new(), yes: None, danger: false, then: None, rich: Some(body), input: None });
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
            None => p.text.split('\n').map(|l| format!(r##"<TextBlock Text="{}" TextWrapping="Wrap" FontSize="{{$fs13}}" Foreground="{{$text1}}" IsTextSelectionEnabled="True"/>"##, ui::esc(l))).collect(),
        };
        let body = match &p.input {
            Some(i) => body + &format!(r##"<TextBox x:Name="DlgInput" Text="{}" FontSize="{{$fs13}}" Margin="0,10,0,0" AutomationProperties.Name="{}"/>"##, ui::esc(&i.initial), ui::esc(&p.title)),
            None => body,
        };
        let (max_w, max_h) = if p.rich.is_some() { (940, 720) } else { (560, 420) };
        let buttons = match &p.yes {
            Some(yes) => format!(
                r##"<Button x:Name="DlgYes" Content="{}" Style="{{StaticResource {}}}"/><Button x:Name="DlgNo" Content="No"/>"##,
                ui::esc(yes),
                if p.danger { "SprawlDanger" } else { "SprawlPrimary" }
            ),
            None => r##"<Button x:Name="DlgNo" Content="OK" Style="{StaticResource SprawlPrimary}"/>"##.to_string(),
        };
        let xaml = format!(
            r##"<Grid {ns} Background="{{$scrim}}"><Border HorizontalAlignment="Center" VerticalAlignment="Center" MinWidth="380" MaxWidth="{max_w}" Margin="24" Padding="22,18" CornerRadius="{{$rc10}}" Background="{{$popup}}" BorderBrush="{edge}" BorderThickness="1">
<Grid RowSpacing="14"><Grid.RowDefinitions><RowDefinition Height="Auto"/><RowDefinition Height="*"/><RowDefinition Height="Auto"/></Grid.RowDefinitions><TextBlock Text="{title}" FontSize="{{$fs17}}" FontWeight="SemiBold" TextWrapping="Wrap"/>
<ScrollViewer Grid.Row="1" MaxHeight="{max_h}" VerticalScrollBarVisibility="Auto"><StackPanel Spacing="2">{body}</StackPanel></ScrollViewer>
<StackPanel Grid.Row="2" Orientation="Horizontal" Spacing="8" HorizontalAlignment="Right">{buttons}</StackPanel></Grid></Border></Grid>"##,
            ns = ui::NS,
            edge = if p.danger { crate::tokens::refer("dangerline") } else { crate::tokens::refer("accentline") },
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
        self.dialogs.in_box = self.dialogs.queue.front().is_some_and(|p| p.input.is_some());
        let Ok(fe) = el.cast::<crate::bindings::Microsoft::UI::Xaml::FrameworkElement>() else { return };
        if let (Some(i), Ok(b)) = (self.dialogs.queue.front().and_then(|p| p.input.as_ref()), find::<crate::bindings::Microsoft::UI::Xaml::Controls::TextBox>(&fe, "DlgInput")) {
            let _ = b.Select(i.select.0, i.select.1);
        }
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
        // a question with a box to type in: the box has the keyboard until Tab moves off it
        if self.dialogs.in_box && self.dialogs.queue.front().is_some_and(|p| p.input.is_some()) {
            if let Ok(c) = find::<Control>(&fe, "DlgInput") {
                let _ = c.Focus(FocusState::Programmatic);
                return;
            }
        }
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
        // a box to type in: it gets every key but Esc, Enter and Tab (Tab goes on to the buttons)
        if p.input.is_some() && self.dialogs.in_box {
            match key {
                27 => self.dialog_answer(false),
                // Enter in the box says yes
                13 => self.dialog_answer(true),
                9 => {
                    self.dialogs.in_box = false;
                    self.dialogs.on_yes = true;
                    self.dialog_focus();
                }
                _ => return false,
            }
            return true;
        }
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

    /// What is typed in the open question's box.
    fn dialog_typed(&self) -> String {
        let Ok(host) = find::<Border>(&self.root, "DlgHost") else { return String::new() };
        let Some(fe) = host.Child().ok().and_then(|c| c.cast::<crate::bindings::Microsoft::UI::Xaml::FrameworkElement>().ok()) else { return String::new() };
        find::<crate::bindings::Microsoft::UI::Xaml::Controls::TextBox>(&fe, "DlgInput").ok().and_then(|b| b.Text().ok()).map(|t| t.to_string_lossy()).unwrap_or_default()
    }

    fn dialog_answer(&mut self, yes: bool) {
        // read before the card is replaced
        let typed = if self.dialogs.queue.front().is_some_and(|p| p.input.is_some()) { self.dialog_typed() } else { String::new() };
        let Some(p) = self.dialogs.queue.pop_front() else { return };
        // the next question (or nothing) first, so a callback that asks again is queued after it
        self.dialog_show();
        if yes {
            if let Some(then) = p.then {
                then(self);
            }
            if let Some(input) = p.input {
                (input.then)(self, typed);
            }
        }
    }
}
