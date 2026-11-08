//! Splits a space's messages into conversation stretches: runs of messages on one line (the
//! main flow, or one thread's replies) without a long pause, capped in size so a stretch
//! stays one topic and fits the embedding model's window.

use std::collections::BTreeMap;

use sha2::{Digest, Sha256};

use crate::ts::{MINUTE, Micros};

/// A pause longer than this starts a new stretch.
pub const GAP: Micros = 30 * MINUTE;
pub const MAX_MESSAGES: usize = 12;
/// About 400 tokens for e5; the model reads 512.
pub const MAX_CHARS: usize = 1200;
/// How much of a thread's root message heads each stretch of its replies, as context.
const ROOT_CONTEXT: usize = 200;

#[derive(Debug, Clone)]
pub struct Msg {
    pub id: i64,
    pub thread: Option<String>,
    pub is_reply: bool,
    pub sender: String,
    pub create_us: Micros,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Chunk {
    /// The thread for a stretch of replies; None for the main flow.
    pub thread: Option<String>,
    pub first_us: Micros,
    pub last_us: Micros,
    pub msg_ids: Vec<i64>,
    /// "Name: text" lines, for keyword search: names and links are things people type.
    pub text: String,
    /// What the embedding model reads: the words only. Sender names make one person's
    /// stretches all look alike, and bare links look like everything.
    pub embed: String,
    /// Of `embed`; vectors are keyed by it.
    pub hash: String,
}

/// `msgs` are one space's live messages; any order. Messages without text are skipped.
pub fn build(msgs: &[Msg]) -> Vec<Chunk> {
    // Thread roots give their replies context; roots themselves sit in the main flow.
    let roots: BTreeMap<&str, &str> = msgs
        .iter()
        .filter(|m| !m.is_reply)
        .filter_map(|m| Some((m.thread.as_deref()?, m.text.as_str())))
        .collect();
    let mut lines: BTreeMap<Option<&str>, Vec<&Msg>> = BTreeMap::new();
    for m in msgs.iter().filter(|m| !m.text.trim().is_empty()) {
        let line = if m.is_reply { m.thread.as_deref() } else { None };
        lines.entry(line).or_default().push(m);
    }
    let mut out = Vec::new();
    for (line, mut items) in lines {
        items.sort_by_key(|m| (m.create_us, m.id));
        let context = line
            .and_then(|t| roots.get(t))
            .map(|r| format!("Re: {}\n", clip(r, ROOT_CONTEXT)))
            .unwrap_or_default();
        let mut cur: Vec<&Msg> = Vec::new();
        let mut chars = 0;
        for m in items {
            let line_text = render_line(m);
            let starts_new = cur.last().is_some_and(|prev| {
                m.create_us - prev.create_us > GAP
                    || cur.len() >= MAX_MESSAGES
                    || chars + line_text.chars().count() > MAX_CHARS
            });
            if starts_new {
                out.push(finish(line, &context, &cur));
                cur.clear();
                chars = 0;
            }
            chars += line_text.chars().count();
            cur.push(m);
        }
        if !cur.is_empty() {
            out.push(finish(line, &context, &cur));
        }
    }
    out
}

fn render_line(m: &Msg) -> String {
    // One long message still has to fit the window on its own.
    format!("{}: {}\n", m.sender, clip(m.text.trim(), MAX_CHARS))
}

fn finish(line: Option<&str>, context: &str, msgs: &[&Msg]) -> Chunk {
    // The thread's root is context for meaning only: as keywords it would match every reply
    // stretch of the thread (the root is already in the main flow), with nothing to highlight.
    let mut text = String::new();
    let mut embed = without_links(context);
    for m in msgs {
        text.push_str(&render_line(m));
        embed.push_str(&without_links(&clip(m.text.trim(), MAX_CHARS)));
        embed.push('\n');
    }
    let text = text.trim_end().to_owned();
    let embed = embed.trim_end().to_owned();
    Chunk {
        thread: line.map(str::to_owned),
        first_us: msgs.first().map(|m| m.create_us).unwrap_or_default(),
        last_us: msgs.last().map(|m| m.create_us).unwrap_or_default(),
        msg_ids: msgs.iter().map(|m| m.id).collect(),
        hash: hash(&embed),
        text,
        embed,
    }
}

/// Links become their host ("github.com"): the path is noise to a language model.
fn without_links(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for (i, line) in s.split('\n').enumerate() {
        if i > 0 {
            out.push('\n');
        }
        let words: Vec<String> = line
            .split(' ')
            .map(
                |w| match w.strip_prefix("https://").or_else(|| w.strip_prefix("http://")) {
                    Some(rest) => rest
                        .split(['/', '?', '#'])
                        .next()
                        .unwrap_or("")
                        .trim_start_matches("www.")
                        .to_owned(),
                    None => w.to_owned(),
                },
            )
            .collect();
        out.push_str(&words.join(" "));
    }
    out
}

/// Too little language to place in meaning (a lone link, "ok", an emoji).
pub fn too_short_for_meaning(embed: &str) -> bool {
    embed
        .split_whitespace()
        .filter(|w| w.chars().any(char::is_alphabetic))
        .count()
        < 3
}

pub fn hash(text: &str) -> String {
    let d = Sha256::digest(text.as_bytes());
    d[..16].iter().map(|b| format!("{b:02x}")).collect()
}

pub fn clip(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_owned();
    }
    let mut out: String = s.chars().take(max.saturating_sub(1)).collect();
    out.push('…');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn m(id: i64, at_min: i64, text: &str) -> Msg {
        Msg {
            id,
            thread: Some(format!("t{id}")),
            is_reply: false,
            sender: "Noor".into(),
            create_us: at_min * MINUTE,
            text: text.into(),
        }
    }

