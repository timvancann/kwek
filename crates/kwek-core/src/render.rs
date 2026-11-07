//! Message text to render-ready blocks. Svelte renders blocks and never parses markup, so
//! this is the only place that understands message formatting, and it is table-tested.
//!
//! Source is `formattedText`, which Chat delivers as Markdown (`markupSyntax:
//! MARKUP_SYNTAX_MARKDOWN`): `**bold**`, `*italic*`, `~~strike~~`, lists, backslash escapes,
//! and lines hard-wrapped at ~80 columns (soft breaks). A line break the sender typed arrives
//! as a paragraph break. On top of Markdown: `<users/ID>` mentions and bare-URL autolinks.

use pulldown_cmark::{CodeBlockKind, Event, Options, Parser, Tag, TagEnd};
use serde::Serialize;
use ts_rs::TS;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(tag = "kind", rename_all = "camelCase")]
#[ts(export)]
pub enum Block {
    Paragraph { inlines: Vec<Inline> },
    Heading { level: u8, inlines: Vec<Inline> },
    List { ordered: bool, start: u32, items: Vec<Vec<Block>> },
    Quote { blocks: Vec<Block> },
    Code { text: String },
    Rule,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(tag = "kind", rename_all = "camelCase")]
#[ts(export)]
pub enum Inline {
    Text { text: String, bold: bool, italic: bool, strike: bool },
    Code { text: String },
    Mention { user: String, label: String, me: bool },
    Link {
        url: String,
        label: String,
        /// `drive`, `meet` or `chat` for Google links that get a chip.
        rich: Option<String>,
    },
    Break,
}

pub struct Ctx<'a> {
    /// `users/{id}` of the signed-in user.
    pub me: &'a str,
    /// Display name for a `users/{id}`, if known.
    pub name_of: &'a dyn Fn(&str) -> Option<String>,
}

enum Frame {
    Blocks(Vec<Block>),
    Paragraph(Vec<Inline>),
    Heading(u8, Vec<Inline>),
    List { ordered: bool, start: u32, items: Vec<Vec<Block>> },
    Item(Vec<Block>),
    Quote(Vec<Block>),
    Code(String),
    Link { url: String, label: String },
}

#[derive(Default, Clone, Copy)]
struct Style {
    bold: u32,
    italic: u32,
    strike: u32,
}

