//! Render-ready views sent to the UI. Names are resolved and text is parsed here, so the
//! webview only renders. TypeScript types are generated with ts-rs (`cargo test` exports
//! them to `src/lib/bindings/`).

use std::collections::HashMap;

use serde::Serialize;
use ts_rs::TS;

use crate::chat::{Attachment, EmojiReactionSummary};
use crate::db::read::{MessageRow, SpaceRow, ThreadStats, UserRow};
use crate::render::{self, Block, Ctx, Inline};
use crate::ts::Micros;

#[derive(Debug, Clone, Serialize, TS, PartialEq)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct UserRef {
    pub name: String,
    pub display_name: String,
    pub email: Option<String>,
    pub avatar_url: Option<String>,
}

#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct SpaceSummary {
    pub name: String,
    /// `space`, `dm` or `group`.
    pub kind: String,
    pub display_name: String,
    pub avatar_url: Option<String>,
    pub unread: u32,
    pub mentions: u32,
    pub muted: bool,
    pub starred: bool,
    /// Kept out of the sidebar's main sections; shown under Hidden.
    pub hidden: bool,
    /// Place in the user's own order for the Spaces section; None when never placed.
    pub position: Option<i32>,
    #[ts(type = "number")]
    pub last_active_us: Micros,
    #[ts(type = "number | null")]
    pub last_read_us: Option<Micros>,
    pub threaded: bool,
    pub member_count: Option<u32>,
    pub web_url: Option<String>,
    pub history_complete: bool,
    /// Cached history is contiguous from here to now; older cached rows may have gaps.
    #[ts(type = "number | null")]
    pub low_water_us: Option<Micros>,
}

#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct AttachmentView {
    pub name: String,
    pub content_type: Option<String>,
    /// `image` renders inline via `media_url`; `file` downloads; `drive` opens in the browser.
    pub kind: String,
    pub media_url: Option<String>,
    pub resource: Option<String>,
    pub open_url: Option<String>,
}

#[derive(Debug, Clone, Serialize, TS, PartialEq)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ReactionView {
    /// Stable key to toggle with (`Emoji::key`).
    pub key: String,
    pub emoji: String,
    pub count: u32,
    pub mine: bool,
}

#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ThreadSummary {
    pub reply_count: u32,
    #[ts(type = "number")]
    pub last_reply_us: Micros,
    pub participants: Vec<UserRef>,
    pub unread: bool,
}

#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct MessageView {
    pub name: String,
    pub space: String,
    pub thread: Option<String>,
    pub is_reply: bool,
    pub sender: UserRef,
    pub is_own: bool,
    #[ts(type = "number")]
    pub create_us: Micros,
    pub edited: bool,
    pub deleted: bool,
    pub mentions_me: bool,
    pub blocks: Vec<Block>,
    pub attachments: Vec<AttachmentView>,
    pub reactions: Vec<ReactionView>,
    pub thread_summary: Option<ThreadSummary>,
    /// `synced`, `pending` or `failed`.
    pub local_state: String,
    /// The message carries app cards that kwek does not render.
    pub has_cards: bool,
    pub web_url: Option<String>,
    /// Set when the message is a call (Chat posts a bare Meet link when one starts).
    pub call: Option<CallView>,
}

/// A meeting from the user's calendar, for the sidebar and reminders.
#[derive(Debug, Clone, Serialize, TS, PartialEq)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct MeetingView {
    pub id: String,
    pub title: String,
    #[ts(type = "number")]
    pub start_us: Micros,
    #[ts(type = "number")]
    pub end_us: Micros,
    /// The Meet link, when the meeting has one.
    pub join_url: Option<String>,
    /// The event in Google Calendar.
    pub open_url: Option<String>,
}

#[derive(Debug, Clone, Serialize, TS, PartialEq)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct CalendarView {
    /// `off` (signed out, or not loaded yet), `ok`, `needsPermission` (the sign-in lacks
    /// calendar access), `unavailable` (see detail).
    pub state: String,
    pub detail: Option<String>,
    /// Running and upcoming meetings for the next day, soonest first.
    pub meetings: Vec<MeetingView>,
}

#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct CallView {
    pub url: String,
    /// How long it lasted: Chat updates the message when the call ends. None while it runs,
    /// or for a call nobody picked up.
    pub duration_secs: Option<u32>,
}

