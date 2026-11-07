//! Google Chat API resources, only the fields kwek reads. Unknown fields are ignored here but
//! kept verbatim in the `raw` column, so new fields can be backfilled without a resync.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct Space {
    pub name: String,
    pub space_type: Option<String>,
    /// Deprecated `type` field (ROOM / DM); fallback when `spaceType` is missing.
    #[serde(rename = "type")]
    pub legacy_type: Option<String>,
    pub display_name: Option<String>,
    pub space_threading_state: Option<String>,
    pub last_active_time: Option<String>,
    pub single_user_bot_dm: Option<bool>,
    pub space_uri: Option<String>,
    pub membership_count: Option<MembershipCount>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct MembershipCount {
    pub joined_direct_human_user_count: Option<u32>,
    pub joined_group_count: Option<u32>,
}

impl Space {
    pub fn kind(&self) -> &str {
        match (self.space_type.as_deref(), self.legacy_type.as_deref()) {
            (Some(t), _) => t,
            (None, Some("DM")) => "DIRECT_MESSAGE",
            _ => "SPACE",
        }
    }

    /// Inline-threaded and threaded spaces both have replies that belong under a root.
    pub fn threaded(&self) -> bool {
        matches!(
            self.space_threading_state.as_deref(),
            Some("THREADED_MESSAGES") | Some("GROUPED_MESSAGES")
        )
    }
}

#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct User {
    pub name: String,
    pub display_name: Option<String>,
    #[serde(rename = "type")]
    pub kind: Option<String>,
    /// Present on message senders with user auth (see docs/api-findings.md).
    pub email: Option<String>,
    pub avatar_url: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct Thread {
    pub name: String,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct Message {
    pub name: String,
    pub sender: Option<User>,
    pub create_time: Option<String>,
    pub last_update_time: Option<String>,
    pub delete_time: Option<String>,
    pub text: Option<String>,
    pub formatted_text: Option<String>,
    pub annotations: Option<Vec<serde_json::Value>>,
    pub thread: Option<Thread>,
    pub thread_reply: Option<bool>,
    pub space: Option<SpaceRef>,
    pub attachment: Option<Vec<Attachment>>,
    pub cards_v2: Option<Vec<serde_json::Value>>,
    pub emoji_reaction_summaries: Option<Vec<EmojiReactionSummary>>,
    pub client_assigned_message_id: Option<String>,
    pub deletion_metadata: Option<serde_json::Value>,
}

impl Message {
    /// `spaces/S` from `spaces/S/messages/M`.
    pub fn space_name(&self) -> String {
        space_of(&self.name)
    }
}

pub fn space_of(resource: &str) -> String {
    let mut parts = resource.splitn(3, '/');
    match (parts.next(), parts.next()) {
        (Some(a), Some(b)) => format!("{a}/{b}"),
        _ => resource.to_owned(),
    }
}

#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct SpaceRef {
    pub name: String,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct Attachment {
    pub name: Option<String>,
    pub content_name: Option<String>,
    pub content_type: Option<String>,
    pub attachment_data_ref: Option<AttachmentDataRef>,
    pub drive_data_ref: Option<DriveDataRef>,
    pub thumbnail_uri: Option<String>,
    pub download_uri: Option<String>,
    pub source: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct AttachmentDataRef {
    pub resource_name: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct DriveDataRef {
    pub drive_file_id: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "camelCase", default)]
pub struct Emoji {
    // Sent in reactions.create: absent fields must be omitted, not null.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unicode: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub custom_emoji: Option<CustomEmoji>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "camelCase", default)]
pub struct CustomEmoji {
    pub uid: Option<String>,
    pub name: Option<String>,
    pub emoji_name: Option<String>,
}

impl Emoji {
    pub fn unicode(s: &str) -> Self {
        Emoji {
            unicode: Some(s.to_owned()),
            custom_emoji: None,
        }
    }

    /// Stable key: the unicode string, or `custom:{uid}`.
    pub fn key(&self) -> String {
        match (&self.unicode, &self.custom_emoji) {
            (Some(u), _) => u.clone(),
            (None, Some(c)) => format!("custom:{}", c.uid.clone().unwrap_or_default()),
            _ => String::new(),
        }
    }

    /// What to show: the unicode, or `:name:` for a custom emoji.
    pub fn label(&self) -> String {
        match (&self.unicode, &self.custom_emoji) {
            (Some(u), _) => u.clone(),
            (None, Some(c)) => c
                .emoji_name
                .clone()
                .or_else(|| c.name.clone())
                .unwrap_or_else(|| ":custom:".into()),
            _ => String::new(),
        }
    }
}

#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct EmojiReactionSummary {
    pub emoji: Emoji,
    pub reaction_count: Option<u32>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct Reaction {
    pub name: String,
    pub user: Option<User>,
    pub emoji: Emoji,
}

impl Reaction {
    /// `spaces/S/messages/M` from `spaces/S/messages/M/reactions/R`.
    pub fn message_name(&self) -> String {
        match self.name.find("/reactions/") {
            Some(i) => self.name[..i].to_owned(),
            None => self.name.clone(),
        }
    }
}

#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct Membership {
    pub name: String,
    pub state: Option<String>,
    pub role: Option<String>,
    pub member: Option<User>,
}

impl Membership {
    pub fn space_name(&self) -> String {
        space_of(&self.name)
    }
}

#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct SpaceReadState {
    pub name: String,
    pub last_read_time: Option<String>,
}

impl SpaceReadState {
    /// `spaces/S` from `users/U/spaces/S/spaceReadState`.
    pub fn space_name(&self) -> Option<String> {
        let mut it = self.name.split('/');
        while let Some(p) = it.next() {
            if p == "spaces" {
                return it.next().map(|s| format!("spaces/{s}"));
            }
        }
        None
    }
}

#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct ThreadReadState {
    pub name: String,
    pub last_read_time: Option<String>,
}