pub fn parse(markdown: &str, ctx: &Ctx) -> Vec<Block> {
    let mut stack: Vec<Frame> = vec![Frame::Blocks(Vec::new())];
    let mut style = Style::default();
    let parser = Parser::new_ext(markdown, Options::ENABLE_STRIKETHROUGH);
    // The parser splits text at `_`, `<` and friends; mentions and URLs can span several
    // text events, so consecutive inline text is coalesced before it is scanned.
    let mut pending = String::new();

    for ev in parser {
        let is_plain_text = matches!(&ev, Event::Text(_) | Event::Html(_) | Event::InlineHtml(_))
            && !matches!(stack.last(), Some(Frame::Code(_) | Frame::Link { .. }));
        if is_plain_text {
            if let Event::Text(t) | Event::Html(t) | Event::InlineHtml(t) = &ev {
                pending.push_str(t);
            }
            continue;
        }
        if !pending.is_empty() {
            for i in split_text(&std::mem::take(&mut pending), style, ctx) {
                push_inline(&mut stack, i);
            }
        }
        match ev {
            Event::Start(tag) => match tag {
                Tag::Paragraph => {
                    close_implicit(&mut stack);
                    stack.push(Frame::Paragraph(Vec::new()));
                }
                Tag::Heading { level, .. } => {
                    close_implicit(&mut stack);
                    stack.push(Frame::Heading(level as u8, Vec::new()));
                }
                Tag::BlockQuote(_) => {
                    close_implicit(&mut stack);
                    stack.push(Frame::Quote(Vec::new()));
                }
                Tag::CodeBlock(_kind @ (CodeBlockKind::Fenced(_) | CodeBlockKind::Indented)) => {
                    close_implicit(&mut stack);
                    stack.push(Frame::Code(String::new()));
                }
                Tag::List(start) => {
                    close_implicit(&mut stack);
                    stack.push(Frame::List {
                        ordered: start.is_some(),
                        start: start.unwrap_or(1) as u32,
                        items: Vec::new(),
                    });
                }
                Tag::Item => stack.push(Frame::Item(Vec::new())),
                Tag::Emphasis => style.italic += 1,
                Tag::Strong => style.bold += 1,
                Tag::Strikethrough => style.strike += 1,
                Tag::Link { dest_url, .. } | Tag::Image { dest_url, .. } => {
                    ensure_inline(&mut stack);
                    stack.push(Frame::Link {
                        url: dest_url.to_string(),
                        label: String::new(),
                    });
                }
                _ => {}
            },
            Event::End(tag) => match tag {
                TagEnd::Paragraph => pop_block(&mut stack, |f| match f {
                    Frame::Paragraph(i) => Some(Block::Paragraph { inlines: merge(i) }),
                    _ => None,
                }),
                TagEnd::Heading(_) => pop_block(&mut stack, |f| match f {
                    Frame::Heading(l, i) => Some(Block::Heading { level: l, inlines: merge(i) }),
                    _ => None,
                }),
                TagEnd::BlockQuote(_) => {
                    close_implicit(&mut stack);
                    pop_block(&mut stack, |f| match f {
                        Frame::Quote(b) => Some(Block::Quote { blocks: b }),
                        _ => None,
                    })
                }
                TagEnd::CodeBlock => pop_block(&mut stack, |f| match f {
                    Frame::Code(mut t) => {
                        if t.ends_with('\n') {
                            t.pop();
                        }
                        Some(Block::Code { text: t })
                    }
                    _ => None,
                }),
                TagEnd::List(_) => pop_block(&mut stack, |f| match f {
                    Frame::List { ordered, start, items } => Some(Block::List { ordered, start, items }),
                    _ => None,
                }),
                TagEnd::Item => {
                    close_implicit(&mut stack);
                    if let Some(Frame::Item(blocks)) = stack.pop() {
                        if let Some(Frame::List { items, .. }) = stack.last_mut() {
                            items.push(blocks);
                        }
                    }
                }
                TagEnd::Emphasis => style.italic = style.italic.saturating_sub(1),
                TagEnd::Strong => style.bold = style.bold.saturating_sub(1),
                TagEnd::Strikethrough => style.strike = style.strike.saturating_sub(1),
                TagEnd::Link | TagEnd::Image => {
                    if let Some(Frame::Link { url, label }) = stack.pop() {
                        let label = if label.is_empty() { url.clone() } else { label };
                        push_inline(
                            &mut stack,
                            Inline::Link {
                                rich: rich_kind(&url),
                                url,
                                label,
                            },
                        );
                    }
                }
                _ => {}
            },
            Event::Text(t) | Event::Html(t) | Event::InlineHtml(t) => {
                if let Some(Frame::Code(buf)) = stack.last_mut() {
                    buf.push_str(&t);
                } else if let Some(Frame::Link { label, .. }) = stack.last_mut() {
                    label.push_str(&t);
                } else {
                    for i in split_text(&t, style, ctx) {
                        push_inline(&mut stack, i);
                    }
                }
            }
            Event::Code(t) => {
                if let Some(Frame::Link { label, .. }) = stack.last_mut() {
                    label.push_str(&t);
                } else {
                    push_inline(&mut stack, Inline::Code { text: t.to_string() });
                }
            }
            Event::SoftBreak => {
                if let Some(Frame::Link { label, .. }) = stack.last_mut() {
                    label.push(' ');
                } else {
                    push_inline(&mut stack, text(" ", style));
                }
            }
            Event::HardBreak => push_inline(&mut stack, Inline::Break),
            Event::Rule => {
                close_implicit(&mut stack);
                push_block(&mut stack, Block::Rule);
            }
            _ => {}
        }
    }
    if !pending.is_empty() {
        for i in split_text(&pending, style, ctx) {
            push_inline(&mut stack, i);
        }
    }
    close_implicit(&mut stack);
    match stack.into_iter().next() {
        Some(Frame::Blocks(b)) => b,
        _ => Vec::new(),
    }
}