/// The Meet link of a call message: no text, only a Meet rich link. Chat posts one when
/// someone starts a call in a conversation, and updates it when the call ends.
pub fn call_link(m: &MessageRow) -> Option<String> {
    if m.text.as_deref().is_some_and(|t| !t.trim().is_empty()) {
        return None;
    }
    let annotations: Vec<serde_json::Value> = serde_json::from_str(m.annotations.as_deref()?).ok()?;
    annotations
        .iter()
        .find(|a| a.pointer("/richLinkMetadata/richLinkType").and_then(|t| t.as_str()) == Some("MEET_SPACE"))
        .and_then(|a| a.pointer("/richLinkMetadata/uri")?.as_str())
        // Only Meet itself: a call is offered as something to join.
        .filter(|u| u.starts_with("https://meet.google.com/"))
        .map(str::to_owned)
}

/// The Google Cloud setup kwek signs in with, for the sign-in screen.
#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct GoogleSetupView {
    pub project: String,
    /// Baked into this build rather than given by the user.
    pub built_in: bool,
}

#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase", tag = "state")]
#[ts(export)]
pub enum AuthState {
    SignedOut,
    SigningIn,
    SignedIn {
        email: Option<String>,
        name: Option<String>,
        user: String,
    },
}

#[derive(Debug, Clone, Serialize, TS, PartialEq)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct SyncStatus {
    /// `starting`, `bootstrapping`, `live`, `polling`, `reconciling`, `offline`,
    /// `authRequired`, `needsSetup` (signed in, but no Google client to use), `error`.
    pub state: String,
    pub detail: Option<String>,
}

#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct SnippetPart {
    pub text: String,
    pub hit: bool,
}

#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct SearchLine {
    /// Message name, to open it.
    pub message: String,
    pub sender: UserRef,
    #[ts(type = "number")]
    pub create_us: Micros,
    pub parts: Vec<SnippetPart>,
}

/// One conversation stretch that matched.
#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct SearchHit {
    pub space: String,
    pub space_name: String,
    /// Set when the stretch is a thread's replies: open the thread.
    pub thread: Option<String>,
    /// The message to jump to: the first with a word match, else the first.
    pub anchor: String,
    #[ts(type = "number")]
    pub first_us: Micros,
    #[ts(type = "number")]
    pub last_us: Micros,
    pub lines: Vec<SearchLine>,
    /// Messages in the stretch not shown as lines.
    pub more: u32,
    /// Matched on the query's words.
    pub words: bool,
    /// Close in meaning to the query.
    pub meaning: bool,
}

#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct SearchResult {
    pub hits: Vec<SearchHit>,
    /// Oldest synced message time: search covers this point onwards.
    #[ts(type = "number | null")]
    pub covered_from_us: Option<Micros>,
    /// Conversations whose older history is still downloading.
    pub history_left: u32,
    /// Search by meaning: off (no model), loading, or how far indexing is.
    pub meaning: MeaningStatus,
}

#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct MeaningStatus {
    pub available: bool,
    pub ready: bool,
    pub indexed: u32,
    pub total: u32,
}

/// Builds views with a fixed user lookup and signed-in identity.
pub struct Builder<'a> {
    pub me: &'a str,
    pub users: &'a HashMap<String, UserRow>,
}

