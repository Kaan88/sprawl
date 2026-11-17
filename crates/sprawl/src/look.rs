//! How the window looks, said once: figures that keep their width while they count, and the
//! one way a selectable item (a tab, a chip, a map mode) is drawn.

use crate::bindings::Microsoft::UI::Xaml::Controls::Control;
use crate::bindings::Microsoft::UI::Xaml::FrameworkElement;
use crate::bindings::Microsoft::UI::Xaml::Media::Brush;
use crate::state::{find, load};
use crate::ui;

/// On a TextBlock (or Run) whose numbers change while the user watches: every digit takes the
/// same width, so the text doesn't shift sideways as it counts.
pub(crate) const NUM: &str = r##"Typography.NumeralAlignment="Tabular""##;

/// The background of the selected item, for every kind.
pub(crate) const SELECTED: &str = "#FF2C3B63";

/// The families of selectable items: they differ only in their background at rest.
#[derive(Clone, Copy, PartialEq, Debug)]
pub(crate) enum Pick {
    /// The tab row: nothing at rest.
    Tab,
    /// The Advisor's and Search's chips: a faint pill at rest.
    Chip,
    /// The map's mode buttons, on their dark strip.
    Mode,
}

impl Pick {
    pub(crate) fn rest(self) -> &'static str {
        match self {
            Pick::Tab => "Transparent",
            Pick::Chip => "#14FFFFFF",
            Pick::Mode => "#FF1E212C",
        }
    }

    /// Its background, selected or not.
    pub(crate) fn bg(self, on: bool) -> &'static str {
        if on {
            SELECTED
        } else {
            self.rest()
        }
    }
}

/// The attributes for a `<Button>` drawn as a selectable item.
pub(crate) fn pick(kind: Pick, on: bool) -> String {
    format!(r##"Style="{{StaticResource SprawlPick}}" Background="{}""##, kind.bg(on))
}

/// The same for a `<DropDownButton>` (its arrow is part of the look).
pub(crate) fn pick_drop(kind: Pick, on: bool) -> String {
    format!(r##"Style="{{StaticResource SprawlPickDrop}}" Background="{}""##, kind.bg(on))
}

/// Light or unlight the selectable item `name` (a tab, a mode) that is already on screen.
pub(crate) fn paint(root: &FrameworkElement, name: &str, kind: Pick, on: bool) {
    let Ok(c) = find::<Control>(root, name) else { return };
    if let Ok(br) = load::<Brush>(&format!(r##"<SolidColorBrush {} Color="{}"/>"##, ui::NS, kind.bg(on))) {
        let _ = c.SetBackground(&br);
    }
}

/// The template both selectable styles share: the item's own background (set by `paint` or
/// `pick`: at rest or selected), a white wash over it for the pointer (pointer over: faint,
/// pressed: a little more), and the system's focus rectangle, which shows for the keyboard
/// only. So selected-and-pointer-over is the selected background with the faint wash, and a
/// selected item the pointer has left is the selected background alone.
fn template(target: &str, arrow: bool) -> String {
    let (cols, arrow) = if arrow {
        (
            r##"<Grid.ColumnDefinitions><ColumnDefinition Width="*"/><ColumnDefinition Width="Auto"/></Grid.ColumnDefinitions>"##,
            r##"<FontIcon x:Name="Arrow" Grid.Column="1" Glyph="&#xE70D;" FontSize="8" Margin="0,0,9,0" VerticalAlignment="Center" Foreground="{TemplateBinding Foreground}"/>"##,
        )
    } else {
        ("", "")
    };
    format!(
        r##"<ControlTemplate TargetType="{target}">
  <Grid x:Name="Root" Background="{{TemplateBinding Background}}" CornerRadius="{{TemplateBinding CornerRadius}}" BorderBrush="{{TemplateBinding BorderBrush}}" BorderThickness="{{TemplateBinding BorderThickness}}">
    {cols}
    <Border x:Name="Wash" Grid.ColumnSpan="2" Background="#FFFFFFFF" Opacity="0" CornerRadius="{{TemplateBinding CornerRadius}}"/>
    <ContentPresenter x:Name="Content" Content="{{TemplateBinding Content}}" ContentTemplate="{{TemplateBinding ContentTemplate}}" Padding="{{TemplateBinding Padding}}" HorizontalContentAlignment="{{TemplateBinding HorizontalContentAlignment}}" VerticalContentAlignment="{{TemplateBinding VerticalContentAlignment}}" Foreground="{{TemplateBinding Foreground}}" FontSize="{{TemplateBinding FontSize}}" FontFamily="{{TemplateBinding FontFamily}}" FontWeight="{{TemplateBinding FontWeight}}"/>
    {arrow}
    <VisualStateManager.VisualStateGroups>
      <VisualStateGroup x:Name="CommonStates">
        <VisualState x:Name="Normal"/>
        <VisualState x:Name="PointerOver"><VisualState.Setters><Setter Target="Wash.Opacity" Value="0.07"/></VisualState.Setters></VisualState>
        <VisualState x:Name="Pressed"><VisualState.Setters><Setter Target="Wash.Opacity" Value="0.12"/><Setter Target="Content.Opacity" Value="0.8"/></VisualState.Setters></VisualState>
        <VisualState x:Name="Disabled"><VisualState.Setters><Setter Target="Content.Opacity" Value="0.4"/></VisualState.Setters></VisualState>
      </VisualStateGroup>
    </VisualStateManager.VisualStateGroups>
  </Grid>
</ControlTemplate>"##
    )
}

fn style(key: &str, target: &str, arrow: bool) -> String {
    format!(
        r##"<Style x:Key="{key}" TargetType="{target}">
  <Setter Property="Background" Value="Transparent"/>
  <Setter Property="Foreground" Value="#FFE6EAF2"/>
  <Setter Property="BorderThickness" Value="0"/>
  <Setter Property="Padding" Value="10,3"/>
  <Setter Property="CornerRadius" Value="6"/>
  <Setter Property="FontSize" Value="12"/>
  <Setter Property="HorizontalContentAlignment" Value="Center"/>
  <Setter Property="VerticalContentAlignment" Value="Center"/>
  <Setter Property="UseSystemFocusVisuals" Value="True"/>
  <Setter Property="FocusVisualMargin" Value="-2"/>
  <Setter Property="Template"><Setter.Value>{}</Setter.Value></Setter>
</Style>"##,
        template(target, arrow)
    )
}

/// The styles, as a dictionary merged into the application's resources at start (so any
/// XAML loaded later, the Advisor's or Search's included, can name them).
pub(crate) fn resources() -> String {
    format!(
        r##"<ResourceDictionary {}>{}{}</ResourceDictionary>"##,
        ui::NS,
        style("SprawlPick", "Button", false),
        style("SprawlPickDrop", "DropDownButton", true)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_selected_look_for_every_kind() {
        for k in [Pick::Tab, Pick::Chip, Pick::Mode] {
            assert_eq!(k.bg(true), SELECTED);
            assert_ne!(k.bg(false), SELECTED);
        }
        assert!(pick(Pick::Tab, true).contains("SprawlPick") && pick(Pick::Tab, true).contains(SELECTED));
        assert!(pick_drop(Pick::Mode, false).contains("SprawlPickDrop"));
    }

    #[test]
    fn the_styles_define_every_state_once() {
        let r = resources();
        for s in ["\"Normal\"", "\"PointerOver\"", "\"Pressed\"", "\"Disabled\""] {
            assert_eq!(r.matches(s).count(), 2, "{s}");
        }
        assert_eq!(r.matches("UseSystemFocusVisuals").count(), 2);
    }
}
