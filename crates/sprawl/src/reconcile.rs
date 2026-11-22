//! Showing a panel again without loading it again.
//!
//! The Advisor's panels are written as one XAML string. Loading that string on every click
//! replaced every control in the panel: the one under the pointer lost its highlight and showed
//! it again at the next mouse move, and the whole panel blinked. Here the new string is compared
//! with the one on screen, and only what differs is changed: an attribute is set on the live
//! control, or one piece is loaded and put in its place. A control that stays keeps its pointer
//! and focus state. Whatever can't be changed that way is loaded again whole, as before.
//!
//! The comparison ([`diff`]) works on the two strings alone and is tested without a window;
//! [`Slot`] applies its result to the live controls, which it reaches by their place in the
//! tree (child 2 of child 0 ...), never by name: a piece loaded later has names of its own.

use std::collections::HashMap;

use windows_core::{IInspectable, Interface, HSTRING};
use windows_reference::IReference;

use crate::bindings::Microsoft::UI::Xaml::Automation::AutomationProperties;
use crate::bindings::Microsoft::UI::Xaml::Controls::Primitives::{RangeBase, Selector, ToggleButton};
use crate::bindings::Microsoft::UI::Xaml::Controls::{Border, ContentControl, Control, FontIcon, IconElement, Panel, ProgressBar, TextBlock, ToggleSwitch, ToolTipService};
use crate::bindings::Microsoft::UI::Xaml::Media::Brush;
use crate::bindings::Microsoft::UI::Xaml::Shapes::Shape;
use crate::bindings::Microsoft::UI::Xaml::{DependencyObject, FrameworkElement, UIElement, Visibility};

/// A byte range of the source.
type R = (u32, u32);

fn cut(src: &str, r: R) -> &str {
    &src[r.0 as usize..r.1 as usize]
}

/// How an element holds its children, as far as this module can reach them.
#[derive(Clone, Copy, PartialEq, Debug, Default)]
enum Kind {
    /// A panel: its children in order.
    Panel,
    /// One child at most (a Border's, a button's content).
    Single,
    /// One piece: changed by its own attributes or loaded again.
    #[default]
    Leaf,
}

fn kind_of(tag: &str) -> Kind {
    match tag {
        "StackPanel" | "Grid" | "Canvas" | "RelativePanel" => Kind::Panel,
        "Border" | "ScrollViewer" | "Button" | "CheckBox" | "ToggleButton" | "RadioButton" | "DropDownButton" | "HyperlinkButton" | "ContentControl" => Kind::Single,
        _ => Kind::Leaf,
    }
}

/// A control that changes its own state when the user acts on it: the state in the string is set
/// again at every update, whether the string changed or not.
fn keeps_state(tag: &str) -> bool {
    matches!(tag, "CheckBox" | "ToggleButton" | "RadioButton" | "ToggleSwitch" | "ComboBox")
}

/// One element of the string.
#[derive(Clone, Debug, Default)]
pub(crate) struct Node {
    /// The whole element: `<Tag ...>...</Tag>` or `<Tag .../>`.
    span: R,
    tag: R,
    /// Each attribute's name and its value as written (without the quotes).
    attrs: Vec<(R, R)>,
    /// The elements inside, in order (not the property elements).
    kids: Vec<Node>,
    /// Property elements (`<Grid.ColumnDefinitions>`): compared as text.
    props: Vec<R>,
    /// Text directly inside.
    text: bool,
    kind: Kind,
    /// A control that keeps its own state is this one or below it, within reach.
    stateful: bool,
    /// One is inside where it can't be reached (a property element, a piece): this element is
    /// loaded again at every update.
    sealed: bool,
    /// This element or one below it is sealed.
    deep: bool,
}

impl Node {
    fn src<'a>(&self, src: &'a str) -> &'a str {
        cut(src, self.span)
    }

    fn tag<'a>(&self, src: &'a str) -> &'a str {
        cut(src, self.tag)
    }

    fn attr<'a>(&self, src: &'a str, name: &str) -> Option<&'a str> {
        self.attrs.iter().find(|(n, _)| cut(src, *n) == name).map(|(_, v)| cut(src, *v))
    }

    fn name<'a>(&self, src: &'a str) -> Option<&'a str> {
        self.attr(src, "x:Name")
    }
}

/// A parsed string.
pub(crate) struct Doc {
    src: String,
    root: Node,
}

impl Doc {
    /// `None`: not the plain XML this module reads (the caller loads it whole).
    pub(crate) fn parse(src: &str) -> Option<Doc> {
        let mut p = Parser { b: src.as_bytes(), at: 0 };
        p.space();
        let root = p.element()?;
        p.space();
        (p.at == src.len()).then(|| Doc { src: src.to_string(), root })
    }
}

struct Parser<'a> {
    b: &'a [u8],
    at: usize,
}

impl Parser<'_> {
    fn space(&mut self) {
        while self.b.get(self.at).is_some_and(u8::is_ascii_whitespace) {
            self.at += 1;
        }
    }

    fn name(&mut self) -> Option<R> {
        let start = self.at;
        while self.b.get(self.at).is_some_and(|c| !c.is_ascii_whitespace() && !matches!(c, b'/' | b'>' | b'=' | b'<' | b'"' | b'\'')) {
            self.at += 1;
        }
        (self.at > start).then_some((start as u32, self.at as u32))
    }

    fn element(&mut self) -> Option<Node> {
        let start = self.at;
        if self.b.get(self.at) != Some(&b'<') {
            return None;
        }
        self.at += 1;
        let tag = self.name()?;
        let mut n = Node { tag, ..Default::default() };
        loop {
            self.space();
            match *self.b.get(self.at)? {
                b'/' => {
                    if self.b.get(self.at + 1) != Some(&b'>') {
                        return None;
                    }
                    self.at += 2;
                    return Some(self.finish(n, start));
                }
                b'>' => {
                    self.at += 1;
                    break;
                }
                _ => {
                    let name = self.name()?;
                    self.space();
                    if self.b.get(self.at) != Some(&b'=') {
                        return None;
                    }
                    self.at += 1;
                    self.space();
                    let quote = *self.b.get(self.at)?;
                    if quote != b'"' && quote != b'\'' {
                        return None;
                    }
                    self.at += 1;
                    let from = self.at;
                    while *self.b.get(self.at)? != quote {
                        self.at += 1;
                    }
                    n.attrs.push((name, (from as u32, self.at as u32)));
                    self.at += 1;
                }
            }
        }
        loop {
            let from = self.at;
            while *self.b.get(self.at)? != b'<' {
                self.at += 1;
            }
            if self.b[from..self.at].iter().any(|c| !c.is_ascii_whitespace()) {
                n.text = true;
            }
            let rest = &self.b[self.at..];
            if rest.starts_with(b"<!--") {
                self.at += rest.windows(3).position(|w| w == b"-->")? + 3;
                continue;
            }
            if rest.starts_with(b"</") {
                self.at += 2;
                let close = self.name()?;
                if self.b[close.0 as usize..close.1 as usize] != self.b[tag.0 as usize..tag.1 as usize] {
                    return None;
                }
                self.space();
                if self.b.get(self.at) != Some(&b'>') {
                    return None;
                }
                self.at += 1;
                return Some(self.finish(n, start));
            }
            // a declaration, CDATA: not read here
            if rest.starts_with(b"<!") || rest.starts_with(b"<?") {
                return None;
            }
            let kid = self.element()?;
            if self.b[kid.tag.0 as usize..kid.tag.1 as usize].contains(&b'.') {
                n.props.push(kid.span);
                // a control that keeps its own state, where it can't be reached
                if kid.stateful || kid.sealed {
                    n.sealed = true;
                }
            } else {
                n.kids.push(kid);
            }
        }
    }

    fn finish(&self, mut n: Node, start: usize) -> Node {
        n.span = (start as u32, self.at as u32);
        // the bytes are the caller's str, and tags are cut at ASCII: valid UTF-8
        let tag = std::str::from_utf8(&self.b[n.tag.0 as usize..n.tag.1 as usize]).unwrap_or("");
        n.kind = match kind_of(tag) {
            Kind::Single if n.kids.len() > 1 => Kind::Leaf,
            _ if n.text => Kind::Leaf,
            k => k,
        };
        let below = n.kids.iter().any(|k| k.stateful);
        let hidden = n.kids.iter().any(|k| k.sealed);
        if n.kind == Kind::Leaf {
            // nothing inside a piece is reached
            n.sealed |= below || hidden;
            n.stateful = keeps_state(tag);
        } else {
            n.stateful = keeps_state(tag) || below;
        }
        n.deep = n.sealed || n.kids.iter().any(|k| k.deep);
        n
    }
}

