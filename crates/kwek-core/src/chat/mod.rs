//! Hand-rolled client for the ~15 Chat endpoints kwek uses, behind a trait so the sync
//! engine runs against a fake in tests.

pub mod types;

use async_trait::async_trait;
use serde_json::json;

pub use types::*;

use crate::error::Result;
use crate::google::{Google, q};

const BASE: &str = "https://chat.googleapis.com/v1";

/// Event types `spaces.spaceEvents.list` is asked for during reconcile.
pub const SPACE_EVENT_TYPES: &[&str] = &[
    "google.workspace.chat.message.v1.created",
    "google.workspace.chat.message.v1.updated",
    "google.workspace.chat.message.v1.deleted",
    "google.workspace.chat.reaction.v1.created",
    "google.workspace.chat.reaction.v1.deleted",
    "google.workspace.chat.membership.v1.created",
    "google.workspace.chat.membership.v1.updated",
    "google.workspace.chat.membership.v1.deleted",
    "google.workspace.chat.space.v1.updated",
];

#[derive(Debug, Clone, Default)]
pub struct ListMessages {
    pub page_size: u32,
    pub page_token: Option<String>,
    pub filter: Option<String>,
    pub newest_first: bool,
}

#[async_trait]
pub trait ChatApi: Send + Sync {
    async fn list_spaces(&self, page_token: Option<&str>) -> Result<Page<Space>>;
    async fn get_space(&self, name: &str) -> Result<Space>;
    async fn list_messages(&self, space: &str, opts: &ListMessages) -> Result<Page<Message>>;
    async fn get_message(&self, name: &str) -> Result<Message>;
    async fn create_message(&self, space: &str, text: &str, thread: Option<&str>, client_id: &str) -> Result<Message>;
    async fn update_message_text(&self, name: &str, text: &str) -> Result<Message>;
    async fn delete_message(&self, name: &str) -> Result<()>;
    async fn create_reaction(&self, message: &str, emoji: &Emoji) -> Result<Reaction>;
    async fn delete_reaction(&self, name: &str) -> Result<()>;
    /// Reactions on `message` by `user` (`users/{id}`).
    async fn list_reactions_by(&self, message: &str, user: &str) -> Result<Vec<Reaction>>;
    async fn list_members(&self, space: &str, page_token: Option<&str>) -> Result<Page<Membership>>;
    async fn get_space_read_state(&self, space: &str) -> Result<SpaceReadState>;
    async fn update_space_read_state(&self, space: &str, last_read_time: &str) -> Result<SpaceReadState>;
    async fn get_thread_read_state(&self, thread: &str) -> Result<ThreadReadState>;
    async fn list_space_events(
        &self,
        space: &str,
        start_time: &str,
        page_token: Option<&str>,
    ) -> Result<Page<SpaceEvent>>;
    async fn download_media(&self, resource_name: &str) -> Result<(Vec<u8>, Option<String>)>;
}

pub struct HttpChat {
    g: Google,
}

impl HttpChat {
    pub fn new(g: Google) -> Self {
        Self { g }
    }
}

/// `spaces/S/threads/T` -> path under `users/me` for read state.
fn me_path(resource: &str) -> String {
    format!("{BASE}/users/me/{resource}")
}

#[async_trait]
impl ChatApi for HttpChat {
    async fn list_spaces(&self, page_token: Option<&str>) -> Result<Page<Space>> {
        let mut url = format!("{BASE}/spaces?pageSize=1000");
        if let Some(t) = page_token {
            url += &format!("&pageToken={}", q(t));
        }
        self.g.get(&url).await
    }

    async fn get_space(&self, name: &str) -> Result<Space> {
        self.g.get(&format!("{BASE}/{name}")).await
    }

    async fn list_messages(&self, space: &str, o: &ListMessages) -> Result<Page<Message>> {
        let mut url = format!(
            "{BASE}/{space}/messages?pageSize={}&showDeleted=true&orderBy={}",
            o.page_size.clamp(1, 1000),
            q(if o.newest_first {
                "createTime desc"
            } else {
                "createTime asc"
            })
        );
        if let Some(f) = &o.filter {
            url += &format!("&filter={}", q(f));
        }
        if let Some(t) = &o.page_token {
            url += &format!("&pageToken={}", q(t));
        }
        self.g.get(&url).await
    }