fn text(t: &str, s: Style) -> Inline {
    Inline::Text {
        text: t.to_owned(),
        bold: s.bold > 0,
        italic: s.italic > 0,
        strike: s.strike > 0,
    }
}

/// Inline content directly inside a container (tight list items) gets an implicit paragraph.
fn ensure_inline(stack: &mut Vec<Frame>) {
    if !matches!(
        stack.last(),
        Some(Frame::Paragraph(_) | Frame::Heading(..) | Frame::Link { .. })
    ) {
        stack.push(Frame::Paragraph(Vec::new()));
    }
}

fn push_inline(stack: &mut Vec<Frame>, i: Inline) {
    ensure_inline(stack);
    match stack.last_mut() {
        Some(Frame::Paragraph(v) | Frame::Heading(_, v)) => v.push(i),
        _ => {}
    }
}

/// Closes an implicit paragraph opened by [`ensure_inline`] before a block starts or ends.
fn close_implicit(stack: &mut Vec<Frame>) {
    if stack.len() >= 2
        && matches!(stack.last(), Some(Frame::Paragraph(_)))
        && matches!(stack[stack.len() - 2], Frame::Item(_) | Frame::Quote(_) | Frame::Blocks(_))
    {
        if let Some(Frame::Paragraph(i)) = stack.pop() {
            let i = merge(i);
            if !i.is_empty() {
                push_block(stack, Block::Paragraph { inlines: i });
            }
        }
    }
}

fn push_block(stack: &mut [Frame], b: Block) {
    match stack.last_mut() {
        Some(Frame::Blocks(v) | Frame::Item(v) | Frame::Quote(v)) => v.push(b),
        _ => {}
    }
}

fn pop_block(stack: &mut Vec<Frame>, f: impl FnOnce(Frame) -> Option<Block>) {
    if stack.len() < 2 {
        return;
    }
    if let Some(b) = stack.pop().and_then(f) {
        push_block(stack, b);
    }
}

/// Splits Markdown text into plain runs, `<users/ID>` mentions and bare-URL links.
fn split_text(t: &str, style: Style, ctx: &Ctx) -> Vec<Inline> {
    let s: Vec<char> = t.chars().collect();
    let mut out = Vec::new();
    let mut buf = String::new();
    let mut i = 0;
    while i < s.len() {
        if s[i] == '<' && starts_with(&s, i + 1, "users/") {
            if let Some(end) = (i + 7..s.len()).find(|&j| s[j] == '>') {
                let id: String = s[i + 7..end].iter().collect();
                if !id.is_empty() && id.chars().all(|c| c.is_ascii_alphanumeric()) {
                    flush(&mut out, &mut buf, style);
                    let user = format!("users/{id}");
                    let label = if id == "all" {
                        "all".to_owned()
                    } else {
                        (ctx.name_of)(&user).unwrap_or_else(|| "Unknown user".to_owned())
                    };
                    out.push(Inline::Mention {
                        me: user == ctx.me || id == "all",
                        user,
                        label,
                    });
                    i = end + 1;
                    continue;
                }
            }
        }
        if (starts_with(&s, i, "https://") || starts_with(&s, i, "http://"))
            && (i == 0 || !s[i - 1].is_alphanumeric())
        {
            let mut end = i;
            while end < s.len() && !s[end].is_whitespace() && s[end] != '<' && s[end] != '>' {
                end += 1;
            }
            while end > i && ".,;:!?)'\"]".contains(s[end - 1]) {
                // Keep a closing paren that balances one inside the URL (Wikipedia links).
                let open = s[i..end].iter().filter(|&&c| c == '(').count();
                let close = s[i..end].iter().filter(|&&c| c == ')').count();
                if s[end - 1] == ')' && open >= close {
                    break;
                }
                end -= 1;
            }
            if end > i + 8 {
                flush(&mut out, &mut buf, style);
                let url: String = s[i..end].iter().collect();
                out.push(Inline::Link {
                    rich: rich_kind(&url),
                    label: url.clone(),
                    url,
                });
                i = end;
                continue;
            }
        }
        buf.push(s[i]);
        i += 1;
    }
    flush(&mut out, &mut buf, style);
    out
}