/// Whether `attr` of a `tag` element can be set on the live control.
fn settable(tag: &str, attr: &str) -> bool {
    let content = matches!(tag, "Button" | "CheckBox" | "ToggleButton" | "RadioButton" | "DropDownButton" | "HyperlinkButton" | "ContentControl");
    let control = content || matches!(tag, "TextBox" | "PasswordBox" | "ComboBox" | "ToggleSwitch" | "ProgressBar" | "ProgressRing" | "ScrollViewer");
    match attr {
        "Visibility" | "Opacity" | "Width" | "Height" | "ToolTipService.ToolTip" | "AutomationProperties.Name" => true,
        "Text" => tag == "TextBlock",
        "IsChecked" => matches!(tag, "CheckBox" | "ToggleButton" | "RadioButton"),
        "IsOn" => tag == "ToggleSwitch",
        "IsEnabled" => control,
        "Background" => control || tag == "Border" || kind_of(tag) == Kind::Panel,
        "Foreground" => control || matches!(tag, "TextBlock" | "FontIcon"),
        "BorderBrush" => control || tag == "Border",
        "Fill" => matches!(tag, "Ellipse" | "Rectangle" | "Polygon"),
        "Content" => content,
        "Glyph" => tag == "FontIcon",
        "Value" | "Maximum" | "IsIndeterminate" => tag == "ProgressBar",
        "SelectedIndex" => tag == "ComboBox",
        _ => false,
    }
}

/// One change to the live tree. A path is the place of an element: the index of each child on
/// the way down from the root, counting elements only (not property elements).
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Op<'n> {
    /// Set an attribute of the element at `path`.
    Set { path: Vec<u16>, tag: &'n str, attr: &'n str, value: &'n str },
    /// Load `xaml` in place of child `index` of the element at `parent`.
    Replace { parent: Vec<u16>, index: u16, xaml: &'n str },
    /// Load `xaml` as a new child `index` of the element at `parent`.
    Insert { parent: Vec<u16>, index: u16, xaml: &'n str },
    /// Take child `index` of the element at `parent` away.
    Remove { parent: Vec<u16>, index: u16 },
    /// Put the children of the panel at `parent` in a new order (a sorted table): new child j
    /// is the live child `Old(i)`, kept as it is, or `New(xaml)` loaded.
    Order { parent: Vec<u16>, from: Vec<Src<'n>>, loads: usize },
}

/// Where a child of [`Op::Order`] comes from.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Src<'n> {
    Old(u16),
    New(&'n str),
}

/// More pieces than this to load inside one element: the element is loaded as one instead.
const MAX_LOADS: usize = 16;

struct Differ<'o, 'n> {
    old: &'o str,
    new: &'n str,
    ops: Vec<Op<'n>>,
    path: Vec<u16>,
    loads: usize,
}

/// The changes that turn what `old` shows into what `new` shows, in the order to make them.
/// `None`: the root itself differs, so `new` is loaded whole.
pub(crate) fn diff<'n>(old: &Doc, new: &'n Doc) -> Option<Vec<Op<'n>>> {
    let mut d = Differ { old: &old.src, new: &new.src, ops: Vec::new(), path: Vec::new(), loads: 0 };
    d.node(&old.root, &new.root).then_some(d.ops)
}

impl<'n> Differ<'_, 'n> {
    /// Whether `a` becomes `b` in place (the changes are added to `ops`). `false`: `b` has to
    /// be loaded in `a`'s place, and nothing was added.
    fn node(&mut self, a: &Node, b: &'n Node) -> bool {
        if b.sealed {
            return false;
        }
        // the same text: nothing to do, unless something below is loaded every time
        if !b.deep && a.src(self.old) == b.src(self.new) {
            return true;
        }
        let tag = b.tag(self.new);
        if a.tag(self.old) != tag || a.kind != b.kind || a.text || b.text {
            return false;
        }
        let (mark, loads) = (self.ops.len(), self.loads);
        if !self.inside(a, b, tag) || self.loads - loads > MAX_LOADS {
            self.ops.truncate(mark);
            self.loads = loads;
            return false;
        }
        true
    }

    fn inside(&mut self, a: &Node, b: &'n Node, tag: &'n str) -> bool {
        // an attribute may change or come, if it can be set; none may go
        for (n, v) in &b.attrs {
            let (name, value) = (cut(self.new, *n), cut(self.new, *v));
            let was = a.attr(self.old, name);
            if was == Some(value) {
                continue;
            }
            // text content and element content are two things: a button that gets or loses its
            // Content attribute is loaded again
            if !settable(tag, name) || (name == "Content" && was.is_none()) {
                return false;
            }
            self.ops.push(Op::Set { path: self.path.clone(), tag, attr: name, value });
        }
        if a.attrs.iter().any(|(n, _)| b.attr(self.new, cut(self.old, *n)).is_none()) {
            return false;
        }
        if a.props.len() != b.props.len() || a.props.iter().zip(&b.props).any(|(x, y)| cut(self.old, *x) != cut(self.new, *y)) {
            return false;
        }
        match b.kind {
            Kind::Leaf => a.kids.len() == b.kids.len() && a.kids.iter().zip(&b.kids).all(|(x, y)| x.src(self.old) == y.src(self.new)),
            Kind::Single => {
                match (a.kids.first(), b.kids.first()) {
                    (None, None) => {}
                    (Some(x), Some(y)) => self.pair(x, y, 0),
                    (None, Some(y)) => self.load(Op::Insert { parent: self.path.clone(), index: 0, xaml: y.src(self.new) }),
                    (Some(_), None) => self.ops.push(Op::Remove { parent: self.path.clone(), index: 0 }),
                }
                true
            }
            Kind::Panel => {
                if self.keyed(a, b) {
                    return true;
                }
                let (na, nb) = (a.kids.len(), b.kids.len());
                let same = |x: &Node, y: &Node| x.tag(self.old) == y.tag(self.new) && x.name(self.old) == y.name(self.new);
                // the children that stay are the same ones at the start and at the end
                let mut pre = 0;
                while pre < na.min(nb) && same(&a.kids[pre], &b.kids[pre]) {
                    pre += 1;
                }
                let mut suf = 0;
                while suf < na.min(nb) - pre && same(&a.kids[na - 1 - suf], &b.kids[nb - 1 - suf]) {
                    suf += 1;
                }
                let (gone, come) = (na - pre - suf, nb - pre - suf);
                if gone == come {
                    for i in 0..na {
                        self.pair(&a.kids[i], &b.kids[i], i);
                    }
                    return true;
                }
                // the middle first, so every place after it is the new one when its turn comes:
                // what went (from the end, so the places before stay), then what came
                for k in (0..gone).rev() {
                    self.ops.push(Op::Remove { parent: self.path.clone(), index: (pre + k) as u16 });
                }
                for k in 0..come {
                    self.load(Op::Insert { parent: self.path.clone(), index: (pre + k) as u16, xaml: b.kids[pre + k].src(self.new) });
                }
                for i in 0..pre {
                    self.pair(&a.kids[i], &b.kids[i], i);
                }
                for j in 0..suf {
                    self.pair(&a.kids[na - suf + j], &b.kids[nb - suf + j], nb - suf + j);
                }
                true
            }
        }
    }

    /// Children that all have names of their own (a table's rows, one per item): the ones that
    /// stay are found by name wherever they moved to, so a folder opened in the middle of the
    /// list inserts its rows and leaves the others alone. `false` (nothing added): not that
    /// case, or the ones that stay changed order (a sort): the caller compares by place.
    fn keyed(&mut self, a: &Node, b: &'n Node) -> bool {
        let names = |src: &str, n: &Node| -> Option<Vec<String>> {
            let v: Vec<String> = n.kids.iter().map(|k| k.name(src).map(str::to_string)).collect::<Option<_>>()?;
            let unique: std::collections::HashSet<&String> = v.iter().collect();
            (unique.len() == v.len()).then_some(v)
        };
        let (Some(old), Some(new)) = (names(self.old, a), names(self.new, b)) else { return false };
        if old.is_empty() || new.is_empty() || old == new {
            return false;
        }
        let in_new: std::collections::HashSet<&String> = new.iter().collect();
        let in_old: HashMap<&String, usize> = old.iter().enumerate().map(|(i, n)| (n, i)).collect();
        let stay_old: Vec<&String> = old.iter().filter(|n| in_new.contains(n)).collect();
        let stay_new: Vec<&String> = new.iter().filter(|n| in_old.contains_key(n)).collect();
        if stay_old.is_empty() {
            return false;
        }
        if stay_old != stay_new {
            // another order: the rows that stay are kept and put in their new places, not
            // loaded again (a sort moves every row)
            let from: Vec<Src<'n>> = new.iter().enumerate().map(|(j, n)| in_old.get(n).map_or(Src::New(b.kids[j].src(self.new)), |&i| Src::Old(i as u16))).collect();
            let loads = from.iter().filter(|s| matches!(s, Src::New(_))).count();
            self.ops.push(Op::Order { parent: self.path.clone(), from, loads });
            self.loads += loads;
            for (j, n) in new.iter().enumerate() {
                if let Some(&i) = in_old.get(n) {
                    self.pair(&a.kids[i], &b.kids[j], j);
                }
            }
            return true;
        }
        // what went (from the end, so the places before stay), then what came at its new
        // place, then the ones that stayed, each at its new place
        for i in (0..old.len()).rev() {
            if !in_new.contains(&old[i]) {
                self.ops.push(Op::Remove { parent: self.path.clone(), index: i as u16 });
            }
        }
        for (j, n) in new.iter().enumerate() {
            if !in_old.contains_key(n) {
                self.load(Op::Insert { parent: self.path.clone(), index: j as u16, xaml: b.kids[j].src(self.new) });
            }
        }
        for (j, n) in new.iter().enumerate() {
            if let Some(&i) = in_old.get(n) {
                self.pair(&a.kids[i], &b.kids[j], j);
            }
        }
        true
    }

    fn load(&mut self, op: Op<'n>) {
        self.ops.push(op);
        self.loads += 1;
    }

    /// Child `index`: changed in place, or loaded again.
    fn pair(&mut self, x: &Node, y: &'n Node, index: usize) {
        self.path.push(index as u16);
        let ok = self.node(x, y);
        self.path.pop();
        if !ok {
            self.load(Op::Replace { parent: self.path.clone(), index: index as u16, xaml: y.src(self.new) });
        }
    }
}

