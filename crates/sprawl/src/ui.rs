//! XAML markup for the window and the table rows (loaded with XamlReader at run time).

pub const NS: &str = r##"xmlns="http://schemas.microsoft.com/winfx/2006/xaml/presentation" xmlns:x="http://schemas.microsoft.com/winfx/2006/xaml""##;

/// Text for a XAML attribute value. File names are anyone's choice, so besides the XML
/// characters: control characters (not allowed in XML) become U+FFFD, and a leading `{` gets
/// the `{}` escape so the name isn't read as a markup extension (`{Binding ...}`).
pub fn esc(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 8);
    if s.starts_with('{') {
        out.push_str("{}");
    }
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            c if (c as u32) < 0x20 || matches!(c, '\u{FFFE}' | '\u{FFFF}') => out.push('\u{FFFD}'),
            c => out.push(c),
        }
    }
    out
}


/// The Markdown models write, as XAML: paragraphs (a single line break kept), `#` headings,
/// `-`/`*`/`1.` lists with their nesting, fenced code, tables, `>` quotes, `---` rules, and
/// folder trees drawn with `├──`/`│` (or `|--`), kept line by line in a fixed-width font so
/// they line up. Inline: **bold**, *italic*, ~~struck~~, `code` and `[links](target)`.
/// Everything else is plain text, escaped.
pub(crate) fn markdown(text: &str, size: u32, colour: &str) -> String {
    let mut out = String::from(r##"<StackPanel Spacing="4">"##);
    let lines: Vec<&str> = text.lines().collect();
    let block = |out: &mut String, runs: String, extra: &str| {
        out.push_str(&format!(r##"<TextBlock TextWrapping="Wrap" IsTextSelectionEnabled="True" FontSize="{size}" Foreground="{colour}"{extra}>{runs}</TextBlock>"##));
    };
    let mono = |out: &mut String, lines: &[&str]| {
        // XAML's `{}` escape is only for the start of the text, so a later line starting with
        // `{` (code) doesn't get one
        let body = lines.iter().enumerate().map(|(n, l)| {
            let e = esc(l.trim_end());
            if n > 0 && l.starts_with('{') { e[2..].to_string() } else { e }
        });
        let body = body.collect::<Vec<_>>().join("&#10;");
        out.push_str(&format!(
            r##"<Border Padding="8,6" CornerRadius="4" Background="#33000000"><ScrollViewer HorizontalScrollBarVisibility="Auto" VerticalScrollBarVisibility="Disabled" HorizontalScrollMode="Auto" VerticalScrollMode="Disabled"><TextBlock FontFamily="Cascadia Mono, Consolas" FontSize="{size}" IsTextSelectionEnabled="True" Foreground="{colour}" Text="{body}"/></ScrollViewer></Border>"##
        ));
    };
    let mut para: Vec<&str> = Vec::new();
    let flush = |out: &mut String, para: &mut Vec<&str>| {
        if !para.is_empty() {
            // a line break inside a paragraph is kept: answers list "Size: ..." under "Path: ..."
            block(out, para.iter().map(|l| inline(l)).collect::<Vec<_>>().join("<LineBreak/>"), "");
            para.clear();
        }
    };
    let mut i = 0;
    while i < lines.len() {
        let line = lines[i];
        let t = line.trim();
        if let Some(fence) = ["```", "~~~"].into_iter().find(|f| t.starts_with(f)) {
            flush(&mut out, &mut para);
            let end = lines[i + 1..].iter().position(|l| l.trim().starts_with(fence)).map_or(lines.len(), |p| i + 1 + p);
            mono(&mut out, &lines[i + 1..end]);
            i = end + 1;
            continue;
        }
        if is_tree(line) {
            // its root line just above (a folder), taken as a paragraph line, goes with it; a
            // sentence leading into it ("Here is the tree:") stays a sentence
            let root = (i > 0 && para.last() == Some(&lines[i - 1].trim()) && !lines[i - 1].trim_end().ends_with(':') && lines[i - 1].len() < 100).then(|| lines[i - 1]);
            if root.is_some() {
                para.pop();
            }
            flush(&mut out, &mut para);
            let mut end = i;
            while end < lines.len() && !lines[end].trim().is_empty() && (is_tree(lines[end]) || lines[end].starts_with("    ")) {
                end += 1;
            }
            let tree: Vec<&str> = root.into_iter().chain(lines[i..end].iter().copied()).collect();
            mono(&mut out, &tree);
            i = end;
            continue;
        }
        if t.starts_with('|') && lines.get(i + 1).is_some_and(|n| is_table_rule(n)) {
            flush(&mut out, &mut para);
            let mut end = i + 2;
            while end < lines.len() && lines[end].trim().starts_with('|') {
                end += 1;
            }
            match table(lines[i], &lines[i + 2..end], size, colour) {
                Some(t) => out.push_str(&t),
                // too wide to lay out: the rows as written, so no cell is lost
                None => mono(&mut out, &lines[i..end]),
            }
            i = end;
            continue;
        }
        i += 1;
        if t.is_empty() {
            flush(&mut out, &mut para);
            continue;
        }
        if t.len() >= 3 && (t.chars().all(|c| c == '-') || t.chars().all(|c| c == '*') || t.chars().all(|c| c == '_')) {
            flush(&mut out, &mut para);
            out.push_str(r##"<Border Height="1" Background="#22FFFFFF" Margin="0,4"/>"##);
            continue;
        }
        let heading = t.trim_start_matches('#');
        if heading.len() < t.len() && heading.starts_with(' ') {
            flush(&mut out, &mut para);
            block(&mut out, inline(heading.trim()), r##" FontWeight="SemiBold""##);
            continue;
        }
        if let Some(q) = t.strip_prefix('>') {
            flush(&mut out, &mut para);
            out.push_str(&format!(
                r##"<Border BorderBrush="#44FFFFFF" BorderThickness="2,0,0,0" Padding="8,0,0,0"><TextBlock TextWrapping="Wrap" IsTextSelectionEnabled="True" FontSize="{size}" Foreground="#FFB9C2D6">{}</TextBlock></Border>"##,
                inline(q.trim())
            ));
            continue;
        }
        // two spaces (or a tab) of indent a level, for nested lists
        let depth = (line.len() - line.trim_start().len() + line.chars().take_while(|c| c.is_whitespace()).filter(|c| *c == '\t').count()) / 2;
        let margin = format!(r##" Margin="{},0,0,0""##, 6 + 16 * depth.min(6));
        let bullet = ["- ", "* ", "+ "].iter().find_map(|b| t.strip_prefix(b));
        let number = t.split_once(". ").or_else(|| t.split_once(") ")).filter(|(n, _)| !n.is_empty() && n.len() < 4 && n.bytes().all(|b| b.is_ascii_digit()));
        if let Some(rest) = bullet {
            flush(&mut out, &mut para);
            let dot = if depth == 0 { "•" } else { "◦" };
            let (mark, rest) = match rest.strip_prefix("[ ] ").map(|r| ("☐", r)).or_else(|| rest.strip_prefix("[x] ").or_else(|| rest.strip_prefix("[X] ")).map(|r| ("☑", r))) {
                Some((m, r)) => (m, r),
                None => (dot, rest),
            };
            block(&mut out, format!(r##"<Run Text="{mark}  "/>{}"##, inline(rest)), &margin);
        } else if let Some((n, rest)) = number {
            flush(&mut out, &mut para);
            block(&mut out, format!(r##"<Run Text="{n}.  "/>{}"##, inline(rest)), &margin);
        } else {
            para.push(t);
        }
    }
    flush(&mut out, &mut para);
    out.push_str("</StackPanel>");
    out
}

/// A line of a folder tree: box-drawing characters (`├── `, `└── `, `│`), or the ASCII kind
/// (`|-- `, `` `-- ``, `+-- `) after any indent.
fn is_tree(line: &str) -> bool {
    let t = line.trim_start();
    t.chars().next().is_some_and(|c| ('\u{2500}'..='\u{257F}').contains(&c)) || ["|-- ", "|__ ", "`-- ", "+-- ", "\\-- "].iter().any(|p| t.starts_with(p))
}

/// The line under a table's head: `|---|:---:|`.
fn is_table_rule(line: &str) -> bool {
    let t = line.trim();
    t.starts_with('|') && t.contains('-') && t.chars().all(|c| matches!(c, '|' | '-' | ':' | ' '))
}

/// The most columns a table is laid out with; a wider one is shown as written.
const TABLE_COLUMNS: usize = 12;

/// A backslash right after this is part of a path (`C:\Cache\*.log`, `x\|`), not an escape.
fn path_char(c: Option<&char>) -> bool {
    c.is_some_and(|c| c.is_alphanumeric() || matches!(c, '\\' | '/' | ':' | '.' | '_' | '-' | '$' | '~' | ')' | ']' | '}' | '*' | '?'))
}

/// The cells of a table row, `\|` kept as a bar inside a cell (but a path's backslash before a
/// bar stays, and the bar divides).
fn cells(row: &str) -> Vec<String> {
    let t = row.trim();
    let t = t.strip_prefix('|').unwrap_or(t);
    let t = t.strip_suffix('|').unwrap_or(t);
    let mut out = vec![String::new()];
    let mut chars = t.chars().peekable();
    let mut prev: Option<char> = None;
    while let Some(c) = chars.next() {
        let before = prev;
        prev = Some(c);
        match c {
            '\\' if chars.peek() == Some(&'|') && !path_char(before.as_ref()) => {
                out.last_mut().unwrap().push('|');
                chars.next();
            }
            '|' => out.push(String::new()),
            c => out.last_mut().unwrap().push(c),
        }
    }
    out.into_iter().map(|c| c.trim().to_string()).collect()
}

/// A table as a grid: the head in semibold, then its rows; wider than the pane, it scrolls.
/// None when it has more columns than are laid out: nothing in it may be dropped.
fn table(head: &str, rows: &[&str], size: u32, colour: &str) -> Option<String> {
    let head = cells(head);
    let rows: Vec<Vec<String>> = rows.iter().map(|r| cells(r)).collect();
    let n = rows.iter().map(Vec::len).chain(std::iter::once(head.len())).max().unwrap_or(1);
    if n > TABLE_COLUMNS {
        return None;
    }
    let mut x = format!(
        r##"<Border CornerRadius="4" Background="#14FFFFFF" Padding="8,6"><ScrollViewer HorizontalScrollBarVisibility="Auto" VerticalScrollBarVisibility="Disabled" HorizontalScrollMode="Auto" VerticalScrollMode="Disabled"><Grid ColumnSpacing="16" RowSpacing="4"><Grid.ColumnDefinitions>{}</Grid.ColumnDefinitions><Grid.RowDefinitions>{}</Grid.RowDefinitions>"##,
        r##"<ColumnDefinition Width="Auto"/>"##.repeat(n),
        r##"<RowDefinition Height="Auto"/>"##.repeat(rows.len() + 1)
    );
    for (r, row) in std::iter::once(&head).chain(rows.iter()).enumerate() {
        for (c, cell) in row.iter().take(n).enumerate() {
            let weight = if r == 0 { r##" FontWeight="SemiBold""## } else { "" };
            x += &format!(r##"<TextBlock Grid.Row="{r}" Grid.Column="{c}" FontSize="{size}" Foreground="{colour}" IsTextSelectionEnabled="True" MaxWidth="420" TextWrapping="Wrap"{weight}>{}</TextBlock>"##, inline(cell));
        }
    }
    x += "</Grid></ScrollViewer></Border>";
    Some(x)
}

/// Runs for one line: `**bold**`, `*italic*` (only around a word, so `*.mp4` stays as it is),
/// `~~struck~~` (only at a word's edges, so `a~~b` stays), `` `code` `` (only when it closes on
/// the line, and not inside a word or path) and `[text](target)` (the text, with the target
/// after it when it says something else; a target whose brackets don't balance is shown as
/// written). `\*`, `\|` and `` \` `` are the characters themselves after a space or a mark;
/// after a name or path the backslash is the path's (`C:\Cache\*.log`, `npm-cache\_cacache`),
/// and a star right after a path separator is a wildcard (`C:\x\**\*.log`).
fn inline(s: &str) -> String {
    let chars: Vec<char> = s.chars().collect();
    let mut out = String::new();
    let mut cur = String::new();
    let (mut bold, mut italic, mut code, mut struck) = (false, false, false, false);
    let emit = |out: &mut String, cur: &mut String, bold: bool, italic: bool, code: bool, struck: bool| {
        if cur.is_empty() {
            return;
        }
        let mut attrs = String::new();
        if bold {
            attrs.push_str(r##" FontWeight="SemiBold""##);
        }
        if italic {
            attrs.push_str(r##" FontStyle="Italic""##);
        }
        if code {
            attrs.push_str(r##" FontFamily="Cascadia Mono, Consolas""##);
        }
        if struck {
            attrs.push_str(r##" TextDecorations="Strikethrough""##);
        }
        out.push_str(&format!(r##"<Run Text="{}"{attrs}/>"##, esc(cur)));
        cur.clear();
    };
    let word = |c: Option<&char>| c.is_some_and(|c| c.is_alphanumeric());
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        let next = chars.get(i + 1);
        let prev = i.checked_sub(1).map(|p| &chars[p]);
        if c == '`' && (code || (!path_char(prev) && chars[i + 1..].contains(&'`'))) {
            emit(&mut out, &mut cur, bold, italic, code, struck);
            code = !code;
        } else if code {
            cur.push(c);
        } else if c == '\\' && matches!(next, Some('*') | Some('|') | Some('`')) && !path_char(prev) {
            cur.push(chars[i + 1]);
            i += 1;
        } else if c == '*' && matches!(prev, Some('\\') | Some('/')) {
            // a wildcard in a path: `C:\x\*.log`, `C:\x\**\y`
            cur.push(c);
            if next == Some(&'*') {
                cur.push('*');
                i += 1;
            }
        } else if c == '*' && next == Some(&'*') {
            emit(&mut out, &mut cur, bold, italic, code, struck);
            bold = !bold;
            i += 1;
        } else if c == '~' && next == Some(&'~') && tilde_mark(&chars, i, struck) {
            emit(&mut out, &mut cur, bold, italic, code, struck);
            struck = !struck;
            i += 1;
        } else if c == '*' && ((!italic && word(next) && chars[i + 1..].contains(&'*')) || (italic && i > 0 && word(chars.get(i - 1)))) {
            emit(&mut out, &mut cur, bold, italic, code, struck);
            italic = !italic;
        } else if let Some((text, target, len)) = (c == '[').then(|| link(&chars[i..])).flatten() {
            cur.push_str(&text);
            if target != text && !target.is_empty() {
                emit(&mut out, &mut cur, bold, italic, code, struck);
                out.push_str(&format!(r##"<Run Text=" ({})" Foreground="#FF8E97AA"/>"##, esc(&target)));
            }
            i += len;
            continue;
        } else {
            cur.push(c);
        }
        i += 1;
    }
    emit(&mut out, &mut cur, bold, italic, code, struck);
    out
}

/// `~~` at `i` strikes through: opening at a word's start with a closing one later, or closing
/// at a word's end. Inside a name (`a~~b`) it is two tildes.
fn tilde_mark(chars: &[char], i: usize, struck: bool) -> bool {
    let before = i.checked_sub(1).map(|p| chars[p]);
    let after = chars.get(i + 2).copied();
    let word = |c: Option<char>| c.is_some_and(|c| c.is_alphanumeric() || c == '~');
    let space = |c: Option<char>| c.is_none_or(char::is_whitespace);
    if struck {
        !space(before) && !word(after)
    } else {
        !word(before) && !space(after) && (i + 2..chars.len().saturating_sub(1)).any(|j| chars[j] == '~' && chars[j + 1] == '~' && tilde_mark(chars, j, true))
    }
}

/// `[text](target)` at the start of `s`: its text, its target and how many characters it took.
/// The target ends at the bracket that balances its own (`(...wiki/Foo_(bar))`); without one
/// it isn't a link.
fn link(s: &[char]) -> Option<(String, String, usize)> {
    let close = s.iter().position(|&c| c == ']')?;
    if s.get(close + 1) != Some(&'(') {
        return None;
    }
    let mut depth = 0usize;
    let end = close + 2 + s[close + 2..].iter().position(|&c| match c {
        '(' => {
            depth += 1;
            false
        }
        ')' if depth == 0 => true,
        ')' => {
            depth -= 1;
            false
        }
        _ => false,
    })?;
    let text: String = s[1..close].iter().collect();
    let target: String = s[close + 2..end].iter().collect();
    (!text.is_empty()).then(|| (text, target.trim().to_string(), end + 1))
}

#[cfg(test)]
mod esc_tests {

    #[test]
    fn markdown_to_xaml() {
        let x = super::markdown("The biggest is **D:\\Games**, at *1.3 TB*.\n\n- one `*.mp4`\n2. two\n\n# Head", 12, "#FFFFFFFF");
        assert!(x.contains(r#"<Run Text="D:\Games" FontWeight="SemiBold"/>"#), "{x}");
        assert!(x.contains(r#"FontStyle="Italic""#));
        assert!(x.contains(r#"<Run Text="*.mp4" FontFamily="Cascadia Mono, Consolas"/>"#), "{x}");
        assert!(x.contains("•") && x.contains(r#"<Run Text="2.  "/>"#));
        assert_eq!(x.matches("<TextBlock").count(), 4);
        // not markup: escaped
        assert!(!super::markdown("<Button x:Name=\"a\"/>", 12, "#FFFFFFFF").contains("<Button"));
        // a lone star stays a star
        assert!(super::inline("files *.log and *.tmp").contains("*.log and *.tmp"));
    }

    #[test]
    fn folder_trees_keep_their_lines() {
        let x = super::markdown("Here is the tree:\nC:\\Users\\me\\\n├── AppData (40 GB)\n│   └── Local\n└── Videos\n\nDone.", 12, "#FFFFFFFF");
        // the sentence stays a paragraph; the root and the tree are one fixed-width block
        assert!(x.contains(r#"<Run Text="Here is the tree:"/>"#), "{x}");
        assert!(x.contains("C:\\Users\\me\\&#10;├── AppData (40 GB)&#10;│   └── Local&#10;└── Videos"), "{x}");
        assert!(x.contains("Cascadia Mono"));
        // the ASCII kind too
        assert!(super::markdown("root\n|-- a\n`-- b", 12, "#FFFFFFFF").contains("root&#10;|-- a&#10;`-- b"));
    }

    #[test]
    fn tables_breaks_links_and_paths() {
        let x = super::markdown("| Folder | Size |\n|---|---:|\n| C:\\a\\_cacache | 2 GB |\n| x \\| y | 1 GB |", 12, "#FFFFFFFF");
        assert!(x.contains("<Grid ColumnSpacing"), "{x}");
        assert!(x.contains(r#"Grid.Row="0" Grid.Column="1""#) && x.contains(r#"Grid.Row="2" Grid.Column="1""#));
        assert!(x.contains(r#"<Run Text="C:\a\_cacache"/>"#), "a backslash in a path stays: {x}");
        assert!(x.contains(r#"<Run Text="x | y"/>"#), "{x}");
        assert!(!x.contains("---"), "the rule row isn't a row");
        let x = super::markdown("```\n{a}\nfn f()\n{\n}\n```", 12, "#FFFFFFFF");
        assert!(x.contains(r#"Text="{}{a}&#10;fn f()&#10;{&#10;}""#), "only the first line gets XAML's escape: {x}");
        // one line break inside a paragraph is kept
        assert!(super::markdown("Path: C:\\x\nSize: 5 GB", 12, "#FFFFFFFF").contains("<LineBreak/>"));
        // a link shows its text, and its target when that says more
        let l = super::inline("see [the cache](C:\\cache) or [x](x)");
        assert!(l.contains(r#"<Run Text="see the cache"/><Run Text=" (C:\cache)""#), "{l}");
        assert!(!l.contains("[x]") && !l.contains(" (x)"), "{l}");
        assert!(super::inline("~~old~~ new").contains(r#"TextDecorations="Strikethrough""#));
        assert!(super::inline(r"a \* b").contains(r#"<Run Text="a * b"/>"#));
        // nesting, quotes and rules
        let n = super::markdown("- top\n  - inner\n> quoted\n\n---", 12, "#FFFFFFFF");
        assert!(n.contains("◦") && n.contains(r#"Margin="22,0,0,0""#), "{n}");
        assert!(n.contains(r#"BorderThickness="2,0,0,0""#) && n.contains(r#"<Border Height="1""#), "{n}");
        // an unclosed fence is still code, line by line
        assert!(super::markdown("```\na\nb", 12, "#FFFFFFFF").contains("a&#10;b"));
    }

    #[test]
    fn paths_globs_and_names_stay_as_written() {
        // Astra 1.0.2 S5 #3: what is shown is the path that was written
        for p in [r"C:\Cache\*.log", r"C:\x\**\*.tmp", r"\\nas\media\*.iso", r"D:\*", r"C:\Users\*\AppData", r"C:\a~~b~~c.txt", r"C:\odd`name.txt", "x~~y"] {
            let x = super::inline(&format!("see {p} now"));
            assert!(x.contains(&super::esc(&format!("see {p} now"))), "{p}: {x}");
        }
        // a path still takes bold and code around it
        assert!(super::inline(r"**C:\Cache\*.log**").contains(r#"<Run Text="C:\Cache\*.log" FontWeight="SemiBold"/>"#));
        assert!(super::inline(r"`C:\Cache\*.log`").contains(r#"<Run Text="C:\Cache\*.log" FontFamily"#));
        assert!(super::inline("~~old~~ new").contains(r#"TextDecorations="Strikethrough""#));
        assert!(super::inline(r"a \* b").contains(r#"<Run Text="a * b"/>"#));
        // a lone backtick is a backtick
        assert!(super::inline("it's a ` mark").contains("it&apos;s a ` mark"));
        // a link's target keeps its own brackets; unbalanced, the link is shown as written
        let l = super::inline("[Foo](https://en.wikipedia.org/wiki/Foo_(bar)) after");
        assert!(l.contains(r#"<Run Text=" (https://en.wikipedia.org/wiki/Foo_(bar))""#) && l.contains(r#"<Run Text=" after"/>"#), "{l}");
        let l = super::inline("[Foo](https://x/(a b");
        assert!(l.contains("[Foo](https://x/(a b"), "{l}");
        // a table too wide to lay out is shown row by row, every cell kept
        let head = (1..=14).map(|n| format!("c{n}")).collect::<Vec<_>>().join(" | ");
        let row = (1..=14).map(|n| format!("v{n}")).collect::<Vec<_>>().join(" | ");
        let rule = vec!["---"; 14].join("|");
        let x = super::markdown(&format!("| {head} |\n|{rule}|\n| {row} |"), 12, "#FFFFFFFF");
        assert!(!x.contains("<Grid ColumnSpacing") && x.contains("v14") && x.contains("c14"), "{x}");
        // a table cell ending a path before its bar
        assert_eq!(super::cells(r"| C:\Cache\| 2 GB |"), vec![r"C:\Cache\".to_string(), "2 GB".to_string()]);
        assert_eq!(super::cells(r"| x \| y | 1 GB |"), vec!["x | y".to_string(), "1 GB".to_string()]);
    }

    #[test]
    fn escapes_names() {
        assert_eq!(super::esc(r#"a<b>&"c'"#), "a&lt;b&gt;&amp;&quot;c&apos;");
        assert_eq!(super::esc("{Binding x}"), "{}{Binding x}");
        assert_eq!(super::esc("a{b}"), "a{b}");
        assert_eq!(super::esc("x\u{1}y"), "x\u{FFFD}y");
    }
}

thread_local! {
    static SHORTCUTS: std::cell::RefCell<Vec<(String, String)>> = const { std::cell::RefCell::new(Vec::new()) };
}

fn note_shortcut(text: &str, key: &str, mods: &str) {
    if key.is_empty() {
        return;
    }
    let key = key.strip_prefix("Number").unwrap_or(key);
    let mods = mods.replace("Control", "Ctrl").replace("Menu", "Alt").replace(',', "+");
    let combo = if mods.is_empty() { key.to_string() } else { format!("{mods}+{key}") };
    let text = text.replace("&quot;", "\"").replace("...", "");
    SHORTCUTS.with(|s| s.borrow_mut().push((combo, text)));
}

/// Sprawl's logo as in `assets/sprawl-256.png` (the skyline with lit windows over its
/// reflection, on the orange-to-purple tile), drawn in XAML so it is sharp at `px` pixels.
pub(crate) fn logo(px: u32) -> String {
    // (left, top, right) of each building on a 256 tile; they all stand on y = 188
    const B: [(u32, u32, u32); 5] = [(33, 128, 70), (70, 93, 106), (106, 112, 145), (145, 82, 180), (180, 138, 221)];
    // lit windows (x, y, colour)
    const W: [(u32, u32, &str); 16] = [
        (42, 142, "#FFFF6F61"), (56, 160, "#FFFF6F61"), (42, 170, "#FFF04E7A"),
        (80, 104, "#FFFF7A45"), (92, 122, "#FFF04E7A"), (80, 140, "#FFFF6F61"), (92, 160, "#FFE5528F"),
        (116, 124, "#FFFF6F61"), (130, 148, "#FFF04E7A"), (116, 166, "#FFFF7A45"),
        (155, 94, "#FFF04E7A"), (166, 116, "#FFFF6F61"), (155, 136, "#FFE5528F"), (166, 158, "#FFFF6F61"),
        (190, 150, "#FFB04CC8"), (204, 150, "#FFE5528F"),
    ];
    let mut x = format!(
        r##"<Viewbox Width="{px}" Height="{px}"><Canvas Width="256" Height="256"><Rectangle Width="256" Height="256" RadiusX="56" RadiusY="56"><Rectangle.Fill><LinearGradientBrush StartPoint="0,0" EndPoint="1,1"><GradientStop Color="#FFFFA23F" Offset="0"/><GradientStop Color="#FFFF5A5F" Offset="0.5"/><GradientStop Color="#FF9B4FE0" Offset="1"/></LinearGradientBrush></Rectangle.Fill></Rectangle>"##
    );
    for (l, t, r) in B {
        // the reflection: a short, faint copy under the street
        x += &format!(r##"<Rectangle Canvas.Left="{l}" Canvas.Top="192" Width="{}" Height="{}" RadiusX="3" RadiusY="3" Fill="#FFFFFFFF" Opacity="0.16"/>"##, r - l - 1, ((188 - t) / 2).min(44));
        x += &format!(r##"<Rectangle Canvas.Left="{l}" Canvas.Top="{t}" Width="{}" Height="{}" RadiusX="4" RadiusY="4" Fill="White"/>"##, r - l - 1, 188 - t);
    }
    for (wx, wy, c) in W {
        x += &format!(r##"<Rectangle Canvas.Left="{wx}" Canvas.Top="{wy}" Width="9" Height="9" RadiusX="1" RadiusY="1" Fill="{c}"/>"##);
    }
    x += "</Canvas></Viewbox>";
    x
}

/// The keyboard shortcuts as the Help menu shows them: groups in two columns, each row with
/// its keys as key caps and what they do.
pub(crate) fn shortcuts_xaml() -> String {
    // keys: "Ctrl+Shift+F" as three caps; a mouse gesture as plain words in one cap
    let caps = |k: &str| -> String {
        let parts: Vec<&str> = if k.contains(' ') { vec![k] } else { k.split('+').collect() };
        parts
            .iter()
            .map(|p| {
                let p = match *p {
                    "Escape" => "Esc",
                    "Delete" => "Del",
                    "PageUp" => "Page Up",
                    "PageDown" => "Page Down",
                    x => x,
                };
                format!(r##"<Border Padding="6,0,6,1" MinWidth="22" CornerRadius="4" Background="#1FFFFFFF" BorderBrush="#30FFFFFF" BorderThickness="1,1,1,2"><TextBlock Text="{}" FontSize="11" FontWeight="SemiBold" Foreground="#FFE4E8F2" HorizontalAlignment="Center"/></Border>"##, esc(p))
            })
            .collect()
    };
    let group = |title: &str, rows: &[(String, String)]| -> String {
        let mut x = format!(r##"<StackPanel Spacing="3" Margin="0,0,0,12"><TextBlock Text="{}" FontSize="12" FontWeight="SemiBold" Foreground="#FF8FB4FF" Margin="0,0,0,2"/>"##, esc(title));
        for (k, what) in rows {
            x += &format!(
                r##"<Grid ColumnSpacing="12"><Grid.ColumnDefinitions><ColumnDefinition Width="140"/><ColumnDefinition Width="*"/></Grid.ColumnDefinitions><StackPanel Orientation="Horizontal" Spacing="3" VerticalAlignment="Center">{}</StackPanel><TextBlock Grid.Column="1" Text="{}" FontSize="12.5" Foreground="#FFD5DBE8" VerticalAlignment="Center" TextWrapping="Wrap"/></Grid>"##,
                caps(k),
                esc(what)
            );
        }
        x + "</StackPanel>"
    };
    let r = |k: &str, w: &str| (k.to_string(), w.to_string());
    let tabs = [r("Ctrl+1", "Tree View"), r("Ctrl+2", "File View"), r("Ctrl+3", "Advisor"), r("Ctrl+5", "Cleaners"), r("Ctrl+F", "Search")];
    let tables = [
        r("Up+Down", "Move through the rows"),
        r("PageUp+PageDown", "A page at a time"),
        r("Home+End", "First or last row"),
        r("Ctrl+click", "Pick several"),
        r("Shift+click", "Pick a range"),
        r("type a name", "Jump to it"),
    ];
    let map = [r("click", "Select"), r("double-click", "Go into a folder"), r("wheel", "Zoom in and out"), r("right-drag", "Pan")];
    // the menus' own, without the tabs listed above
    let menu: Vec<(String, String)> = shortcuts().into_iter().filter(|(k, _)| !tabs.iter().any(|(t, _)| t == k)).collect();
    let half = menu.len().div_ceil(2);
    let mut left = group("Tabs", &tabs);
    left += &group("Commands", &menu[..half]);
    let mut right = group("Commands, continued", &menu[half..]);
    right += &group("Lists and tables", &tables);
    right += &group("Treemap", &map);
    format!(r##"<Grid ColumnSpacing="36" Margin="0,4,8,0"><Grid.ColumnDefinitions><ColumnDefinition Width="*"/><ColumnDefinition Width="*"/></Grid.ColumnDefinitions><StackPanel>{left}</StackPanel><StackPanel Grid.Column="1">{right}</StackPanel></Grid>"##)
}

/// Every menu shortcut, as (keys, what it does), in menu order.
pub(crate) fn shortcuts() -> Vec<(String, String)> {
    SHORTCUTS.with(|s| s.borrow().clone())
}

fn item(name: &str, text: &str, key: &str, mods: &str) -> String {
    note_shortcut(text, key, mods);
    let acc = if key.is_empty() {
        String::new()
    } else if mods.is_empty() {
        format!(r##"<MenuFlyoutItem.KeyboardAccelerators><KeyboardAccelerator Key="{key}"/></MenuFlyoutItem.KeyboardAccelerators>"##)
    } else {
        format!(r##"<MenuFlyoutItem.KeyboardAccelerators><KeyboardAccelerator Key="{key}" Modifiers="{mods}"/></MenuFlyoutItem.KeyboardAccelerators>"##)
    };
    format!(r##"<MenuFlyoutItem x:Name="{name}" Text="{text}">{acc}</MenuFlyoutItem>"##)
}

fn toggle(name: &str, text: &str, checked: bool, key: &str, mods: &str) -> String {
    note_shortcut(text, key, mods);
    let acc = if key.is_empty() {
        String::new()
    } else if mods.is_empty() {
        format!(r##"<ToggleMenuFlyoutItem.KeyboardAccelerators><KeyboardAccelerator Key="{key}"/></ToggleMenuFlyoutItem.KeyboardAccelerators>"##)
    } else {
        format!(r##"<ToggleMenuFlyoutItem.KeyboardAccelerators><KeyboardAccelerator Key="{key}" Modifiers="{mods}"/></ToggleMenuFlyoutItem.KeyboardAccelerators>"##)
    };
    format!(r##"<ToggleMenuFlyoutItem x:Name="{name}" Text="{text}" IsChecked="{checked}">{acc}</ToggleMenuFlyoutItem>"##)
}

fn radio(name: &str, text: &str, checked: bool) -> String {
    radio_in("units", name, text, checked)
}

fn radio_in(group: &str, name: &str, text: &str, checked: bool) -> String {
    format!(r##"<RadioMenuFlyoutItem x:Name="{name}" Text="{text}" GroupName="{group}" IsChecked="{checked}"/>"##)
}

const SEP: &str = "<MenuFlyoutSeparator/>";

/// Glass surfaces over the acrylic window.
const CARD: &str = r##"CornerRadius="8" Background="#12FFFFFF" BorderBrush="#1CFFFFFF" BorderThickness="1""##;
const PANE: &str = r##"CornerRadius="8" Background="#0DFFFFFF" BorderBrush="#18FFFFFF" BorderThickness="1""##;

fn hud() -> String {
    let num = crate::look::NUM;
    let lanes: String = (0..crate::hud::LANES)
        .map(|k| {
            format!(
                r##"<Grid x:Name="Lane{k}" Height="20" VerticalAlignment="Top" Opacity="0" ColumnSpacing="8">
  <Grid.ColumnDefinitions><ColumnDefinition Width="56"/><ColumnDefinition Width="*"/><ColumnDefinition Width="70"/></Grid.ColumnDefinitions>
  <TextBlock x:Name="Lane{k}Name" FontSize="12" VerticalAlignment="Center"/>
  <Border x:Name="Lane{k}Bar" Grid.Column="1" Height="8" CornerRadius="4" HorizontalAlignment="Left" Width="2" Background="#FF4CC2FF"/>
  <TextBlock x:Name="Lane{k}Val" Grid.Column="2" {num} FontSize="12" Foreground="#FFA8AFC1" HorizontalAlignment="Right" VerticalAlignment="Center"/>
</Grid>"##
            )
        })
        .collect();
    let stat = |label: &str, name: &str, col: u8, row: u8| {
        format!(
            r##"<StackPanel Grid.Column="{col}" Grid.Row="{row}"><TextBlock Text="{label}" FontSize="12" Foreground="#FF7A8296"/><TextBlock x:Name="{name}" Text="0" {num} FontSize="17" FontWeight="SemiBold"/></StackPanel>"##
        )
    };
    format!(
        r##"<Border x:Name="Hud" HorizontalAlignment="Right" VerticalAlignment="Top" Margin="14" Width="336" CornerRadius="10" Padding="14,12" BorderBrush="#26FFFFFF" BorderThickness="1" Visibility="Collapsed" Opacity="0" IsHitTestVisible="False">
  <Border.Background><AcrylicBrush TintColor="#FF141620" TintOpacity="0.55" TintLuminosityOpacity="0.7" FallbackColor="#E8161822"/></Border.Background>
  <StackPanel Spacing="8">
    <Grid>
      <TextBlock x:Name="HudMethod" Text="Reading" FontSize="12" FontWeight="SemiBold" Foreground="#FF7A8296" CharacterSpacing="60"/>
      <TextBlock x:Name="HudPct" Text="0%" {num} FontSize="12" FontWeight="SemiBold" Foreground="#FF4CC2FF" HorizontalAlignment="Right"/>
    </Grid>
    <Image x:Name="HudImage" Height="76" Stretch="Fill"/>
    <Grid ColumnSpacing="14" RowSpacing="6">
      <Grid.ColumnDefinitions><ColumnDefinition/><ColumnDefinition/></Grid.ColumnDefinitions>
      <Grid.RowDefinitions><RowDefinition/><RowDefinition/></Grid.RowDefinitions>
      {s1}{s2}{s3}{s4}
    </Grid>
    <Grid Height="134">{lanes}</Grid>
  </StackPanel>
</Border>"##,
        s1 = stat("Files", "HudFiles", 0, 0),
        s2 = stat("Folders", "HudFolders", 1, 0),
        s3 = stat("Data found", "HudBytes", 0, 1),
        s4 = stat("Items / s", "HudRate", 1, 1),
    )
}

pub fn page(drives: &[String]) -> String {
    let num = crate::look::NUM;
    use crate::look::{pick, pick_drop, Pick};
    let (tab_on, tab_off) = (pick(Pick::Tab, true), pick(Pick::Tab, false));
    // the activity indicator: the selectable look on its own blue
    let busy_style = r##"Style="{StaticResource SprawlPick}" Background="#332F6BFF""##;
    let (mode_on, mode_off, drop_off) = (pick(Pick::Mode, true), pick(Pick::Mode, false), pick_drop(Pick::Mode, false));
    let drive_items: String = drives
        .iter()
        .map(|d| format!(r##"<ComboBoxItem Content="{}"/>"##, esc(d)))
        .collect();
    let file_menu = [
        item("mScan", "Scan Again", "F5", ""),
        item("mSelectFolder", "Select Folder...", "F3", ""),
        item("mSelectMultiple", "Select Multiple Folders...", "F3", "Shift"),
        item("mSelectCsv", "Open CSV File...", "O", "Control"),
        item("mStop", "Stop Scan", "Escape", ""),
        item("mNewInstance", "New Instance", "N", "Control"),
        SEP.to_string(),
        item("mReset", "Reset tree expansion", "F4", ""),
        item("mExpandAll", "Expand All", "F4", "Shift"),
        item("mParent", "Select Parent", "F6", ""),
        item("mReselectChild", "Reselect Child", "F6", "Shift"),
        item("mZoomIn", "Zoom in Treemap", "F10", ""),
        item("mZoomOut", "Zoom out Treemap", "F11", ""),
        item("mSavePng", "Save Treemap Image as .png file", "S", "Control"),
        item("mSearch", "Search Names...", "F", "Control"),
        item("mFilter", "Filter...", "F", "Control,Shift"),
        SEP.to_string(),
        item("mExplore", "Explore Folder", "E", "Control"),
        item("mCmd", "Command Prompt Here", "P", "Control"),
        item("mCopyPath", "Copy Path", "P", "Control,Shift"),
        SEP.to_string(),
        item("mDelete", "Delete (to Recycle Bin)", "Delete", ""),
        item("mDeleteForever", "Delete (No undelete possible)", "Delete", "Shift"),
        SEP.to_string(),
        item("mExportCsv", "Export to .CSV file...", "E", "Control,Shift"),
        item("mCopyInfo", "Copy file and size info to clipboard", "C", "Control,Shift"),
        item("mExportTypes", "Export File Types to CSV file...", "X", "Control,Shift"),
        SEP.to_string(),
        item("mExit", "Exit", "F4", "Menu"),
    ]
    .concat();
    let options_menu = [
        item("mSettings", "Settings...", "S", "Control,Shift"),
        item("mTweaks", "Colours and Tweaks...", "T", "Control"),
        SEP.to_string(),
        format!(
            r##"<MenuFlyoutSubItem Text="Show">{}{}{}</MenuFlyoutSubItem>"##,
            toggle("oHeader", "Header", true, "H", "Control"),
            toggle("oTypes", "File Types", true, "F8", ""),
            toggle("oTreemap", "Map", true, "F9", ""),
        ),
        format!(
            r##"<MenuFlyoutSubItem Text="Sizes in">{}{}{}{}{}{}</MenuFlyoutSubItem>"##,
            radio("uDyn", "Best fit", true),
            radio("uB", "Bytes", false),
            radio("uKB", "KB", false),
            radio("uMB", "MB", false),
            radio("uGB", "GB", false),
            radio("uTB", "TB", false),
        ),
        format!(
            r##"<MenuFlyoutSubItem Text="Map">{}{}{}{}{}{}{}{}</MenuFlyoutSubItem>"##,
            toggle("oTilt", "3D", true, "T", "Control,Shift"),
            toggle("oNames", "Folder and File Names", true, "N", "Control,Shift"),
            toggle("oAlloc", "Allocated Size", false, "", ""),
            toggle("oCushion", "Cushion Shading", true, "", ""),
            toggle("oFree", "Free Space", false, "", ""),
            toggle("oMirror", "Mirror Floor", true, "", ""),
            toggle("oPulses", "Live Change Pulses", true, "", ""),
            format!(
                r##"<MenuFlyoutSubItem Text="3D Antialiasing">{}{}{}{}</MenuFlyoutSubItem>"##,
                radio_in("aa", "aaOff", "Off", false),
                radio_in("aa", "aaLow", "Low (2x)", false),
                radio_in("aa", "aaMedium", "Medium (4x)", true),
                radio_in("aa", "aaHigh", "High (8x)", false),
            ),
        ),
        toggle("oAuto", "Scan as Soon as a Drive Is Picked", true, "", ""),
        SEP.to_string(),
        item("mAdmin", "Restart as Administrator", "", ""),
        toggle("oAlwaysAdmin", "Always Run as Administrator", false, "", ""),
        toggle("oExplorer", "Add &quot;Scan with Sprawl&quot; to Explorer", false, "", ""),
        SEP.to_string(),
        item("mWizard", "Setup Wizard...", "", ""),
    ]
    .concat();
    let help_menu = [
        item("mShortcuts", "Keyboard Shortcuts", "F1", ""),
        item("mUpdate", "Check for Updates", "", ""),
        toggle("oUpdates", "Check for Updates Automatically", true, "", ""),
        SEP.to_string(),
        item("mAbout", "About Sprawl", "", ""),
    ]
    .concat();
    let hud = hud();
    let welcome = crate::welcome::xaml();
    let item = |k: crate::views::Kind| {
        let (_, cmd, name, _) = crate::views::ALL.iter().find(|a| a.0 == k).copied().unwrap();
        format!(r##"<MenuFlyoutItem x:Name="{cmd}" Text="{name}: {}" ToolTipService.ToolTip="{}"/>"##, crate::ui::esc(k.short()), crate::ui::esc(k.hint()))
    };
    let mut view_items = String::new();
    for (g, (group, kinds)) in crate::views::GROUPS.iter().enumerate() {
        if g + 1 == crate::views::GROUPS.len() {
            view_items += &format!(r##"<MenuFlyoutSeparator/><MenuFlyoutSubItem Text="{group}">{}</MenuFlyoutSubItem>"##, kinds.iter().map(|&k| item(k)).collect::<String>());
        } else {
            if g > 0 {
                view_items += "<MenuFlyoutSeparator/>";
            }
            view_items += &format!(r##"<MenuFlyoutItem Text="{group}" IsEnabled="False" FontSize="12"/>"##);
            view_items += &kinds.iter().map(|&k| item(k)).collect::<String>();
        }
    }
    let paint_items: String = crate::views::PAINTS.iter().map(|(_, cmd, name)| format!(r##"<MenuFlyoutItem x:Name="{cmd}" Text="{name}"/>"##)).collect();
    let tree_hint = crate::welcome::tree_hint();
    // Explorer's own menu (as administrator only when allowed in Settings > Scanning: crate::admin)
    let shell = |name: &str| {
        let tip = if sprawl_core::is_elevated() { "Explorer's own right-click menu (or Shift+right-click). As administrator it is off unless you allow it in Settings > Scanning." } else { "Explorer's own right-click menu (or Shift+right-click). What it does is not checked by Sprawl." };
        format!(r##"<MenuFlyoutItem x:Name="{name}" Text="Windows menu..." ToolTipService.ToolTip="{tip}"><MenuFlyoutItem.Icon><FontIcon Glyph="&#xE7C4;"/></MenuFlyoutItem.Icon></MenuFlyoutItem>"##)
    };
    let (shell_item, shell_more) = (shell("cShell"), shell("sxShell"));
    let types_hint = crate::welcome::types_hint();
    let wizard = crate::wizard::xaml();
    let hist = crate::history::xaml();
    let version = env!("CARGO_PKG_VERSION");
    let (logo_small, logo_big) = (logo(46), logo(96));
    let palettes: String = crate::paint::PALETTES.iter().map(|p| format!(r##"<ComboBoxItem Content="{p}"/>"##)).collect();
    let tweaks = format!(r##"
    <Border x:Name="Tweaks" HorizontalAlignment="Left" VerticalAlignment="Top" Margin="12" Width="300" CornerRadius="10" Padding="16,10,4,10" BorderBrush="#26FFFFFF" BorderThickness="1" Visibility="Collapsed">
      <Border.Background><AcrylicBrush TintColor="#FF141620" TintOpacity="0.55" TintLuminosityOpacity="0.7" FallbackColor="#E8161822"/></Border.Background>
      <Grid><Grid.RowDefinitions><RowDefinition Height="Auto"/><RowDefinition Height="*"/></Grid.RowDefinitions>
        <Grid Margin="0,0,12,0">
          <TextBlock Text="Colours and Tweaks" FontSize="13" FontWeight="SemiBold" VerticalAlignment="Center"/>
          <Button x:Name="TweaksClose" HorizontalAlignment="Right" Padding="6" Background="Transparent" BorderThickness="0"><FontIcon Glyph="&#xE711;" FontSize="12"/></Button>
        </Grid>
      <!-- a short map scrolls the panel rather than cutting off its last switch -->
      <ScrollViewer Grid.Row="1" HorizontalScrollBarVisibility="Disabled" VerticalScrollBarVisibility="Auto" Padding="0,0,12,0">
      <StackPanel Spacing="4">
        <ToggleSwitch x:Name="AnimSwitch" Header="Animations" IsOn="True" FontSize="12" Margin="0,2,0,0"/>
        <TextBlock Text="Colours" FontSize="12" Foreground="#FF8E97AA" Margin="0,4,0,0"/>
        <ComboBox x:Name="PaletteBox" HorizontalAlignment="Stretch" SelectedIndex="0">{palettes}</ComboBox>
        <TextBlock Text="Tile height when hovered" FontSize="12" Foreground="#FF8E97AA" Margin="0,8,0,0"/>
        <Slider x:Name="LiftSlider" Minimum="0" Maximum="250" Value="100" StepFrequency="5"/>
        <TextBlock Text="Cushion shading" FontSize="12" Foreground="#FF8E97AA"/>
        <Slider x:Name="CushionSlider" Minimum="0" Maximum="200" Value="100" StepFrequency="5"/>
        <TextBlock Text="Tilt" FontSize="12" Foreground="#FF8E97AA"/>
        <Slider x:Name="TiltSlider" Minimum="0" Maximum="100" Value="55" StepFrequency="1"/>
      </StackPanel>
      </ScrollViewer>
      </Grid>
    </Border>"##);

    format!(
        r##"<Grid {NS} x:Name="Root" RequestedTheme="Dark" Background="Transparent">
  <Grid.RowDefinitions>
    <RowDefinition Height="40"/>
    <RowDefinition Height="Auto"/>
    <RowDefinition Height="Auto"/>
    <RowDefinition Height="Auto"/>
    <RowDefinition Height="1.5*"/>
    <RowDefinition Height="Auto"/>
    <RowDefinition x:Name="MapRow" Height="1.25*"/>
    <RowDefinition Height="28"/>
  </Grid.RowDefinitions>

  <Grid x:Name="TitleBar" Background="Transparent">
    <StackPanel Orientation="Horizontal" VerticalAlignment="Center" Margin="14,0,0,0" Spacing="10">
      <Border Width="18" Height="18" CornerRadius="5">
        <Border.Background>
          <LinearGradientBrush StartPoint="0,0" EndPoint="1,1">
            <GradientStop Color="#FFFFB347" Offset="0"/><GradientStop Color="#FFFF5E62" Offset="0.55"/><GradientStop Color="#FF8E54E9" Offset="1"/>
          </LinearGradientBrush>
        </Border.Background>
        <Grid Width="12" Height="9" Margin="0,1,0,0">
          <Rectangle Fill="White" RadiusX="0.4" RadiusY="0.4" Width="2.4" Height="5.1" HorizontalAlignment="Left" VerticalAlignment="Bottom" Margin="0,0,0,0"/>
          <Rectangle Fill="White" RadiusX="0.4" RadiusY="0.4" Width="2.4" Height="8.1" HorizontalAlignment="Left" VerticalAlignment="Bottom" Margin="2.3,0,0,0"/>
          <Rectangle Fill="White" RadiusX="0.4" RadiusY="0.4" Width="2.6" Height="6.4" HorizontalAlignment="Left" VerticalAlignment="Bottom" Margin="4.5,0,0,0"/>
          <Rectangle Fill="White" RadiusX="0.4" RadiusY="0.4" Width="2.4" Height="9" HorizontalAlignment="Left" VerticalAlignment="Bottom" Margin="7,0,0,0"/>
          <Rectangle Fill="White" RadiusX="0.4" RadiusY="0.4" Width="2.8" Height="4.3" HorizontalAlignment="Left" VerticalAlignment="Bottom" Margin="9.2,0,0,0"/>
        </Grid>
      </Border>
      <TextBlock x:Name="TitleText" Text="Sprawl" FontSize="12" VerticalAlignment="Center" Foreground="#FFC9CFDC"/>
    </StackPanel>
  </Grid>

  <MenuBar Grid.Row="1" Margin="6,0,0,0">
    <MenuBarItem Title="File">{file_menu}</MenuBarItem>
    <MenuBarItem Title="Options">{options_menu}</MenuBarItem>
    <MenuBarItem Title="Help">{help_menu}</MenuBarItem>
  </MenuBar>

  <Grid x:Name="Header" Grid.Row="2" Margin="12,4,12,8" ColumnSpacing="10">
    <Grid.ColumnDefinitions>
      <ColumnDefinition Width="380"/>
      <ColumnDefinition Width="*"/>
      <ColumnDefinition Width="Auto"/>
    </Grid.ColumnDefinitions>
    <Border {CARD} Padding="12,10">
      <StackPanel Spacing="8">
        <Grid ColumnSpacing="8">
          <Grid.ColumnDefinitions><ColumnDefinition Width="Auto"/><ColumnDefinition Width="*"/><ColumnDefinition Width="Auto"/></Grid.ColumnDefinitions>
          <TextBlock Text="Select:" VerticalAlignment="Center" Foreground="#FFB9C2D6"/>
          <ComboBox x:Name="DriveBox" Grid.Column="1" HorizontalAlignment="Stretch" SelectedIndex="0">{drive_items}</ComboBox>
          <Button x:Name="ScanButton" Grid.Column="2" Style="{{StaticResource AccentButtonStyle}}" MinWidth="78">
            <StackPanel Orientation="Horizontal" Spacing="6"><FontIcon x:Name="ScanButtonIcon" Glyph="&#xE721;" FontSize="13"/><TextBlock x:Name="ScanButtonText" Text="Scan"/></StackPanel>
          </Button>
        </Grid>
        <TextBlock x:Name="StatusText" {num} TextTrimming="CharacterEllipsis" Text="Ready. Pick a drive or folder and press Scan." FontSize="12" Foreground="#FFB9C2D6"/>
        <ProgressBar x:Name="Progress" Value="0" Maximum="100" Height="4" CornerRadius="2"/>
      </StackPanel>
    </Border>
    <Border Grid.Column="1" {CARD} Padding="14,10">
      <Grid ColumnSpacing="14" RowSpacing="7" VerticalAlignment="Center">
        <Grid.ColumnDefinitions><ColumnDefinition Width="Auto"/><ColumnDefinition Width="Auto"/><ColumnDefinition Width="*"/></Grid.ColumnDefinitions>
        <Grid.RowDefinitions><RowDefinition/><RowDefinition/><RowDefinition/></Grid.RowDefinitions>
        <TextBlock x:Name="HeadLabel" Text="Selected:" Foreground="#FF8E97AA" FontSize="12"/>
        <TextBlock x:Name="SelText" {num} Grid.Column="1" Grid.ColumnSpan="2" Text="-" FontWeight="SemiBold" FontSize="12" TextTrimming="CharacterEllipsis"/>
        <StackPanel Grid.Row="1" Grid.Column="1" Grid.ColumnSpan="2" Orientation="Horizontal" Spacing="10">
          <TextBlock x:Name="ScanInfo" {num} Text="Not scanned yet" FontSize="12" Foreground="#FFC9CFDC" TextTrimming="CharacterEllipsis" VerticalAlignment="Center"/>
          <HyperlinkButton x:Name="CoverageDetails" Content="Details" Visibility="Collapsed" Padding="4,0" FontSize="12" VerticalAlignment="Center" ToolTipService.ToolTip="What this scan covers: how it was read, what it couldn't read or skipped, and what isn't in it"/>
        </StackPanel>
        <TextBlock x:Name="DriveLabel" Text="Drive:" Grid.Row="2" Foreground="#FF8E97AA" FontSize="12"/>
        <TextBlock x:Name="DriveText" {num} Grid.Row="2" Grid.Column="1" Text="-" FontSize="12" Foreground="#FFC9CFDC"/>
        <Grid Grid.Row="2" Grid.Column="2" VerticalAlignment="Center" Margin="10,0,0,0">
          <Border x:Name="UsageTrack" Height="8" CornerRadius="4" Background="#14FFFFFF"/>
          <Border x:Name="UsageBar" Height="8" CornerRadius="4" HorizontalAlignment="Left" Width="0">
            <Border.Background>
              <LinearGradientBrush StartPoint="0,0" EndPoint="1,0">
                <GradientStop Color="#FF4CC2FF" Offset="0"/><GradientStop Color="#FF9B7BFF" Offset="0.8"/><GradientStop Color="#FFFF6B8B" Offset="1"/>
              </LinearGradientBrush>
            </Border.Background>
          </Border>
        </Grid>
      </Grid>
    </Border>
    <Border x:Name="BrandCard" Grid.Column="2" {CARD} Padding="16,10,22,10">
      <StackPanel Orientation="Horizontal" Spacing="14" VerticalAlignment="Center">
        {logo_small}
        <StackPanel VerticalAlignment="Center" Spacing="2">
          <TextBlock FontSize="18" FontWeight="SemiBold"><Run Text="Sprawl"/><Run Text=" {version}" Foreground="#FF8E97AA" FontWeight="Normal" FontSize="14"/></TextBlock>
          <TextBlock Text="Disk space analyser" FontSize="12" Foreground="#FF8E97AA"/>
        </StackPanel>
      </StackPanel>
    </Border>
  </Grid>

  <Grid Grid.Row="3" Margin="12,0,12,6">
  <Button x:Name="BusyPill" Visibility="Collapsed" {busy_style} HorizontalAlignment="Right" VerticalAlignment="Center" Margin="0,0,352,0" Width="260" Height="32" Padding="10,0" HorizontalContentAlignment="Stretch" ToolTipService.ToolTip="What is running now. Click for the list; an entry opens its tab.">
    <Grid ColumnSpacing="8"><Grid.ColumnDefinitions><ColumnDefinition Width="Auto"/><ColumnDefinition Width="*"/><ColumnDefinition Width="Auto"/></Grid.ColumnDefinitions>
      <ProgressRing IsActive="True" Width="14" Height="14" MinWidth="0" MinHeight="0" VerticalAlignment="Center"/>
      <TextBlock x:Name="BusyText" Grid.Column="1" FontSize="12" Foreground="#FFCFE0FF" VerticalAlignment="Center" TextTrimming="CharacterEllipsis"/>
      <TextBlock x:Name="BusyMore" Grid.Column="2" {num} FontSize="12" FontWeight="SemiBold" Foreground="#FFCFE0FF" VerticalAlignment="Center"/>
    </Grid>
    <Button.Flyout><Flyout x:Name="BusyFly" Placement="BottomEdgeAlignedRight"><Border x:Name="BusyList" MinWidth="320" MaxWidth="520"/></Flyout></Button.Flyout>
  </Button>
  <TextBox x:Name="FilterBox" HorizontalAlignment="Right" Width="340" Height="32" PlaceholderText="Filter: *.mp4;  node_modules;  *\cache\*" FontSize="12"/>
  <StackPanel Orientation="Horizontal" Spacing="4">
    <Button x:Name="TabTree" Content="Tree View" {tab_on} Padding="14,4" FontSize="14"><Button.KeyboardAccelerators><KeyboardAccelerator Key="Number1" Modifiers="Control"/></Button.KeyboardAccelerators></Button>
    <Button x:Name="TabFiles" Content="File View" {tab_off} Padding="14,4" FontSize="14"><Button.KeyboardAccelerators><KeyboardAccelerator Key="Number2" Modifiers="Control"/></Button.KeyboardAccelerators></Button>
    <Button x:Name="TabAdvisor" Content="Advisor" {tab_off} Padding="14,4" FontSize="14"><Button.KeyboardAccelerators><KeyboardAccelerator Key="Number3" Modifiers="Control"/></Button.KeyboardAccelerators></Button>
    <Button x:Name="TabCleaners" Content="Cleaners" {tab_off} Padding="14,4" FontSize="14"><Button.KeyboardAccelerators><KeyboardAccelerator Key="Number5" Modifiers="Control"/></Button.KeyboardAccelerators></Button>
    <Button x:Name="TabSearch" {tab_off} Padding="14,4" FontSize="14"><StackPanel Orientation="Horizontal" Spacing="6"><FontIcon Glyph="&#xE721;" FontSize="12"/><TextBlock Text="Search"/></StackPanel></Button>
    <Button x:Name="TabAbout" Visibility="Collapsed" Content="About" {tab_off} Padding="14,4" FontSize="14"><Button.KeyboardAccelerators><KeyboardAccelerator Key="Number4" Modifiers="Control"/></Button.KeyboardAccelerators></Button>
  </StackPanel>
  </Grid>

  <Grid x:Name="Panes" Grid.Row="4" Margin="12,0,12,0" ColumnSpacing="10">
    <Grid.ColumnDefinitions>
      <ColumnDefinition x:Name="TreeCol" Width="1.4*"/>
      <ColumnDefinition x:Name="TypesCol" Width="1*"/>
    </Grid.ColumnDefinitions>
    <Border {PANE}>
      <Grid>
        <Grid.RowDefinitions><RowDefinition Height="Auto"/><RowDefinition Height="Auto"/><RowDefinition Height="*"/><RowDefinition Height="Auto"/></Grid.RowDefinitions>
        <Border x:Name="SearchMore" Grid.Row="3" Visibility="Collapsed" BorderBrush="#22FFFFFF" BorderThickness="0,1,0,0">
          <FlyoutBase.AttachedFlyout>
            <MenuFlyout x:Name="MoreMenu">
              <MenuFlyoutItem x:Name="sxExplore" Text="Show in Explorer"><MenuFlyoutItem.Icon><FontIcon Glyph="&#xEC50;"/></MenuFlyoutItem.Icon></MenuFlyoutItem>
              <MenuFlyoutItem x:Name="sxCopy" Text="Copy Path"><MenuFlyoutItem.Icon><FontIcon Glyph="&#xE8C8;"/></MenuFlyoutItem.Icon></MenuFlyoutItem>
              <MenuFlyoutItem x:Name="sxScan" Text="Scan This Folder"><MenuFlyoutItem.Icon><FontIcon Glyph="&#xE721;"/></MenuFlyoutItem.Icon></MenuFlyoutItem>
              <MenuFlyoutSeparator/>
              {shell_more}
            </MenuFlyout>
          </FlyoutBase.AttachedFlyout>
        </Border>
        <Grid x:Name="SearchBar" Visibility="Collapsed" Margin="8,8,8,6" ColumnSpacing="8" RowSpacing="6">
          <Grid.ColumnDefinitions><ColumnDefinition Width="*"/><ColumnDefinition Width="Auto"/><ColumnDefinition Width="Auto"/><ColumnDefinition Width="Auto"/><ColumnDefinition Width="Auto"/><ColumnDefinition Width="Auto"/></Grid.ColumnDefinitions>
          <Grid.RowDefinitions><RowDefinition Height="Auto"/><RowDefinition Height="Auto"/><RowDefinition Height="Auto"/></Grid.RowDefinitions>
          <TextBox x:Name="SearchBox" FontSize="13" Height="32" PlaceholderText="Search names:  report 2024    *.iso    \steam\ *.vdf    &gt;1gb    folder:cache    !tmp" ToolTipService.ToolTip="Space: and.  a|b: either.  !word: not.  *, ?: wildcards.  A word with \ looks at the whole path.  &gt;1gb, &lt;10kb: size.  folder: or file: one kind.  ? lists everything."/>
          <Button x:Name="SearchHelp" Grid.Column="1" Content="?" FontSize="13" Padding="10,3" Height="32" ToolTipService.ToolTip="Everything the search box understands, with examples to try">
            <Button.Flyout><Flyout x:Name="SearchHelpFly" Placement="Bottom"><ScrollViewer MaxHeight="560" HorizontalScrollBarVisibility="Disabled"><Border x:Name="SearchHelpHost" MaxWidth="640"/></ScrollViewer></Flyout></Button.Flyout>
          </Button>
          <Border x:Name="SearchSaved" Grid.Column="2"/>
          <TextBlock x:Name="SearchInfo" Grid.Column="3" {num} VerticalAlignment="Center" FontSize="12" Foreground="#FF8E97AA" MaxWidth="420" TextTrimming="CharacterEllipsis"/>
          <Button x:Name="SearchToFilter" Grid.Column="4" Content="Use as filter" FontSize="12" Padding="10,4" ToolTipService.ToolTip="Narrow the whole analysis (tree, files, map) to this pattern"/>
          <ToggleButton x:Name="SearchMapBtn" Grid.Column="5" FontSize="12" Padding="10,4" ToolTipService.ToolTip="Show the map under the results, with the matches lit"><StackPanel Orientation="Horizontal" Spacing="6"><FontIcon Glyph="&#xE80A;" FontSize="12"/><TextBlock Text="Map"/></StackPanel></ToggleButton>
          <ScrollViewer Grid.Row="1" Grid.ColumnSpan="6" HorizontalScrollBarVisibility="Auto" HorizontalScrollMode="Enabled" VerticalScrollBarVisibility="Disabled" VerticalScrollMode="Disabled"><StackPanel Orientation="Horizontal" Spacing="18"><Border x:Name="SearchChips"/><Border x:Name="SearchDrives"/></StackPanel></ScrollViewer>
          <TextBlock x:Name="SearchHint" Grid.Row="2" Grid.ColumnSpan="6" FontSize="12" Foreground="#FF8E97AA" TextWrapping="Wrap" MaxLines="3"/>
        </Grid>
        <Border x:Name="SearchNone" Grid.Row="2" Visibility="Collapsed" Padding="16,14" VerticalAlignment="Top" IsHitTestVisible="False" Canvas.ZIndex="5"><TextBlock x:Name="SearchNoneText" TextWrapping="Wrap" FontSize="13" LineHeight="22" Foreground="#FFB9C2D6"/></Border>
        <Border x:Name="SearchEmpty" Grid.Row="2" Visibility="Collapsed" Background="Transparent"><ScrollViewer HorizontalScrollBarVisibility="Disabled" Padding="16,12"><Border x:Name="SearchEmptyHost"/></ScrollViewer></Border>
        <Border x:Name="SearchSuggest" Grid.RowSpan="3" Canvas.ZIndex="20" Visibility="Collapsed" VerticalAlignment="Top" HorizontalAlignment="Left" Margin="8,42,0,0" MinWidth="320" MaxWidth="640" Padding="4" CornerRadius="6" Background="#F21C1F2A" BorderBrush="#33FFFFFF" BorderThickness="1"/>
        <Border x:Name="TreeHeader" Grid.Row="1"/>
        <ScrollViewer x:Name="TreeScroll" Grid.Row="2" HorizontalScrollBarVisibility="Disabled" IsTabStop="True" UseSystemFocusVisuals="False">
          <FlyoutBase.AttachedFlyout>
            <MenuFlyout x:Name="RowMenu">
              <MenuFlyoutItem x:Name="cOpen" Text="Open"><MenuFlyoutItem.Icon><FontIcon Glyph="&#xE8E5;"/></MenuFlyoutItem.Icon></MenuFlyoutItem>
              <MenuFlyoutItem x:Name="cInTree" Text="Show in the Tree (Ctrl+Enter)" Visibility="Collapsed"><MenuFlyoutItem.Icon><FontIcon Glyph="&#xE8A9;"/></MenuFlyoutItem.Icon></MenuFlyoutItem>
              <MenuFlyoutItem x:Name="cExplore" Text="Show in Explorer"><MenuFlyoutItem.Icon><FontIcon Glyph="&#xEC50;"/></MenuFlyoutItem.Icon></MenuFlyoutItem>
              <MenuFlyoutItem x:Name="cCmd" Text="Command Prompt Here"><MenuFlyoutItem.Icon><FontIcon Glyph="&#xE756;"/></MenuFlyoutItem.Icon></MenuFlyoutItem>
              <MenuFlyoutItem x:Name="cCopyPath" Text="Copy Path"><MenuFlyoutItem.Icon><FontIcon Glyph="&#xE8C8;"/></MenuFlyoutItem.Icon></MenuFlyoutItem>
              <MenuFlyoutSeparator/>
              <MenuFlyoutItem x:Name="cMapHere" Text="Show on Treemap"><MenuFlyoutItem.Icon><FontIcon Glyph="&#xE80A;"/></MenuFlyoutItem.Icon></MenuFlyoutItem>
              <MenuFlyoutItem x:Name="cExpandAll" Text="Expand All"><MenuFlyoutItem.Icon><FontIcon Glyph="&#xE8A9;"/></MenuFlyoutItem.Icon></MenuFlyoutItem>
              <MenuFlyoutItem x:Name="cCopyInfo" Text="Copy Name and Size"><MenuFlyoutItem.Icon><FontIcon Glyph="&#xE16F;"/></MenuFlyoutItem.Icon></MenuFlyoutItem>
              <MenuFlyoutSeparator/>
              <MenuFlyoutItem x:Name="cDelete" Text="Delete (to Recycle Bin)"><MenuFlyoutItem.Icon><FontIcon Glyph="&#xE74D;"/></MenuFlyoutItem.Icon></MenuFlyoutItem>
              <MenuFlyoutItem x:Name="cDeleteForever" Text="Delete Permanently"><MenuFlyoutItem.Icon><FontIcon Glyph="&#xE74D;" Foreground="#FFFF6B6B"/></MenuFlyoutItem.Icon></MenuFlyoutItem>
              <MenuFlyoutSeparator/>
              {shell_item}
            </MenuFlyout>
          </FlyoutBase.AttachedFlyout>
        </ScrollViewer>
        {tree_hint}
        <Grid x:Name="AdvisorPanel" Grid.RowSpan="4" Visibility="Collapsed"><Grid.RowDefinitions><RowDefinition Height="*"/><RowDefinition Height="Auto"/></Grid.RowDefinitions><ScrollViewer x:Name="AdvScroll" HorizontalScrollBarVisibility="Disabled"><Border x:Name="AdvHost"/></ScrollViewer><Border x:Name="ClHost" Visibility="Collapsed"/><Border x:Name="AdvFoot" Grid.Row="1"/></Grid>
        <ScrollViewer x:Name="AboutPanel" Grid.RowSpan="4" Visibility="Collapsed" Padding="24">
          <StackPanel Spacing="10" MaxWidth="640" HorizontalAlignment="Left">
            <StackPanel Orientation="Horizontal" Spacing="18" Margin="0,0,0,6">
              {logo_big}
              <StackPanel VerticalAlignment="Center" Spacing="2">
                <TextBlock Text="Sprawl" FontSize="30" FontWeight="SemiBold"/>
                <TextBlock Text="Version {version} · Disk space analyser" FontSize="13" Foreground="#FF8E97AA"/>
              </StackPanel>
            </StackPanel>
            <TextBlock TextWrapping="Wrap" Foreground="#FFB9C2D6" Text="Scans a drive or folder and shows which folders, files and file types use the most space, and helps win it back."/>
            <TextBlock TextWrapping="Wrap" Foreground="#FF8E97AA" Text="Tree View and File View: the scan as a sortable tree and a list of the biggest files, beside a table of file types and a lit 3D treemap. Advisor: what in the scan can go, how safe that is, and what has grown. Cleaners: the caches and leftovers of the applications on this PC, ticked by you and cleaned by Sprawl. Search: names in this scan, on other drives and in saved scans."/>
            <TextBlock TextWrapping="Wrap" Foreground="#FF8E97AA" Text="Treemap: hover to lift tiles, click to find a file in the tree, double-click or scroll up to zoom into a folder, scroll down to zoom out, right-drag to pan. Click a file type to light up its files."/>
            <TextBlock TextWrapping="Wrap" Foreground="#FF8E97AA" Text="Shortcuts: Ctrl+1 Tree View, Ctrl+2 File View, Ctrl+3 Advisor, Ctrl+5 Cleaners, Ctrl+F Search. F5 scan, F3 pick a folder, F4 collapse, Shift+F4 expand all, F6 parent, F10/F11 zoom the treemap, F8/F9 hide panels, Ctrl+S save the treemap picture, Ctrl+Shift+T flat or 3D. F1 lists them all."/>
          </StackPanel>
        </ScrollViewer>
      </Grid>
    </Border>
    <Border x:Name="TypesPanel" Grid.Column="1" {PANE}>
      <Grid>
        <Grid.RowDefinitions><RowDefinition Height="Auto"/><RowDefinition Height="*"/></Grid.RowDefinitions>
        <Border x:Name="ExtHeader"/>
        <ScrollViewer x:Name="ExtScroll" Grid.Row="1" HorizontalScrollBarVisibility="Disabled"/>
        {types_hint}
      </Grid>
    </Border>
  </Grid>

  <Grid x:Name="MapBar" Grid.Row="5" Margin="12,6,12,6" ColumnSpacing="12">
    <Grid.ColumnDefinitions><ColumnDefinition Width="Auto"/><ColumnDefinition Width="*"/><ColumnDefinition Width="Auto"/></Grid.ColumnDefinitions>
    <StackPanel Orientation="Horizontal" Spacing="2" HorizontalAlignment="Left" Padding="2" CornerRadius="7" Background="#14FFFFFF">
      <Button x:Name="ModeMap" {mode_on} CornerRadius="5" ToolTipService.ToolTip="Treemap: wheel zooms, right-drag pans"><StackPanel Orientation="Horizontal" Spacing="6"><FontIcon Glyph="&#xF0E2;" FontSize="12"/><TextBlock Text="Treemap" FontSize="12"/></StackPanel></Button>
      <Button x:Name="ModeCity" {mode_off} CornerRadius="5" ToolTipService.ToolTip="City skyline: footprint is size, height is item count, windows show how recently it changed. Drag to turn."><StackPanel Orientation="Horizontal" Spacing="6"><FontIcon Glyph="&#xE80F;" FontSize="12"/><TextBlock Text="City" FontSize="12"/></StackPanel></Button>
      <Button x:Name="ModeGalaxy" {mode_off} CornerRadius="5" ToolTipService.ToolTip="Galaxy: folders are star systems, planets are their biggest files"><StackPanel Orientation="Horizontal" Spacing="6"><FontIcon Glyph="&#xE7FC;" FontSize="12"/><TextBlock Text="Galaxy" FontSize="12"/></StackPanel></Button>
      <DropDownButton x:Name="ViewMore" {drop_off} Padding="10,3,4,3" CornerRadius="5" ToolTipService.ToolTip="More ways to see the scan"><StackPanel Orientation="Horizontal" Spacing="6"><FontIcon Glyph="&#xE8A9;" FontSize="12"/><TextBlock x:Name="ViewMoreText" Text="More views" FontSize="12"/></StackPanel>
        <DropDownButton.Flyout><MenuFlyout Placement="Bottom">{view_items}</MenuFlyout></DropDownButton.Flyout></DropDownButton>
      <Button x:Name="ViewOnly" Visibility="Collapsed" {mode_off} CornerRadius="5" ToolTipService.ToolTip="Waffle: show every type, or only what can be won back"><TextBlock x:Name="ViewOnlyText" Text="Showing: everything" FontSize="12"/></Button>
      <DropDownButton x:Name="ViewPaint" {drop_off} Padding="10,3,4,3" CornerRadius="5" ToolTipService.ToolTip="What the colours show"><TextBlock x:Name="ViewPaintText" Text="Colour: Types" FontSize="12"/>
        <DropDownButton.Flyout><MenuFlyout Placement="Bottom">{paint_items}</MenuFlyout></DropDownButton.Flyout></DropDownButton>
    </StackPanel>
    <Button x:Name="ModeHist" Grid.Column="2" HorizontalAlignment="Right" {mode_off} CornerRadius="5" ToolTipService.ToolTip="History: compare with earlier scans, or play them as a time-lapse"><StackPanel Orientation="Horizontal" Spacing="6"><FontIcon Glyph="&#xE81C;" FontSize="12"/><TextBlock Text="History" FontSize="12"/></StackPanel></Button>
    <TextBlock x:Name="MapPath" Grid.Column="1" Text=" " HorizontalAlignment="Center" VerticalAlignment="Center" FontSize="12" Foreground="#FFB9C2D6" TextTrimming="CharacterEllipsis"/>
  </Grid>
  {welcome}
  <Grid x:Name="MapBorder" Grid.Row="6" Margin="12,0,12,0" CornerRadius="8" BorderBrush="#18FFFFFF" BorderThickness="1" Background="#38000000">
    <SwapChainPanel x:Name="MapPanel"/>
    <TextBlock x:Name="MapCaption" Visibility="Collapsed" IsHitTestVisible="False" HorizontalAlignment="Left" VerticalAlignment="Top" Margin="14,10,14,0" MaxWidth="720" TextWrapping="Wrap" FontSize="12" Foreground="#FFD5DBE8"/>
    <Border x:Name="MapKey" IsHitTestVisible="False" HorizontalAlignment="Left" VerticalAlignment="Bottom" Margin="12" Padding="10,6" CornerRadius="6" Background="#B314161E"/>
    {hud}
    {tweaks}
    {hist}
  </Grid>

  <Grid Grid.Row="7" Margin="16,0,16,0">
    <TextBlock x:Name="FootLeft" {num} Text="" FontSize="12" Foreground="#FF8E97AA" VerticalAlignment="Center" TextTrimming="CharacterEllipsis"/>
    <TextBlock x:Name="FootRight" {num} Text="" FontSize="12" Foreground="#FF8E97AA" VerticalAlignment="Center" HorizontalAlignment="Right"/>
  </Grid>
  <Border x:Name="UpdateCard" Grid.RowSpan="8" Visibility="Collapsed" HorizontalAlignment="Right" VerticalAlignment="Bottom" Margin="0,0,20,36" Padding="14,10" CornerRadius="8" Background="#F01A1D27" BorderBrush="#552F7BF6" BorderThickness="1" Width="400">
    <StackPanel Spacing="6"><TextBlock x:Name="UpdateTitle" FontSize="14" FontWeight="SemiBold"/><TextBlock x:Name="UpdateSub" FontSize="12" Foreground="#FF8E97AA"/>
      <ScrollViewer MaxHeight="300" VerticalScrollBarVisibility="Auto" VerticalScrollMode="Enabled"><TextBlock x:Name="UpdateNotes" FontSize="12" Foreground="#FFD5DBE8" TextWrapping="Wrap" IsTextSelectionEnabled="True"/></ScrollViewer>
      <TextBlock x:Name="UpdateStatus" FontSize="12" Foreground="#FFF5B342" TextWrapping="Wrap"/>
      <StackPanel Orientation="Horizontal" Spacing="8"><Button x:Name="UpdateNow" Content="Update and restart" Padding="12,3" FontSize="12" CornerRadius="5" Background="#FF2F7BF6" Foreground="White"/><Button x:Name="UpdateLater" Content="Later" Padding="12,3" FontSize="12" CornerRadius="5"/><Button x:Name="UpdateSkip" Content="Skip this version" Padding="12,3" FontSize="12" CornerRadius="5"/></StackPanel></StackPanel>
  </Border>
  <Border x:Name="SetupOffer" Grid.RowSpan="8" Visibility="Collapsed" HorizontalAlignment="Right" VerticalAlignment="Bottom" Margin="0,0,20,36" Padding="14,10" CornerRadius="8" Background="#E61A1D27" BorderBrush="#552F7BF6" BorderThickness="1" MaxWidth="360">
    <StackPanel Spacing="8"><TextBlock Text="Set up Sprawl?" FontSize="14" FontWeight="SemiBold"/><TextBlock Text="An AI to explain unknown folders, faster scans as administrator, and history to see what grows. All optional." FontSize="12" Foreground="#FFB9C2D6" TextWrapping="Wrap"/>
      <StackPanel Orientation="Horizontal" Spacing="8"><Button x:Name="SetupYes" Content="Set up" Padding="12,3" FontSize="12" CornerRadius="5" Background="#FF2F7BF6" Foreground="White"/><Button x:Name="SetupNo" Content="Not now" Padding="12,3" FontSize="12" CornerRadius="5"/></StackPanel></StackPanel>
  </Border>
  {wizard}
  <Border x:Name="DlgHost" Grid.RowSpan="8" Canvas.ZIndex="100" Visibility="Collapsed"/>
</Grid>"##
    )
}
