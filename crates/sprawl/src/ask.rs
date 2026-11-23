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

/// Where the keyboard can be in a question with a box to type in.
#[derive(Clone, Copy, PartialEq, Debug)]
enum Spot {
    Box,
    Yes,
    No,
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

/// The `in_box` and `on_yes` flags for where the keyboard really is.
fn spot_flags(spot: Spot) -> (bool, bool) {
    (spot == Spot::Box, spot == Spot::Yes)
}

/// The questions waiting: the first is on screen, the rest follow in turn.
#[derive(Default)]
pub(crate) struct Dialogs {
    queue: std::collections::VecDeque<Pending>,
    /// The keyboard is on "Yes" (it starts on "No" for every question).
    on_yes: bool,
    /// The keyboard is meant to be in the question's box to type in (it starts there when there
    /// is one). Only where the focus is put back to; which key does what follows the real focus
    /// (`dialog_spot`), so a click into the box or onto a button is respected.
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
            Some(_) => body + &format!(r##"<TextBox x:Name="DlgInput" FontSize="{{$fs13}}" Margin="0,10,0,0" AutomationProperties.Name="{}"/>"##, ui::esc(&p.title)),
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
            // the text is set here and not written into the XAML: a value like #FF102030 there
            // would be read as a colour (and changed in the light theme), a control character lost
            let _ = b.SetText(&i.initial.as_str().into());
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
        // while the window was in front the user may have clicked into the box or onto a button:
        // keep what the focus goes back to in step with where they were
        if self.dialogs.front && self.dialogs.queue.front().is_some_and(|p| p.input.is_some()) {
            if let Some(spot) = self.dialog_spot() {
                (self.dialogs.in_box, self.dialogs.on_yes) = spot_flags(spot);
            }
        }
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

    /// Where the keyboard really is in a question with a box to type in: the box, "Yes" or "No",
    /// or none of them (the window was behind, or the focus is elsewhere).
    fn dialog_spot(&self) -> Option<Spot> {
        let host = find::<Border>(&self.root, "DlgHost").ok()?;
        let fe = host.Child().ok()?.cast::<crate::bindings::Microsoft::UI::Xaml::FrameworkElement>().ok()?;
        [("DlgInput", Spot::Box), ("DlgYes", Spot::Yes), ("DlgNo", Spot::No)]
            .into_iter()
            .find(|(name, _)| find::<Control>(&fe, name).ok().and_then(|c| c.FocusState().ok()).is_some_and(|f| f != FocusState::Unfocused))
            .map(|(_, spot)| spot)
    }

    /// Put the keyboard on `to` (and remember it for `dialog_focus`).
    fn dialog_go(&mut self, to: Spot) {
        self.dialogs.in_box = to == Spot::Box;
        self.dialogs.on_yes = to == Spot::Yes;
        self.dialog_focus();
    }

    /// Every key while a question is open (the window's PreviewKeyDown, before any control):
    /// Esc says no; Tab and the arrows move between the two buttons; Enter and Space press the
    /// one the keyboard is on ("No" until moved). Any other key is kept from the window behind,
    /// so nothing there acts while the question is open. True when the key was the dialog's.
    ///
    /// With a box to type in, the keys go by where the focus really is: in the box it gets every
    /// key but Esc (no), Enter (yes) and Tab (on to "Yes", then "No", then the box again; Shift+Tab
    /// the other way), and Space is only a space; on a button it is as without a box; with the
    /// focus in none of them every key but Esc only brings the focus back.
    pub(crate) fn dialog_key(&mut self, key: i32) -> bool {
        let Some(p) = self.dialogs.queue.front() else { return false };
        let two = p.yes.is_some();
        if p.input.is_some() {
            let shift = crate::win32::ctrl_shift_down().1;
            let step = |from: Spot| {
                let order = [Spot::Box, Spot::Yes, Spot::No];
                let at = order.iter().position(|&s| s == from).unwrap_or(0);
                order[(at + if shift { order.len() - 1 } else { 1 }) % order.len()]
            };
            match self.dialog_spot() {
                Some(Spot::Box) => match key {
                    27 => self.dialog_answer(false),
                    // Enter in the box says yes
                    13 => self.dialog_answer(true),
                    9 => self.dialog_go(step(Spot::Box)),
                    _ => return false,
                },
                Some(button) => match key {
                    27 => self.dialog_answer(false),
                    13 | 32 => self.dialog_answer(button == Spot::Yes),
                    9 => self.dialog_go(step(button)),
                    37 | 38 | 39 | 40 => self.dialog_go(if button == Spot::Yes { Spot::No } else { Spot::Yes }),
                    _ => {}
                },
                None => match key {
                    27 => self.dialog_answer(false),
                    _ => self.dialog_focus(),
                },
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_flags_follow_where_the_keyboard_really_is() {
        // Tab to Rename, then a click back into the box: coming back to the window must put the
        // focus in the box, not on the button
        assert_eq!(spot_flags(Spot::Yes), (false, true));
        assert_eq!(spot_flags(Spot::Box), (true, false));
        assert_eq!(spot_flags(Spot::No), (false, false));
    }
}