/// The names (`x:Name`) written in `xaml`.
pub(crate) fn names_in(xaml: &str) -> Vec<String> {
    const KEY: &str = "x:Name=\"";
    let mut out = Vec::new();
    let mut rest = xaml;
    while let Some(p) = rest.find(KEY) {
        rest = &rest[p + KEY.len()..];
        let Some(end) = rest.find('"') else { break };
        out.push(rest[..end].to_string());
        rest = &rest[end..];
    }
    out
}

fn holds_name(xaml: &str, name: &str) -> bool {
    xaml.contains(&format!("x:Name=\"{name}\""))
}

/// Where the element called `name` is: the path to it, or (`true`) to the piece it is inside,
/// to be asked for it by name.
fn locate(src: &str, n: &Node, name: &str, path: &mut Vec<u16>) -> Option<bool> {
    if n.name(src) == Some(name) {
        return Some(false);
    }
    if n.kind == Kind::Leaf {
        return holds_name(n.src(src), name).then_some(true);
    }
    for (i, k) in n.kids.iter().enumerate() {
        path.push(i as u16);
        if let Some(inside) = locate(src, k, name, path) {
            return Some(inside);
        }
        path.pop();
    }
    n.props.iter().any(|p| holds_name(cut(src, *p), name)).then_some(true)
}

/// An attribute value as the text it stands for.
pub(crate) fn unescape(raw: &str) -> String {
    // "{}" in front says the rest is plain text, not a markup extension
    let raw = raw.strip_prefix("{}").unwrap_or(raw);
    let mut out = String::with_capacity(raw.len());
    let mut rest = raw;
    while let Some(p) = rest.find('&') {
        out.push_str(&rest[..p]);
        rest = &rest[p..];
        let Some(end) = rest.find(';') else { break };
        let entity = &rest[1..end];
        let c = match entity {
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            _ => entity.strip_prefix("#x").or_else(|| entity.strip_prefix("#X")).map(|h| u32::from_str_radix(h, 16)).or_else(|| entity.strip_prefix('#').map(str::parse)).and_then(Result::ok).and_then(char::from_u32),
        };
        match c {
            Some(c) => out.push(c),
            None => out.push_str(&rest[..=end]),
        }
        rest = &rest[end + 1..];
    }
    out.push_str(rest);
    out
}

/// `xaml` (an element cut from a larger string) with the namespaces a string loaded alone needs.
fn alone(xaml: &str) -> String {
    let at = xaml.find(|c: char| c.is_whitespace() || c == '/' || c == '>').unwrap_or(xaml.len());
    format!("{} {}{}", &xaml[..at], crate::ui::NS, &xaml[at..])
}

/// "This can't be done in place": the caller loads the string whole.
fn bad() -> windows_core::Error {
    // E_FAIL
    windows_core::Error::from_hresult(windows_core::HRESULT(0x8000_4005_u32 as i32))
}

fn flag(raw: &str) -> windows_core::Result<bool> {
    match raw {
        "True" | "true" => Ok(true),
        "False" | "false" => Ok(false),
        _ => Err(bad()),
    }
}

fn number(raw: &str) -> windows_core::Result<f64> {
    raw.parse::<f64>().ok().filter(|v| v.is_finite()).ok_or_else(bad)
}

fn boxed(text: &str) -> IInspectable {
    IReference::<HSTRING>::from(text).into()
}