    #[test]
    fn splits_on_pauses_and_size() {
        let msgs = vec![
            m(1, 0, "can you make a demo"),
            m(2, 5, "with a dataset"),
            m(3, 120, "lunch?"),
            m(4, 121, ""),
        ];
        let c = build(&msgs);
        assert_eq!(c.len(), 2);
        assert_eq!(c[0].msg_ids, vec![1, 2]);
        assert_eq!(c[0].text, "Noor: can you make a demo\nNoor: with a dataset");
        assert_eq!(c[1].msg_ids, vec![3], "empty messages are skipped");

        let many: Vec<Msg> = (0..30).map(|i| m(i, i, "hello")).collect();
        let c = build(&many);
        assert!(c.iter().all(|c| c.msg_ids.len() <= MAX_MESSAGES));
        assert_eq!(c.iter().map(|c| c.msg_ids.len()).sum::<usize>(), 30);
    }

    #[test]
    fn replies_form_their_own_line_with_root_context() {
        let mut root = m(1, 0, "Plan for the client pitch");
        root.thread = Some("tA".into());
        let mut reply = m(2, 1, "I'll prepare the demo data");
        reply.thread = Some("tA".into());
        reply.is_reply = true;
        let other = m(3, 2, "unrelated");
        let c = build(&[root, reply, other]);
        let main = c.iter().find(|c| c.thread.is_none()).unwrap();
        let thread = c.iter().find(|c| c.thread.as_deref() == Some("tA")).unwrap();
        assert_eq!(main.msg_ids, vec![1, 3]);
        assert_eq!(thread.msg_ids, vec![2]);
        assert!(thread.embed.starts_with("Re: Plan for the client pitch\n"));
        assert_eq!(
            thread.text, "Noor: I'll prepare the demo data",
            "root is not a keyword match"
        );
    }

    #[test]
    fn model_text_drops_names_and_link_paths() {
        let c = build(&[m(
            1,
            0,
            "see https://www.github.com/acme/studio/pull/1?x=2 for the demo",
        )]);
        assert_eq!(
            c[0].text,
            "Noor: see https://www.github.com/acme/studio/pull/1?x=2 for the demo"
        );
        assert_eq!(c[0].embed, "see github.com for the demo");
        assert!(too_short_for_meaning(&build(&[m(1, 0, "https://x.nl/a/b")])[0].embed));
        assert!(too_short_for_meaning("ok 👍"));
        assert!(!too_short_for_meaning(c[0].embed.as_str()));
    }

    #[test]
    fn hash_follows_text() {
        let a = build(&[m(1, 0, "x")]);
        let b = build(&[m(1, 0, "x")]);
        let c = build(&[m(1, 0, "y")]);
        assert_eq!(a[0].hash, b[0].hash);
        assert_ne!(a[0].hash, c[0].hash);
    }
}