impl Builder<'_> {
    pub fn user(&self, name: &str) -> UserRef {
        let u = self.users.get(name);
        let display_name = u
            .and_then(|u| u.display_name.clone().or_else(|| u.email.clone()))
            .unwrap_or_else(|| {
                if name.is_empty() {
                    "Unknown".into()
                } else {
                    "Unknown user".into()
                }
            });
        UserRef {
            name: name.to_owned(),
            display_name,
            email: u.and_then(|u| u.email.clone()),
            avatar_url: u.and_then(|u| u.avatar_url.as_deref()).and_then(avatar_media_url),
        }
    }

    fn name_of(&self, user: &str) -> Option<String> {
        self.users.get(user).and_then(|u| u.display_name.clone())
    }

    pub fn space(&self, s: &SpaceRow, members: &[String], space_web: Option<String>) -> SpaceSummary {
        let kind = match s.space_type.as_str() {
            "DIRECT_MESSAGE" => "dm",
            "GROUP_CHAT" => "group",
            _ => "space",
        };
        let (display_name, avatar_url) = match (kind, &s.display_name) {
            (_, Some(n)) if !n.is_empty() => (n.clone(), None),
            ("dm", _) => match &s.dm_peer {
                Some(p) => {
                    let u = self.user(p);
                    (u.display_name, u.avatar_url)
                }
                None => ("Direct message".into(), None),
            },
            _ => {
                let mut names: Vec<String> = members
                    .iter()
                    .filter(|m| m.as_str() != self.me)
                    .map(|m| self.user(m).display_name)
                    .filter(|n| n != "Unknown user")
                    .collect();
                names.sort();
                if names.is_empty() {
                    ("Group chat".into(), None)
                } else {
                    let first = first_names(&names);
                    (first, None)
                }
            }
        };
        SpaceSummary {
            name: s.name.clone(),
            kind: kind.into(),
            display_name,
            avatar_url,
            unread: if s.muted { 0 } else { s.unread.max(0) as u32 },
            mentions: s.mentions.max(0) as u32,
            muted: s.muted,
            starred: s.starred,
            hidden: s.hidden,
            position: s.position.map(|p| p as i32),
            last_active_us: s.last_active_us.unwrap_or(0).max(s.newest_us.unwrap_or(0)),
            last_read_us: s.last_read_us,
            threaded: s.threaded,
            member_count: s.member_count.map(|n| n as u32),
            web_url: space_web,
            history_complete: s.history_complete,
            low_water_us: s.low_water_us,
        }
    }

    pub fn message(
        &self,
        m: &MessageRow,
        stats: Option<&ThreadStats>,
        mine: &[(String, String)],
        space_uri: Option<&str>,
    ) -> MessageView {
        let deleted = m.delete_us.is_some();
        let ctx = Ctx {
            me: self.me,
            name_of: &|u| self.name_of(u),
        };
        let call = (!deleted).then(|| call_link(m)).flatten().map(|url| CallView {
            url,
            duration_secs: (m.update_us > m.create_us + crate::ts::SECOND)
                .then(|| ((m.update_us - m.create_us) / crate::ts::SECOND) as u32),
        });
        let mut blocks = if deleted {
            vec![]
        } else {
            render::parse(m.formatted_text.as_deref().or(m.text.as_deref()).unwrap_or(""), &ctx)
        };
        // A call shows as a call, not as its bare link.
        if blocks.is_empty() && !deleted && call.is_none() {
            blocks = rich_link_blocks(m.annotations.as_deref());
        }

        let attachments: Vec<Attachment> = m
            .attachments
            .as_deref()
            .and_then(|a| serde_json::from_str(a).ok())
            .unwrap_or_default();
        let summaries: Vec<EmojiReactionSummary> = m
            .reactions
            .as_deref()
            .and_then(|a| serde_json::from_str(a).ok())
            .unwrap_or_default();
        let reactions = summaries
            .into_iter()
            .filter(|s| s.reaction_count.unwrap_or(0) > 0)
            .map(|s| {
                let key = s.emoji.key();
                ReactionView {
                    mine: mine.iter().any(|(k, _)| k == &key),
                    emoji: s.emoji.label(),
                    key,
                    count: s.reaction_count.unwrap_or(0),
                }
            })
            .collect();

        MessageView {
            name: m.name.clone(),
            space: m.space.clone(),
            thread: m.thread.clone(),
            is_reply: m.is_reply,
            sender: self.user(&m.sender),
            is_own: m.sender == self.me,
            create_us: m.create_us,
            // A call's update is its end, not an edit.
            edited: !deleted && m.update_us > m.create_us + 1_000_000 && call.is_none(),
            deleted,
            mentions_me: m.mentions_me,
            blocks,
            attachments: attachments.iter().map(attachment_view).collect(),
            reactions,
            thread_summary: stats
                .filter(|s| s.reply_count > 0 && !m.is_reply)
                .map(|s| ThreadSummary {
                    reply_count: s.reply_count as u32,
                    last_reply_us: s.last_reply_us,
                    participants: s.participants.iter().map(|p| self.user(p)).collect(),
                    unread: s.unread,
                }),
            local_state: m.local_state.clone(),
            has_cards: m.cards.as_deref().is_some_and(|c| c != "[]"),
            web_url: message_web_url(space_uri, m),
            call,
        }
    }
}

fn first_names(names: &[String]) -> String {
    let short: Vec<&str> = names.iter().map(|n| n.split_whitespace().next().unwrap_or(n)).collect();
    match short.len() {
        0 => String::new(),
        1..=3 => short.join(", "),
        n => format!("{}, +{}", short[..3].join(", "), n - 3),
    }
}