impl ThreadReadState {
    /// `spaces/S/threads/T` from `users/U/spaces/S/threads/T/threadReadState`.
    pub fn thread_name(&self) -> Option<String> {
        let i = self.name.find("spaces/")?;
        let rest = &self.name[i..];
        let end = rest.find("/threadReadState").unwrap_or(rest.len());
        Some(rest[..end].to_owned())
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Page<T> {
    #[serde(
        default = "Vec::new",
        alias = "spaces",
        alias = "messages",
        alias = "memberships",
        alias = "reactions",
        alias = "spaceEvents"
    )]
    pub items: Vec<T>,
    #[serde(default)]
    pub next_page_token: Option<String>,
}

impl<T> Page<T> {
    pub fn next(&self) -> Option<&str> {
        self.next_page_token.as_deref().filter(|t| !t.is_empty())
    }
}

/// A `spaces.spaceEvents` entry. The payload is kept as JSON and decoded by
/// [`crate::events::decode_event_data`], the same decoder the live loop uses.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct SpaceEvent {
    pub name: String,
    pub event_time: Option<String>,
    pub event_type: String,
    #[serde(flatten)]
    pub data: serde_json::Map<String, serde_json::Value>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resource_name_helpers() {
        assert_eq!(space_of("spaces/AAA/messages/BBB.CCC"), "spaces/AAA");
        let r = Reaction {
            name: "spaces/A/messages/M/reactions/R".into(),
            ..Default::default()
        };
        assert_eq!(r.message_name(), "spaces/A/messages/M");
        let s = SpaceReadState {
            name: "users/123/spaces/AAA/spaceReadState".into(),
            last_read_time: None,
        };
        assert_eq!(s.space_name().as_deref(), Some("spaces/AAA"));
        let t = ThreadReadState {
            name: "users/123/spaces/AAA/threads/TTT/threadReadState".into(),
            last_read_time: None,
        };
        assert_eq!(t.thread_name().as_deref(), Some("spaces/AAA/threads/TTT"));
    }

    #[test]
    fn page_accepts_any_list_key() {
        let p: Page<Space> = serde_json::from_str(r#"{"spaces":[{"name":"spaces/A"}],"nextPageToken":""}"#).unwrap();
        assert_eq!(p.items.len(), 1);
        assert_eq!(p.next(), None);
    }

    #[test]
    fn emoji_keys() {
        assert_eq!(Emoji::unicode("👍").key(), "👍");
        let c = Emoji {
            unicode: None,
            custom_emoji: Some(CustomEmoji {
                uid: Some("u1".into()),
                name: None,
                emoji_name: Some(":party:".into()),
            }),
        };
        assert_eq!(c.key(), "custom:u1");
        assert_eq!(c.label(), ":party:");
    }
}