    async fn get_message(&self, name: &str) -> Result<Message> {
        self.g.get(&format!("{BASE}/{name}")).await
    }

    async fn create_message(&self, space: &str, text: &str, thread: Option<&str>, client_id: &str) -> Result<Message> {
        let mut url = format!("{BASE}/{space}/messages?messageId={}", q(client_id));
        let mut body = json!({ "text": text });
        if let Some(t) = thread {
            url += "&messageReplyOption=REPLY_MESSAGE_FALLBACK_TO_NEW_THREAD";
            body["thread"] = json!({ "name": t });
        }
        self.g.post(&url, &body).await
    }

    async fn update_message_text(&self, name: &str, text: &str) -> Result<Message> {
        self.g
            .patch(&format!("{BASE}/{name}?updateMask=text"), &json!({ "text": text }))
            .await
    }

    async fn delete_message(&self, name: &str) -> Result<()> {
        self.g.delete(&format!("{BASE}/{name}")).await
    }

    async fn create_reaction(&self, message: &str, emoji: &Emoji) -> Result<Reaction> {
        self.g
            .post(&format!("{BASE}/{message}/reactions"), &json!({ "emoji": emoji }))
            .await
    }

    async fn delete_reaction(&self, name: &str) -> Result<()> {
        self.g.delete(&format!("{BASE}/{name}")).await
    }

    async fn list_reactions_by(&self, message: &str, user: &str) -> Result<Vec<Reaction>> {
        let filter = format!("user.name = \"{user}\"");
        let mut out = Vec::new();
        let mut token: Option<String> = None;
        loop {
            let mut url = format!("{BASE}/{message}/reactions?pageSize=200&filter={}", q(&filter));
            if let Some(t) = &token {
                url += &format!("&pageToken={}", q(t));
            }
            let page: Page<Reaction> = self.g.get(&url).await?;
            token = page.next().map(str::to_owned);
            out.extend(page.items);
            if token.is_none() {
                return Ok(out);
            }
        }
    }

    async fn list_members(&self, space: &str, page_token: Option<&str>) -> Result<Page<Membership>> {
        let mut url = format!("{BASE}/{space}/members?pageSize=1000");
        if let Some(t) = page_token {
            url += &format!("&pageToken={}", q(t));
        }
        self.g.get(&url).await
    }

    async fn get_space_read_state(&self, space: &str) -> Result<SpaceReadState> {
        self.g.get(&me_path(&format!("{space}/spaceReadState"))).await
    }

    async fn update_space_read_state(&self, space: &str, last_read_time: &str) -> Result<SpaceReadState> {
        let url = format!(
            "{}?updateMask=lastReadTime",
            me_path(&format!("{space}/spaceReadState"))
        );
        self.g.patch(&url, &json!({ "lastReadTime": last_read_time })).await
    }

    async fn get_thread_read_state(&self, thread: &str) -> Result<ThreadReadState> {
        self.g.get(&me_path(&format!("{thread}/threadReadState"))).await
    }

    async fn list_space_events(
        &self,
        space: &str,
        start_time: &str,
        page_token: Option<&str>,
    ) -> Result<Page<SpaceEvent>> {
        let types = SPACE_EVENT_TYPES
            .iter()
            .map(|t| format!("event_types:\"{t}\""))
            .collect::<Vec<_>>()
            .join(" OR ");
        let filter = format!("start_time=\"{start_time}\" AND ({types})");
        let mut url = format!("{BASE}/{space}/spaceEvents?pageSize=1000&filter={}", q(&filter));
        if let Some(t) = page_token {
            url += &format!("&pageToken={}", q(t));
        }
        self.g.get(&url).await
    }

    async fn download_media(&self, resource_name: &str) -> Result<(Vec<u8>, Option<String>)> {
        // Resource names may contain '/', which the media path keeps literally.
        self.g.bytes(&format!("{BASE}/media/{resource_name}?alt=media")).await
    }
}

/// Builds the `createTime` filter for paging back.
pub fn older_than_filter(create_time: &str) -> String {
    format!("createTime < \"{create_time}\"")
}

pub fn thread_filter(thread: &str) -> String {
    format!("thread.name = {thread}")
}