/// Meet and Drive chips arrive as RICH_LINK annotations with empty text.
fn rich_link_blocks(annotations: Option<&str>) -> Vec<Block> {
    let Some(a) = annotations.and_then(|a| serde_json::from_str::<Vec<serde_json::Value>>(a).ok()) else {
        return vec![];
    };
    let links: Vec<Inline> = a
        .iter()
        .filter(|x| x.get("type").and_then(|t| t.as_str()) == Some("RICH_LINK"))
        .filter_map(|x| x.pointer("/richLinkMetadata/uri").and_then(|u| u.as_str()))
        .map(|u| Inline::Link {
            url: u.to_owned(),
            label: u.trim_start_matches("https://").to_owned(),
            rich: render::rich_kind(u),
        })
        .collect();
    if links.is_empty() {
        vec![]
    } else {
        vec![Block::Paragraph { inlines: links }]
    }
}

/// Avatars are proxied through Rust (`media://localhost/avatar/...`) so the webview never
/// talks to the network. Only Google's avatar host is allowed.
pub fn avatar_media_url(url: &str) -> Option<String> {
    is_avatar_host(url).then(|| {
        format!(
            "media://localhost/avatar/{}",
            percent_encoding::utf8_percent_encode(url, percent_encoding::NON_ALPHANUMERIC)
        )
    })
}

pub fn is_avatar_host(url: &str) -> bool {
    url::Url::parse(url).ok().is_some_and(|u| {
        u.scheme() == "https"
            && u.host_str()
                .is_some_and(|h| h == "googleusercontent.com" || h.ends_with(".googleusercontent.com"))
    })
}

pub fn media_url(resource: &str) -> String {
    format!(
        "media://localhost/{}",
        percent_encoding::utf8_percent_encode(resource, percent_encoding::NON_ALPHANUMERIC)
    )
}

fn attachment_view(a: &Attachment) -> AttachmentView {
    let resource = a.attachment_data_ref.as_ref().and_then(|r| r.resource_name.clone());
    let drive = a.drive_data_ref.as_ref().and_then(|d| d.drive_file_id.clone());
    let is_image = a.content_type.as_deref().is_some_and(|c| c.starts_with("image/"));
    let (kind, media_url, open_url) = match (&resource, &drive) {
        (_, Some(id)) => ("drive", None, Some(format!("https://drive.google.com/open?id={id}"))),
        (Some(r), None) if is_image => ("image", Some(media_url(r)), None),
        (Some(_), None) => ("file", None, None),
        (None, None) => ("file", None, a.download_uri.clone()),
    };
    AttachmentView {
        name: a.content_name.clone().unwrap_or_else(|| "attachment".into()),
        content_type: a.content_type.clone(),
        kind: kind.into(),
        media_url,
        resource,
        open_url,
    }
}

/// `https://chat.google.com/room/{space}/{thread}/{message}` (or `/dm/...`), built from the
/// space's `spaceUri`.
fn message_web_url(space_uri: Option<&str>, m: &MessageRow) -> Option<String> {
    let base = space_uri?.split('?').next()?.trim_end_matches('/');
    if m.name.starts_with("pending:") {
        return None;
    }
    let msg = m.name.rsplit('/').next()?;
    let thread = m.thread.as_deref().and_then(|t| t.rsplit('/').next()).unwrap_or(msg);
    Some(format!("{base}/{thread}/{msg}"))
}

/// Splits an FTS snippet with `\u{1}`/`\u{2}` hit markers into parts.
pub fn snippet_parts(s: &str) -> Vec<SnippetPart> {
    let mut out = Vec::new();
    let mut buf = String::new();
    let mut hit = false;
    for c in s.chars() {
        match c {
            '\u{1}' | '\u{2}' => {
                if !buf.is_empty() {
                    out.push(SnippetPart {
                        text: std::mem::take(&mut buf),
                        hit,
                    });
                }
                hit = c == '\u{1}';
            }
            _ => buf.push(c),
        }
    }
    if !buf.is_empty() {
        out.push(SnippetPart { text: buf, hit });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snippet_markers() {
        let p = snippet_parts("a \u{1}hit\u{2} b");
        assert_eq!(p.len(), 3);
        assert!(p[1].hit && p[1].text == "hit");
        assert!(!p[2].hit);
    }

    #[test]
    fn group_names() {
        assert_eq!(first_names(&["Ada Lovelace".into(), "Ron B".into()]), "Ada, Ron");
        assert_eq!(
            first_names(&["A".into(), "B".into(), "C".into(), "D".into(), "E".into()]),
            "A, B, C, +2"
        );
    }
}