thread_local! {
    /// The brushes made so far, by what they are for and their value as written.
    static BRUSHES: std::cell::RefCell<HashMap<(&'static str, String), Brush>> = std::cell::RefCell::new(HashMap::new());
}

/// The theme changed: a brush made for the old one is never given out again.
pub(crate) fn forget_brushes() {
    BRUSHES.with(|m| m.borrow_mut().clear());
}

/// The brush `attr="raw"` gives, as loading it gives it: a token's value for the theme in use,
/// a colour written by hand turned around for the light page. Kept by the text as it is written;
/// one key means one brush until the theme changes ([`forget_brushes`]).
fn brush(attr: &'static str, raw: &str) -> windows_core::Result<Brush> {
    if let Some(b) = BRUSHES.with(|m| m.borrow().get(&(attr, raw.to_string())).cloned()) {
        return Ok(b);
    }
    let ns = crate::ui::NS;
    let b = match attr {
        "Foreground" => crate::state::load::<TextBlock>(&format!(r#"<TextBlock {ns} Foreground="{raw}"/>"#))?.Foreground()?,
        "BorderBrush" => crate::state::load::<Border>(&format!(r#"<Border {ns} BorderBrush="{raw}"/>"#))?.BorderBrush()?,
        "Fill" => crate::state::load::<Shape>(&format!(r#"<Rectangle {ns} Fill="{raw}"/>"#))?.Fill()?,
        _ => crate::state::load::<Border>(&format!(r#"<Border {ns} Background="{raw}"/>"#))?.Background()?,
    };
    BRUSHES.with(|m| m.borrow_mut().insert((attr, raw.to_string()), b.clone()));
    Ok(b)
}

/// Set `attr` of the live `el` (a `tag` element) to the value written as `raw`.
fn set(el: &UIElement, tag: &str, attr: &str, raw: &str) -> windows_core::Result<()> {
    // a token is resolved for the values read here; a brush is made from `raw` as it is written,
    // so `state::load` puts the theme's value in (resolved first, it would be turned around again)
    let resolved = crate::tokens::resolve(raw);
    let value = resolved.as_ref();
    match attr {
        "Visibility" => el.SetVisibility(match value {
            "Visible" => Visibility::Visible,
            "Collapsed" => Visibility::Collapsed,
            _ => return Err(bad()),
        }),
        "Opacity" => el.SetOpacity(number(value)?),
        "Width" => el.cast::<FrameworkElement>()?.SetWidth(number(value)?),
        "Height" => el.cast::<FrameworkElement>()?.SetHeight(number(value)?),
        "ToolTipService.ToolTip" => ToolTipService::SetToolTip(&el.cast::<DependencyObject>()?, &boxed(&unescape(value))),
        "AutomationProperties.Name" => AutomationProperties::SetName(&el.cast::<DependencyObject>()?, &HSTRING::from(unescape(value))),
        "Text" => el.cast::<TextBlock>()?.SetText(&HSTRING::from(unescape(value))),
        "IsChecked" => el.cast::<ToggleButton>()?.SetIsChecked(Some(flag(value)?)),
        "IsOn" => el.cast::<ToggleSwitch>()?.SetIsOn(flag(value)?),
        "IsEnabled" => el.cast::<Control>()?.SetIsEnabled(flag(value)?),
        "Background" => {
            let b = brush("Background", raw)?;
            if tag == "Border" {
                el.cast::<Border>()?.SetBackground(&b)
            } else if kind_of(tag) == Kind::Panel {
                el.cast::<Panel>()?.SetBackground(&b)
            } else {
                el.cast::<Control>()?.SetBackground(&b)
            }
        }
        "Foreground" => {
            let b = brush("Foreground", raw)?;
            match tag {
                "TextBlock" => el.cast::<TextBlock>()?.SetForeground(&b),
                "FontIcon" => el.cast::<IconElement>()?.SetForeground(&b),
                _ => el.cast::<Control>()?.SetForeground(&b),
            }
        }
        "BorderBrush" => {
            let b = brush("BorderBrush", raw)?;
            if tag == "Border" {
                el.cast::<Border>()?.SetBorderBrush(&b)
            } else {
                el.cast::<Control>()?.SetBorderBrush(&b)
            }
        }
        "Fill" => el.cast::<Shape>()?.SetFill(&brush("Fill", raw)?),
        "Content" => el.cast::<ContentControl>()?.SetContent(&boxed(&unescape(value))),
        "Glyph" => el.cast::<FontIcon>()?.SetGlyph(&HSTRING::from(unescape(value))),
        "Value" => el.cast::<RangeBase>()?.SetValue(number(value)?),
        "Maximum" => el.cast::<RangeBase>()?.SetMaximum(number(value)?),
        "IsIndeterminate" => el.cast::<ProgressBar>()?.SetIsIndeterminate(flag(value)?),
        "SelectedIndex" => el.cast::<Selector>()?.SetSelectedIndex(value.parse::<i32>().map_err(|_| bad())?),
        _ => Err(bad()),
    }
}

/// Child `index` of the live `el`, which the string says is a `node`.
fn child(el: &UIElement, src: &str, node: &Node, index: u16) -> windows_core::Result<UIElement> {
    match node.kind {
        Kind::Panel => el.cast::<Panel>()?.Children()?.GetAt(u32::from(index)),
        Kind::Single if node.tag(src) == "Border" => el.cast::<Border>()?.Child(),
        Kind::Single => el.cast::<ContentControl>()?.Content()?.cast(),
        Kind::Leaf => Err(bad()),
    }
}

/// The live element at `path`, and its node.
fn reach<'d>(root: &UIElement, doc: &'d Doc, path: &[u16]) -> windows_core::Result<(UIElement, &'d Node)> {
    let (mut el, mut node) = (root.clone(), &doc.root);
    for &i in path {
        el = child(&el, &doc.src, node, i)?;
        node = node.kids.get(usize::from(i)).ok_or_else(bad)?;
    }
    Ok((el, node))
}

/// A piece loaded while a panel was shown: its root, and the names inside it.
pub(crate) struct Loaded {
    pub root: FrameworkElement,
    pub names: Vec<String>,
    /// The root's own name, if it has one.
    pub own: Option<String>,
}

impl Loaded {
    fn new(root: FrameworkElement, xaml: &str) -> Loaded {
        let names = names_in(xaml);
        // the first tag's attributes end at its first '>' (one inside a value is written &gt;)
        let head = &xaml[..xaml.find('>').unwrap_or(xaml.len())];
        let own = names.first().filter(|n| holds_name(head, n)).cloned();
        Loaded { root, names, own }
    }

    /// The element called `name` in this piece: its root, or one inside it.
    pub(crate) fn find<T: Interface>(&self, name: &str) -> Option<T> {
        if self.own.as_deref() == Some(name) {
            return self.root.cast().ok();
        }
        crate::state::find::<T>(&self.root, name).ok()
    }
}

/// What [`Slot::show`] did.
pub(crate) struct Shown {
    /// The whole string was loaded (every control is new).
    pub whole: bool,
    /// What was loaded: all of it, or the pieces that changed.
    pub loaded: Vec<Loaded>,
}

/// A host and what was last put into it.
#[derive(Default)]
pub(crate) struct Slot {
    doc: Option<Doc>,
    root: Option<FrameworkElement>,
    /// Elements set from code since (a status line that counts): set again from the string at
    /// the next update, whether the string changed or not.
    touched: std::cell::RefCell<Vec<String>>,
}

impl Slot {
    /// The root of what is shown.
    pub(crate) fn root(&self) -> Option<&FrameworkElement> {
        self.root.as_ref()
    }

    /// The next `show` loads the string whole.
    pub(crate) fn forget(&mut self) {
        self.doc = None;
    }

    /// Put `xaml` into `host`: in place where `in_place` allows and the strings do, whole
    /// otherwise (the first time, another root, anything this module doesn't know how to do).
    pub(crate) fn show(&mut self, host: &Border, xaml: &str, in_place: bool) -> windows_core::Result<Shown> {
        // the strings are compared as they are written, tokens and all: one that didn't change
        // reads the same on both sides. Whatever is loaded or set goes through `state::load`,
        // which puts the theme's value in, and `set` resolves a token for a value it reads itself
        let new = Doc::parse(xaml);
        let mut pieces = None;
        if let (true, Some(old), Some(root), Some(new)) = (in_place, self.doc.as_ref(), self.root.clone(), new.as_ref()) {
            // the host still holds what this slot put there
            let held = host.Child().ok().and_then(|c| c.cast::<FrameworkElement>().ok()).is_some_and(|c| c == root);
            if let (true, Some(ops)) = (held, diff(old, new)) {
                if std::env::var_os("SPRAWL_TIMING").is_some() {
                    for op in &ops {
                        let line = match op {
                            Op::Set { path, attr, value, tag } => format!("op Set {path:?} {tag}.{attr}={value}"),
                            Op::Replace { parent, index, xaml } => format!("op Replace {parent:?}[{index}] {}", xaml.chars().take(90).collect::<String>()),
                            Op::Insert { parent, index, xaml } => format!("op Insert {parent:?}[{index}] {}", xaml.chars().take(90).collect::<String>()),
                            Op::Remove { parent, index } => format!("op Remove {parent:?}[{index}]"),
                            Op::Order { parent, from, loads } => format!("op Order {parent:?} {} children, {loads} loaded", from.len()),
                        };
                        crate::state::timing(&line, std::time::Instant::now());
                    }
                }
                let touched = std::mem::take(&mut *self.touched.borrow_mut());
                // a step that fails part way leaves neither string on screen: all of it again
                pieces = apply(&root, new, &ops).and_then(|l| restate(&root, new, &touched).map(|()| l)).ok();
            }
        }
        if let Some(loaded) = pieces {
            self.doc = new;
            return Ok(Shown { whole: false, loaded });
        }
        let el = crate::state::load::<UIElement>(xaml)?;
        host.SetChild(&el)?;
        let root = el.cast::<FrameworkElement>()?;
        self.root = Some(root.clone());
        self.doc = new;
        self.touched.borrow_mut().clear();
        Ok(Shown { whole: true, loaded: vec![Loaded::new(root, xaml)] })
    }

    /// The element called `name` in what is shown. Looked for by its place where it has one, so
    /// a piece loaded later is found too (it is not in the first string's names).
    pub(crate) fn find<T: Interface>(&self, name: &str) -> Option<T> {
        let root = self.root.as_ref()?;
        let Some(doc) = &self.doc else { return crate::state::find::<T>(root, name).ok() };
        let mut path = Vec::new();
        let inside = locate(&doc.src, &doc.root, name, &mut path)?;
        let (el, _) = reach(&root.cast::<UIElement>().ok()?, doc, &path).ok()?;
        if inside {
            crate::state::find::<T>(&el.cast::<FrameworkElement>().ok()?, name).ok()
        } else {
            el.cast::<T>().ok()
        }
    }

    /// Set the text of the TextBlock called `name` from code (a status line that changes while
    /// the string doesn't). The string's own text comes back at the next update.
    pub(crate) fn text(&self, name: &str, value: &str) -> bool {
        let Some(t) = self.find::<TextBlock>(name) else { return false };
        let _ = t.SetText(&HSTRING::from(value));
        self.touch(name);
        true
    }

    /// `name` was changed from code.
    pub(crate) fn touch(&self, name: &str) {
        let mut t = self.touched.borrow_mut();
        if !t.iter().any(|n| n == name) {
            t.push(name.to_string());
        }
    }
}

/// Make `ops` on the live tree under `root`. The pieces loaded on the way come back.
fn apply(root: &FrameworkElement, new: &Doc, ops: &[Op]) -> windows_core::Result<Vec<Loaded>> {
    let root = root.cast::<UIElement>()?;
    let mut loaded = Vec::new();
    for op in ops {
        match op {
            Op::Set { path, tag, attr, value } => set(&reach(&root, new, path)?.0, tag, attr, value)?,
            Op::Remove { parent, index } => {
                let (el, node) = reach(&root, new, parent)?;
                match node.kind {
                    Kind::Panel => el.cast::<Panel>()?.Children()?.RemoveAt(u32::from(*index))?,
                    Kind::Single if node.tag(&new.src) == "Border" => el.cast::<Border>()?.SetChild(None::<&UIElement>)?,
                    Kind::Single => el.cast::<ContentControl>()?.SetContent(None::<&IInspectable>)?,
                    Kind::Leaf => return Err(bad()),
                }
            }
            Op::Order { parent, from, .. } => {
                let (el, node) = reach(&root, new, parent)?;
                if node.kind != Kind::Panel {
                    return Err(bad());
                }
                let kids = el.cast::<Panel>()?.Children()?;
                let old: Vec<UIElement> = (0..kids.Size()?).map(|i| kids.GetAt(i)).collect::<windows_core::Result<_>>()?;
                let mut order = Vec::with_capacity(from.len());
                for s in from {
                    order.push(match s {
                        Src::Old(i) => old.get(usize::from(*i)).cloned().ok_or_else(bad)?,
                        Src::New(xaml) => {
                            let piece = crate::state::load::<UIElement>(&alone(xaml))?;
                            loaded.push(Loaded::new(piece.cast()?, xaml));
                            piece
                        }
                    });
                }
                kids.Clear()?;
                for e in &order {
                    kids.Append(e)?;
                }
            }
            Op::Replace { parent, index, xaml } | Op::Insert { parent, index, xaml } => {
                let (el, node) = reach(&root, new, parent)?;
                let piece = crate::state::load::<UIElement>(&alone(xaml))?;
                match node.kind {
                    Kind::Panel if matches!(op, Op::Insert { .. }) => el.cast::<Panel>()?.Children()?.InsertAt(u32::from(*index), &piece)?,
                    Kind::Panel => el.cast::<Panel>()?.Children()?.SetAt(u32::from(*index), &piece)?,
                    Kind::Single if node.tag(&new.src) == "Border" => el.cast::<Border>()?.SetChild(&piece)?,
                    Kind::Single => el.cast::<ContentControl>()?.SetContent(&piece.cast::<IInspectable>()?)?,
                    Kind::Leaf => return Err(bad()),
                }
                loaded.push(Loaded::new(piece.cast()?, xaml));
            }
        }
    }
    Ok(loaded)
}

/// After the changes: the state of every control that keeps its own (a tick box the user
/// clicked shows what the string says, also when the string didn't change), and the elements
/// set from code.
fn restate(root: &FrameworkElement, new: &Doc, touched: &[String]) -> windows_core::Result<()> {
    let root = root.cast::<UIElement>()?;
    restate_node(&root, &new.src, &new.root)?;
    for name in touched {
        let mut path = Vec::new();
        if locate(&new.src, &new.root, name, &mut path) != Some(false) {
            continue;
        }
        let (el, node) = reach(&root, new, &path)?;
        let tag = node.tag(&new.src);
        for attr in ["Text", "Visibility"] {
            if settable(tag, attr) {
                // not written: the property's own default
                let value = node.attr(&new.src, attr).unwrap_or(if attr == "Text" { "" } else { "Visible" });
                set(&el, tag, attr, value)?;
            }
        }
    }
    Ok(())
}

fn restate_node(el: &UIElement, src: &str, node: &Node) -> windows_core::Result<()> {
    if !node.stateful {
        return Ok(());
    }
    match node.tag(src) {
        "CheckBox" | "ToggleButton" | "RadioButton" => {
            let want = node.attr(src, "IsChecked").map_or(Ok(false), flag)?;
            let t = el.cast::<ToggleButton>()?;
            if t.IsChecked().ok() != Some(want) {
                t.SetIsChecked(Some(want))?;
            }
        }
        "ToggleSwitch" => {
            let want = node.attr(src, "IsOn").map_or(Ok(false), flag)?;
            let t = el.cast::<ToggleSwitch>()?;
            if t.IsOn().ok() != Some(want) {
                t.SetIsOn(want)?;
            }
        }
        "ComboBox" => {
            if let Some(want) = node.attr(src, "SelectedIndex").and_then(|v| v.parse::<i32>().ok()) {
                let s = el.cast::<Selector>()?;
                if s.SelectedIndex().ok() != Some(want) {
                    s.SetSelectedIndex(want)?;
                }
            }
        }
        _ => {}
    }
    if node.kind == Kind::Leaf {
        return Ok(());
    }
    for (i, k) in node.kids.iter().enumerate() {
        if k.stateful {
            restate_node(&child(el, src, node, i as u16)?, src, k)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    /// The tree a string stands for, as the live tree holds it: what the ops act on.
    #[derive(Clone, Debug, PartialEq)]
    struct Model {
        tag: String,
        attrs: BTreeMap<String, String>,
        props: Vec<String>,
        kids: Vec<Model>,
        /// A piece: its inside, as written.
        piece: Option<String>,
    }

    fn model(src: &str, n: &Node) -> Model {
        Model {
            tag: n.tag(src).to_string(),
            attrs: n.attrs.iter().map(|(k, v)| (cut(src, *k).to_string(), cut(src, *v).to_string())).collect(),
            props: n.props.iter().map(|p| cut(src, *p).to_string()).collect(),
            kids: if n.kind == Kind::Leaf { Vec::new() } else { n.kids.iter().map(|k| model(src, k)).collect() },
            piece: (n.kind == Kind::Leaf).then(|| n.kids.iter().map(|k| k.src(src)).collect::<Vec<_>>().join("") + if n.text { "#text" } else { "" }),
        }
    }

    fn of(xaml: &str) -> Model {
        let d = Doc::parse(xaml).expect("parses");
        model(&d.src, &d.root)
    }

    fn at<'m>(m: &'m mut Model, path: &[u16]) -> &'m mut Model {
        path.iter().fold(m, |m, &i| &mut m.kids[usize::from(i)])
    }

    fn run(m: &mut Model, ops: &[Op]) {
        for op in ops {
            match op {
                Op::Set { path, attr, value, .. } => {
                    at(m, path).attrs.insert(attr.to_string(), value.to_string());
                }
                Op::Replace { parent, index, xaml } => at(m, parent).kids[usize::from(*index)] = of(xaml),
                Op::Insert { parent, index, xaml } => at(m, parent).kids.insert(usize::from(*index), of(xaml)),
                Op::Order { parent, from, .. } => {
                    let p = at(m, parent);
                    let old = std::mem::take(&mut p.kids);
                    p.kids = from.iter().map(|s| match s {
                        Src::Old(i) => old[usize::from(*i)].clone(),
                        Src::New(xaml) => of(xaml),
                    }).collect();
                }
                Op::Remove { parent, index } => {
                    at(m, parent).kids.remove(usize::from(*index));
                }
            }
        }
    }

    /// The ops for `old` to `new`, checked: made on `old`'s tree they give `new`'s.
    fn ops(old: &str, new: &str) -> Option<Vec<String>> {
        let (a, b) = (Doc::parse(old).expect("old parses"), Doc::parse(new).expect("new parses"));
        let ops = diff(&a, &b)?;
        let mut m = model(&a.src, &a.root);
        run(&mut m, &ops);
        assert_eq!(m, model(&b.src, &b.root), "the ops don't give the new tree\nold: {old}\nnew: {new}\nops: {ops:?}");
        Some(
            ops.iter()
                .map(|op| match op {
                    Op::Set { path, attr, value, .. } => format!("set {path:?} {attr}={value}"),
                    Op::Replace { parent, index, .. } => format!("replace {parent:?}/{index}"),
                    Op::Insert { parent, index, .. } => format!("insert {parent:?}/{index}"),
                    Op::Remove { parent, index } => format!("remove {parent:?}/{index}"),
                    Op::Order { parent, loads, .. } => format!("order {parent:?} {loads} new"),
                })
                .collect(),
        )
    }

    #[test]
    fn reads_the_xaml_the_app_writes() {
        let x = r##"<Grid xmlns="a" xmlns:x="b" ColumnSpacing="6"><Grid.ColumnDefinitions><ColumnDefinition Width="40"/><ColumnDefinition Width="*"/></Grid.ColumnDefinitions>
<CheckBox x:Name="AdvT3" IsChecked="True" AutomationProperties.Name="Clean up a &amp; b"/><StackPanel Grid.Column="1"><TextBlock FontSize="{$fs15}"><Run Text="1 GiB" FontWeight="SemiBold"/><Run Text=" can be cleaned"/></TextBlock><FontIcon Glyph="&#xE72E;"/><!-- a note --></StackPanel></Grid>"##;
        let d = Doc::parse(x).unwrap();
        assert_eq!(d.root.tag(&d.src), "Grid");
        assert_eq!(d.root.kind, Kind::Panel);
        assert_eq!((d.root.kids.len(), d.root.props.len()), (2, 1), "the column definitions are not a child");
        assert_eq!(d.root.kids[0].name(&d.src), Some("AdvT3"));
        assert_eq!(d.root.kids[0].attr(&d.src, "AutomationProperties.Name"), Some("Clean up a &amp; b"));
        assert!(d.root.stateful && d.root.kids[0].stateful && !d.root.kids[1].stateful);
        // a TextBlock with Runs is one piece
        assert_eq!(d.root.kids[1].kids[0].kind, Kind::Leaf);
        assert_eq!(names_in(x), ["AdvT3"]);
        // not XML this reads: loaded whole
        for bad in ["", "<Grid>", "<Grid></Border>", "<Grid a=b/>", "<Grid/><Grid/>", "text", "<Grid><![CDATA[x]]></Grid>"] {
            assert!(Doc::parse(bad).is_none(), "{bad}");
        }
        assert_eq!(unescape("a &amp; b &lt;c&gt; &quot;d&quot; &#xE72E;&#65;"), "a & b <c> \"d\" \u{E72E}A");
        assert_eq!(unescape("{}{braces}"), "{braces}");
        assert_eq!(unescape("50 % &unknown; &"), "50 % &unknown; &");
        assert_eq!(alone(r#"<Border x:Name="a"><Grid/></Border>"#), format!(r#"<Border {} x:Name="a"><Grid/></Border>"#, crate::ui::NS));
        assert_eq!(alone("<Grid/>"), format!("<Grid {}/>", crate::ui::NS));
    }

    /// A list as the Advisor writes one: chips, rows with a tick box and a button, a foot.
    fn panel(ticked: &[usize], open: Option<usize>, order: &[usize], sort: &str) -> String {
        let chip = |name: &str| format!(r##"<Button x:Name="AdvSort{name}" Content="{name}" Style="{{StaticResource SprawlChip}}" Background="{}" Foreground="{}"/>"##, if sort == name { "#97C8CE" } else { "#14FFFFFF" }, if sort == name { "#000000" } else { "#FFE6EAF2" });
        let mut x = format!(r##"<Grid xmlns="a" xmlns:x="b"><StackPanel Orientation="Horizontal">{}{}</StackPanel><ScrollViewer x:Name="FnScroll"><StackPanel Spacing="4">"##, chip("Size"), chip("Risk"));
        for &i in order {
            x += &format!(
                r##"<Border x:Name="AdvRow{i}"><Grid><Grid.ColumnDefinitions><ColumnDefinition Width="40"/><ColumnDefinition Width="*"/></Grid.ColumnDefinitions><CheckBox x:Name="AdvT{i}" IsChecked="{}"/><StackPanel Grid.Column="1"><Button x:Name="AdvF{i}" Background="{}"><TextBlock Text="Finding {i}"/></Button>{}</StackPanel></Grid></Border>"##,
                if ticked.contains(&i) { "True" } else { "False" },
                if open == Some(i) { "#FF262C3D" } else { "#0CFFFFFF" },
                if open == Some(i) { r##"<Border Padding="8"><TextBlock Text="What it is, and what removing it costs."/></Border>"## } else { "" },
            );
        }
        x += &format!(r##"</StackPanel></ScrollViewer><TextBlock x:Name="AdvCount" Text="{} ticked"/></Grid>"##, ticked.len());
        x
    }

    #[test]
    fn a_tick_changes_the_box_and_the_count_and_loads_nothing() {
        let order = [0, 1, 2, 3];
        let got = ops(&panel(&[], None, &order, "Size"), &panel(&[2], None, &order, "Size")).unwrap();
        assert_eq!(got, ["set [1, 0, 2, 0, 0] IsChecked=True", "set [2] Text=1 ticked"]);
        // nothing changed: nothing to do
        assert_eq!(ops(&panel(&[2], None, &order, "Size"), &panel(&[2], None, &order, "Size")).unwrap(), Vec::<String>::new());
    }

    #[test]
    fn opening_a_row_keeps_its_button_and_loads_the_details_alone() {
        let order = [0, 1, 2, 3];
        let got = ops(&panel(&[], None, &order, "Size"), &panel(&[], Some(1), &order, "Size")).unwrap();
        assert_eq!(got, ["insert [1, 0, 1, 0, 1]/1", "set [1, 0, 1, 0, 1, 0] Background=#FF262C3D"]);
        // another row opened: the first one's details go, the other's come, both buttons stay
        let got = ops(&panel(&[], Some(1), &order, "Size"), &panel(&[], Some(3), &order, "Size")).unwrap();
        assert_eq!(got, ["remove [1, 0, 1, 0, 1]/1", "set [1, 0, 1, 0, 1, 0] Background=#0CFFFFFF", "insert [1, 0, 3, 0, 1]/1", "set [1, 0, 3, 0, 1, 0] Background=#FF262C3D"]);
    }

    #[test]
    fn a_chip_keeps_itself_and_the_rows_are_put_in_their_new_order() {
        let got = ops(&panel(&[1], None, &[0, 1, 2, 3], "Size"), &panel(&[1], None, &[3, 2, 1, 0], "Risk")).unwrap();
        // the chips: colours only; the rows are the same ones (their names say so), in another order
        assert_eq!(&got[..4], ["set [0, 0] Background=#14FFFFFF", "set [0, 0] Foreground=#FFE6EAF2", "set [0, 1] Background=#97C8CE", "set [0, 1] Foreground=#000000"]);
        assert_eq!(&got[4..], ["order [1, 0] 0 new"]);
        // a long list in another order: kept, nothing loaded
        let long: Vec<usize> = (0..60).collect();
        let back: Vec<usize> = (0..60).rev().collect();
        let got = ops(&panel(&[], None, &long, "Size"), &panel(&[], None, &back, "Size")).unwrap();
        assert_eq!(got, ["order [1, 0] 0 new"]);
        // the rows are others: one load for the list, not one for each row
        let other: Vec<usize> = (100..160).collect();
        assert_eq!(ops(&panel(&[], None, &long, "Size"), &panel(&[], None, &other, "Size")).unwrap(), ["replace [1]/0"]);
    }

    #[test]
    fn rows_that_go_and_come_leave_the_others_alone() {
        // a filter took rows 1 and 2 out
        let got = ops(&panel(&[], None, &[0, 1, 2, 3], "Size"), &panel(&[], None, &[0, 3], "Size")).unwrap();
        assert_eq!(got, ["remove [1, 0]/2", "remove [1, 0]/1"]);
        // and back, with row 3 ticked meanwhile: the places after the new rows are the new ones
        let got = ops(&panel(&[], None, &[0, 3], "Size"), &panel(&[3], None, &[0, 1, 2, 3], "Size")).unwrap();
        assert_eq!(got, ["insert [1, 0]/1", "insert [1, 0]/2", "set [1, 0, 3, 0, 0] IsChecked=True", "set [2] Text=1 ticked"]);
        // the list emptied, and filled
        assert_eq!(ops(&panel(&[], None, &[0, 1], "Size"), &panel(&[], None, &[], "Size")).unwrap(), ["remove [1, 0]/1", "remove [1, 0]/0"]);
        assert_eq!(ops(&panel(&[], None, &[], "Size"), &panel(&[], None, &[5], "Size")).unwrap(), ["insert [1, 0]/0"]);
    }

    /// A table's rows as `fill` writes them: one Grid per item, named by the item.
    fn rows(items: &[(usize, usize)], selected: usize) -> String {
        let mut x = String::from(r##"<StackPanel xmlns="a" xmlns:x="b" Margin="0,0,0,0">"##);
        for (k, &(n, depth)) in items.iter().enumerate() {
            let bg = if n == selected { "#FF2C3B63" } else if k % 2 == 1 { "#08FFFFFF" } else { "Transparent" };
            x += &format!(r##"<Grid x:Name="TRow{n}" Height="24" Background="{bg}"><Grid Margin="{},0,6,0"><TextBlock Text="item {n}"/></Grid><TextBlock Grid.Column="2" Text="{n} KB"/></Grid>"##, 8 + depth * 16);
        }
        x + "</StackPanel>"
    }

    #[test]
    fn a_table_row_selected_or_a_folder_opened_changes_only_those_rows() {
        let list: Vec<(usize, usize)> = (0..30).map(|n| (n, 1)).collect();
        // another row selected: two backgrounds
        assert_eq!(ops(&rows(&list, 3), &rows(&list, 4)).unwrap(), ["set [3] Background=#08FFFFFF", "set [4] Background=#FF2C3B63"]);
        // row 3 opened: its three children come after it; the window keeps 30 rows, so the
        // last three go, and the rows after the new ones change stripe
        let mut open = list[..4].to_vec();
        open.extend([(100, 2), (101, 2), (102, 2)]);
        open.extend(&list[4..27]);
        let got = ops(&rows(&list, 3), &rows(&open, 3)).unwrap();
        assert_eq!(&got[..6], ["remove []/29", "remove []/28", "remove []/27", "insert []/4", "insert []/5", "insert []/6"]);
        assert!(got[6..].iter().all(|o| o.contains("Background")), "{got:?}");
        // and closed again
        let got = ops(&rows(&open, 3), &rows(&list, 3)).unwrap();
        assert_eq!(&got[..6], ["remove []/6", "remove []/5", "remove []/4", "insert []/27", "insert []/28", "insert []/29"]);
        // sorted: the rows are put in their new order, none loaded; then the stripes
        let rev: Vec<(usize, usize)> = list.iter().rev().copied().collect();
        let got = ops(&rows(&list, 3), &rows(&rev, 3)).unwrap();
        assert_eq!(got[0], "order [] 0 new");
        assert!(got[1..].iter().all(|o| o.contains("Background")), "{got:?}");
        // sorted with other rows coming into the window: those are loaded
        let other: Vec<(usize, usize)> = (20..40).rev().map(|n| (n, 1)).collect();
        assert_eq!(ops(&rows(&list, 3), &rows(&other, 3)).unwrap()[0], "order [] 10 new");
        // too many new ones: the list is loaded whole
        let far: Vec<(usize, usize)> = (25..60).rev().map(|n| (n, 1)).collect();
        assert!(ops(&rows(&list, 3), &rows(&far, 3)).is_none());
        // a name twice: by place too
        let dup = r#"<StackPanel xmlns="a" xmlns:x="b"><Grid x:Name="a"/><Grid x:Name="a"/></StackPanel>"#;
        let one = r#"<StackPanel xmlns="a" xmlns:x="b"><Grid x:Name="a"/></StackPanel>"#;
        assert_eq!(ops(dup, one).unwrap(), ["remove []/1"]);
    }

    #[test]
    fn what_cant_be_set_is_loaded_again() {
        let ns = r#"xmlns="a" xmlns:x="b""#;
        let one = |old: &str, new: &str| ops(&format!("<StackPanel {ns}>{old}</StackPanel>"), &format!("<StackPanel {ns}>{new}</StackPanel>")).unwrap();
        // an attribute that can't be set, one that went, another name, another tag
        assert_eq!(one(r#"<TextBlock FontSize="{$fs12}"/>"#, r#"<TextBlock FontSize="{$fs13}"/>"#), ["replace []/0"]);
        assert_eq!(one(r#"<Button Content="a" Foreground="White"/>"#, r#"<Button Content="a"/>"#), ["replace []/0"]);
        assert_eq!(one(r#"<Button x:Name="AdvT1"/>"#, r#"<Button x:Name="AdvT2"/>"#), ["replace []/0"]);
        assert_eq!(one(r#"<Border/>"#, r#"<Grid/>"#), ["replace []/0"]);
        // one that came, and can be set
        assert_eq!(one(r#"<Button Content="a"/>"#, r#"<Button Content="a" Foreground="White"/>"#), ["set [0] Foreground=White"]);
        // a button's words are an attribute or an element, not one become the other
        assert_eq!(one(r#"<Button><Grid/></Button>"#, r#"<Button Content="a"/>"#), ["replace []/0"]);
        assert_eq!(one(r#"<Button Content="a"/>"#, r#"<Button Content="b"/>"#), ["set [0] Content=b"]);
        // a property element changed: its owner again
        assert_eq!(one(r#"<Grid><Grid.ColumnDefinitions><ColumnDefinition Width="1*"/></Grid.ColumnDefinitions><Border/></Grid>"#, r#"<Grid><Grid.ColumnDefinitions><ColumnDefinition Width="2*"/></Grid.ColumnDefinitions><Border/></Grid>"#), ["replace []/0"]);
        // inside a piece (a TextBlock's Runs, a ComboBox's items): the piece again
        assert_eq!(one(r#"<TextBlock><Run Text="1 GiB"/></TextBlock>"#, r#"<TextBlock><Run Text="2 GiB"/></TextBlock>"#), ["replace []/0"]);
        assert_eq!(one(r#"<ComboBox SelectedIndex="0"><ComboBoxItem Content="a"/></ComboBox>"#, r#"<ComboBox SelectedIndex="1"><ComboBoxItem Content="a"/></ComboBox>"#), ["set [0] SelectedIndex=1"]);
        assert_eq!(one(r#"<ComboBox SelectedIndex="0"><ComboBoxItem Content="a"/></ComboBox>"#, r#"<ComboBox SelectedIndex="0"><ComboBoxItem Content="b"/></ComboBox>"#), ["replace []/0"]);
        // a Border's one child
        assert_eq!(one(r#"<Border/>"#, r#"<Border><Grid/></Border>"#), ["insert [0]/0"]);
        assert_eq!(one(r#"<Border><Grid/></Border>"#, r#"<Border/>"#), ["remove [0]/0"]);
        assert_eq!(one(r#"<Border><Grid/></Border>"#, r#"<Border><StackPanel/></Border>"#), ["replace [0]/0"]);
        // another root: everything
        assert!(ops(&format!("<StackPanel {ns}/>"), &format!("<Grid {ns}/>")).is_none());
        assert!(ops(&format!(r#"<StackPanel {ns} Spacing="4"/>"#), &format!(r#"<StackPanel {ns} Spacing="5"/>"#)).is_none());
    }

    #[test]
    fn a_tick_box_out_of_reach_is_loaded_every_time() {
        let ns = r#"xmlns="a" xmlns:x="b""#;
        // inside a flyout (a property element): its button is loaded again, changed or not,
        // since the box's state can't be set from outside
        let x = format!(r#"<StackPanel {ns}><Button Content="More"><Button.Flyout><Flyout><CheckBox IsChecked="False"/></Flyout></Button.Flyout></Button><TextBlock Text="a"/></StackPanel>"#);
        assert_eq!(ops(&x, &x).unwrap(), ["replace []/0"]);
        // within reach: nothing
        let y = format!(r#"<StackPanel {ns}><Button><CheckBox IsChecked="False"/></Button></StackPanel>"#);
        assert_eq!(ops(&y, &y).unwrap(), Vec::<String>::new());
    }

    #[test]
    fn elements_are_found_by_place_or_inside_their_piece() {
        let d = Doc::parse(&panel(&[], Some(1), &[0, 1], "Size")).unwrap();
        let find = |name: &str| {
            let mut p = Vec::new();
            locate(&d.src, &d.root, name, &mut p).map(|inside| (p, inside))
        };
        assert_eq!(find("AdvF1"), Some((vec![1, 0, 1, 0, 1, 0], false)));
        assert_eq!(find("FnScroll"), Some((vec![1], false)));
        assert_eq!(find("AdvCount"), Some((vec![2], false)));
        assert_eq!(find("Nothing"), None);
        // inside a piece: the piece's place, to be asked by name
        let d = Doc::parse(r#"<Grid xmlns="a" xmlns:x="b"><Border/><ComboBox><ComboBoxItem x:Name="It"/></ComboBox></Grid>"#).unwrap();
        let mut p = Vec::new();
        assert_eq!(locate(&d.src, &d.root, "It", &mut p), Some(true));
        assert_eq!(p, [1]);
    }

    /// A small generator of numbers, the same every run.
    struct Rng(u64);

    impl Rng {
        fn next(&mut self, n: usize) -> usize {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            (self.0 % n as u64) as usize
        }
    }

    fn random(r: &mut Rng, depth: usize, names: &mut usize) -> String {
        let name = |r: &mut Rng, names: &mut usize| {
            if r.next(3) == 0 {
                *names += 1;
                format!(r#" x:Name="n{}""#, r.next(6))
            } else {
                String::new()
            }
        };
        let colour = |r: &mut Rng| ["#FF000000", "#FFFFFFFF", "#14FFFFFF"][r.next(3)];
        match if depth == 0 { 3 + r.next(3) } else { r.next(6) } {
            0 | 1 => {
                let tag = ["StackPanel", "Grid"][r.next(2)];
                let kids: String = (0..r.next(5)).map(|_| random(r, depth - 1, names)).collect();
                let props = if tag == "Grid" && r.next(2) == 0 { format!(r#"<Grid.ColumnDefinitions><ColumnDefinition Width="{}*"/></Grid.ColumnDefinitions>"#, 1 + r.next(2)) } else { String::new() };
                format!(r#"<{tag}{} Background="{}">{props}{kids}</{tag}>"#, name(r, names), colour(r))
            }
            2 => {
                let tag = ["Border", "Button", "ScrollViewer"][r.next(3)];
                let kid = if r.next(3) == 0 { String::new() } else { random(r, depth - 1, names) };
                format!(r#"<{tag}{}>{kid}</{tag}>"#, name(r, names))
            }
            3 => format!(r#"<TextBlock{} Text="t{}" FontSize="{}"/>"#, name(r, names), r.next(4), 12 + r.next(2)),
            4 => format!(r#"<CheckBox{} IsChecked="{}"/>"#, name(r, names), ["True", "False"][r.next(2)]),
            _ => format!(r#"<TextBlock{}><Run Text="r{}"/></TextBlock>"#, name(r, names), r.next(3)),
        }
    }

    #[test]
    fn any_two_trees_the_ops_turn_one_into_the_other() {
        let mut r = Rng(0x9E37_79B9_7F4A_7C15);
        let (mut in_place, mut whole) = (0, 0);
        for _ in 0..4000 {
            let mut names = 0;
            let root = |r: &mut Rng, names: &mut usize| {
                let kids: String = (0..r.next(6)).map(|_| random(r, 3, names)).collect();
                format!(r#"<StackPanel xmlns="a" xmlns:x="b">{kids}</StackPanel>"#)
            };
            let (old, new) = (root(&mut r, &mut names), root(&mut r, &mut names));
            // `ops` checks that the result is the new tree
            match ops(&old, &new) {
                Some(_) => in_place += 1,
                None => whole += 1,
            }
            // and a tree against itself asks for nothing
            assert_eq!(ops(&new, &new).unwrap(), Vec::<String>::new());
        }
        // a whole load is asked for only when very much of a tree differs
        assert!(in_place > 3000, "{in_place} in place, {whole} whole");
    }

    #[test]
    fn a_small_change_is_made_in_place() {
        // one tree, and the same with one thing changed at random: the ops give it
        let mut r = Rng(0x2545_F491_4F6C_DD1D);
        for round in 0..3000 {
            let mut names = 0;
            let kids: Vec<String> = (0..2 + r.next(8)).map(|_| random(&mut r, 2, &mut names)).collect();
            let mut changed = kids.clone();
            match r.next(4) {
                0 => {
                    changed.remove(r.next(kids.len()));
                }
                1 => changed.insert(r.next(kids.len() + 1), random(&mut r, 2, &mut names)),
                2 => {
                    let i = r.next(kids.len());
                    changed[i] = changed[i].replacen("IsChecked=\"True\"", "IsChecked=\"False\"", 1).replacen("Text=\"t0\"", "Text=\"t9\"", 1);
                }
                _ => changed.swap(0, kids.len() - 1),
            }
            let wrap = |k: &[String]| format!(r#"<StackPanel xmlns="a" xmlns:x="b">{}</StackPanel>"#, k.concat());
            assert!(ops(&wrap(&kids), &wrap(&changed)).is_some(), "round {round}: the root is the same");
        }
    }
}