fn flush(out: &mut Vec<Inline>, buf: &mut String, style: Style) {
    if !buf.is_empty() {
        out.push(text(&std::mem::take(buf), style));
    }
}

fn starts_with(s: &[char], i: usize, pat: &str) -> bool {
    let mut j = i;
    for c in pat.chars() {
        if s.get(j) != Some(&c) {
            return false;
        }
        j += 1;
    }
    true
}

pub fn rich_kind(url: &str) -> Option<String> {
    let host = url::Url::parse(url).ok()?.host_str()?.to_owned();
    match host.as_str() {
        "drive.google.com" | "docs.google.com" => Some("drive".into()),
        "meet.google.com" => Some("meet".into()),
        "chat.google.com" => Some("chat".into()),
        _ => None,
    }
}

fn merge(v: Vec<Inline>) -> Vec<Inline> {
    let mut out: Vec<Inline> = Vec::with_capacity(v.len());
    for i in v {
        if let (
            Some(Inline::Text { text: a, bold: b1, italic: i1, strike: s1 }),
            Inline::Text { text: b, bold: b2, italic: i2, strike: s2 },
        ) = (out.last_mut(), &i)
        {
            if (*b1, *i1, *s1) == (*b2, *i2, *s2) {
                a.push_str(b);
                continue;
            }
        }
        out.push(i);
    }
    // Trim the outer whitespace a paragraph can pick up from soft breaks.
    if let Some(Inline::Text { text, .. }) = out.first_mut() {
        *text = text.trim_start().to_owned();
    }
    if let Some(Inline::Text { text, .. }) = out.last_mut() {
        *text = text.trim_end().to_owned();
    }
    out.retain(|i| !matches!(i, Inline::Text { text, .. } if text.is_empty()));
    out
}

/// Plain one-line text for previews and notifications.
pub fn plain(blocks: &[Block]) -> String {
    fn inl(out: &mut String, v: &[Inline]) {
        for i in v {
            match i {
                Inline::Text { text, .. } | Inline::Code { text } => out.push_str(text),
                Inline::Mention { label, .. } => {
                    out.push('@');
                    out.push_str(label);
                }
                Inline::Link { label, .. } => out.push_str(label),
                Inline::Break => out.push(' '),
            }
        }
    }
    fn blk(out: &mut String, v: &[Block]) {
        for b in v {
            if !out.is_empty() && !out.ends_with(' ') {
                out.push(' ');
            }
            match b {
                Block::Paragraph { inlines } | Block::Heading { inlines, .. } => inl(out, inlines),
                Block::List { items, .. } => {
                    for it in items {
                        blk(out, it);
                    }
                }
                Block::Quote { blocks } => blk(out, blocks),
                Block::Code { text } => out.push_str(text),
                Block::Rule => {}
            }
        }
    }
    let mut out = String::new();
    blk(&mut out, blocks);
    out.trim().to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(u: &str) -> Option<String> {
        match u {
            "users/1" => Some("Ada".into()),
            "users/2" => Some("Tim".into()),
            _ => None,
        }
    }

    fn p(s: &str) -> Vec<Block> {
        parse(s, &Ctx { me: "users/2", name_of: &names })
    }

    fn t(s: &str) -> Inline {
        Inline::Text { text: s.into(), bold: false, italic: false, strike: false }
    }
    fn b(s: &str) -> Inline {
        Inline::Text { text: s.into(), bold: true, italic: false, strike: false }
    }
    fn i(s: &str) -> Inline {
        Inline::Text { text: s.into(), bold: false, italic: true, strike: false }
    }
    fn para(v: Vec<Inline>) -> Block {
        Block::Paragraph { inlines: v }
    }
    fn link(url: &str, label: &str) -> Inline {
        Inline::Link { url: url.into(), label: label.into(), rich: rich_kind(url) }
    }

    #[test]
    fn table() {
        let cases: Vec<(&str, Vec<Block>)> = vec![
            ("hello\n", vec![para(vec![t("hello")])]),
            ("", vec![]),
            // Escapes from Chat's Markdown export.
            ("staat live\\!\n", vec![para(vec![t("staat live!")])]),
            ("**bold** and *it* and ~~gone~~", vec![para(vec![
                b("bold"),
                t(" and "),
                i("it"),
                t(" and "),
                Inline::Text { text: "gone".into(), bold: false, italic: false, strike: true },
            ])]),
            // Hard-wrapped lines are one paragraph; a blank line splits.
            ("one line that\nwraps\n\nsecond", vec![para(vec![t("one line that wraps")]), para(vec![t("second")])]),
            // Chat's loose, indented lists.
            ("**Title**\n\n  - first item\n    wrapped\n\n  - second\n", vec![
                para(vec![b("Title")]),
                Block::List {
                    ordered: false,
                    start: 1,
                    items: vec![vec![para(vec![t("first item wrapped")])], vec![para(vec![t("second")])]],
                },
            ]),
            // Tight list: implicit paragraphs.
            ("- a\n- b", vec![Block::List {
                ordered: false,
                start: 1,
                items: vec![vec![para(vec![t("a")])], vec![para(vec![t("b")])]],
            }]),
            ("3. x\n4. y", vec![Block::List {
                ordered: true,
                start: 3,
                items: vec![vec![para(vec![t("x")])], vec![para(vec![t("y")])]],
            }]),
            ("use `cargo *test*` now", vec![para(vec![t("use "), Inline::Code { text: "cargo *test*".into() }, t(" now")])]),
            ("```\nfn main() {}\n```", vec![Block::Code { text: "fn main() {}".into() }]),
            ("> quoted\n\nafter", vec![Block::Quote { blocks: vec![para(vec![t("quoted")])] }, para(vec![t("after")])]),
            ("# Head", vec![Block::Heading { level: 1, inlines: vec![t("Head")] }]),
            // Links.
            ("see [the site](https://x.nl)", vec![para(vec![t("see "), link("https://x.nl", "the site")])]),
            ("go https://example.com/a, now", vec![para(vec![t("go "), link("https://example.com/a", "https://example.com/a"), t(", now")])]),
            ("<https://meet.google.com/abc>", vec![para(vec![link("https://meet.google.com/abc", "https://meet.google.com/abc")])]),
            ("(https://en.wikipedia.org/wiki/Rust_(language))", vec![para(vec![
                t("("),
                link("https://en.wikipedia.org/wiki/Rust_(language)", "https://en.wikipedia.org/wiki/Rust_(language)"),
                t(")"),
            ])]),
            // Mentions.
            ("hi <users/1>!", vec![para(vec![
                t("hi "),
                Inline::Mention { user: "users/1".into(), label: "Ada".into(), me: false },
                t("!"),
            ])]),
            ("<users/2> and <users/all>", vec![para(vec![
                Inline::Mention { user: "users/2".into(), label: "Tim".into(), me: true },
                t(" and "),
                Inline::Mention { user: "users/all".into(), label: "all".into(), me: true },
            ])]),
            // HTML is never interpreted: it stays text.
            ("<b>x</b>", vec![para(vec![t("<b>x</b>")])]),
            ("2 * 3 * 4", vec![para(vec![t("2 * 3 * 4")])]),
            ("snake_case_name", vec![para(vec![t("snake_case_name")])]),
        ];
        for (input, want) in cases {
            assert_eq!(p(input), want, "input: {input:?}");
        }
    }

    #[test]
    fn real_chat_export_shape() {
        let src = "Nieuwe changes op dev (niet in prod)\n\n**Governance**\n\n  - Een browser met Domains.\n\n  - Slepen om iemand een positie te geven of een positie owner of steward te\n    maken, met Undo.\n";
        let blocks = p(src);
        assert_eq!(blocks.len(), 3);
        assert_eq!(
            plain(&blocks),
            "Nieuwe changes op dev (niet in prod) Governance Een browser met Domains. Slepen om iemand een positie te geven of een positie owner of steward te maken, met Undo."
        );
    }

    #[test]
    fn plain_preview() {
        assert_eq!(plain(&p("hi <users/1>, **see** [this](https://x.nl)")), "hi @Ada, see this");
    }

    #[test]
    fn never_panics_on_odd_input() {
        for s in ["`", "```", "<", "<users/>", "**_~", "- ", "> ", "[](", "\u{FFFC}", "1.", "* * *", "[a](b", "<users/1"] {
            let _ = p(s);
        }
    }
}
