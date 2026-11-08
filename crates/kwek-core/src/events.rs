//! Workspace Events subscriptions and the Pub/Sub device subscription, plus the decoder that
//! turns event payloads into [`Event`]s. The same decoder serves the live Pub/Sub loop and
//! `spaces.spaceEvents.list` during reconcile, so both paths apply identical changes.
//!
//! Two Events subscriptions feed one topic:
//! - `//chat.googleapis.com/spaces/-`: messages, reactions, memberships, spaces in every
//!   space the user belongs to.
//! - `//cloudidentity.googleapis.com/users/{id}`: the user's read state and memberships.

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::chat::{Membership, Message, Reaction, Space, SpaceReadState, ThreadReadState};
use crate::config::{GoogleSetup, SharedSetup};
use crate::error::{Error, Result};
use crate::google::{Google, q};

const EVENTS: &str = "https://workspaceevents.googleapis.com/v1";
const PUBSUB: &str = "https://pubsub.googleapis.com/v1";

/// Events subscriptions with `includeResource: true` live at most 4 hours.
pub const SUBSCRIPTION_TTL_SECS: u64 = 4 * 3600;

pub const SPACES_TARGET: &str = "//chat.googleapis.com/spaces/-";

pub const SPACES_EVENT_TYPES: &[&str] = &[
    "google.workspace.chat.message.v1.created",
    "google.workspace.chat.message.v1.updated",
    "google.workspace.chat.message.v1.deleted",
    "google.workspace.chat.reaction.v1.created",
    "google.workspace.chat.reaction.v1.deleted",
    "google.workspace.chat.membership.v1.created",
    "google.workspace.chat.membership.v1.updated",
    "google.workspace.chat.membership.v1.deleted",
    "google.workspace.chat.space.v1.updated",
    "google.workspace.chat.space.v1.deleted",
];

pub const USER_EVENT_TYPES: &[&str] = &[
    "google.workspace.chat.spaceReadState.v1.updated",
    "google.workspace.chat.threadReadState.v1.updated",
    "google.workspace.chat.membership.v1.created",
    "google.workspace.chat.membership.v1.deleted",
];

pub fn user_target(user: &str) -> String {
    let id = user.trim_start_matches("users/");
    format!("//cloudidentity.googleapis.com/users/{id}")
}

/// What an event means for local state. Events are short-lived (decoded, applied, dropped),
/// so the size difference between variants does not matter.
#[derive(Debug, Clone, PartialEq)]
#[allow(clippy::large_enum_variant)]
pub enum Event {
    MessageUpserted(Message),
    MessageDeleted {
        name: String,
        delete_time: Option<String>,
    },
    ReactionCreated(Reaction),
    ReactionDeleted(Reaction),
    MembershipUpserted(Membership),
    MembershipDeleted(Membership),
    SpaceUpdated(Space),
    SpaceDeleted {
        name: String,
    },
    SpaceReadState(SpaceReadState),
    ThreadReadState(ThreadReadState),
    /// Subscription lifecycle: `expirationReminder`, `expired`, `suspended`, ...
    Lifecycle {
        kind: String,
        subscription: String,
    },
    Unknown(String),
}

/// Decodes one event's payload. `ce_type` is the CloudEvent type
/// (`google.workspace.chat.message.v1.created`); batch variants expand to several events.
pub fn decode_event_data(ce_type: &str, data: &Value) -> Vec<Event> {
    let one = |key: &str| data.get(key).cloned();
    let many = |key: &str, inner: &str| -> Vec<Value> {
        data.get(key)
            .and_then(Value::as_array)
            .map(|a| a.iter().filter_map(|e| e.get(inner).cloned()).collect())
            .unwrap_or_default()
    };
    fn parse<T: serde::de::DeserializeOwned>(v: Value) -> Option<T> {
        serde_json::from_value(v).ok()
    }

    if let Some(kind) = ce_type.strip_prefix("google.workspace.events.subscription.v1.") {
        let subscription = data
            .pointer("/subscription/name")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned();
        return vec![Event::Lifecycle {
            kind: kind.to_owned(),
            subscription,
        }];
    }

    let Some(rest) = ce_type.strip_prefix("google.workspace.chat.") else {
        return vec![Event::Unknown(ce_type.to_owned())];
    };
    let (resource, action) = match rest.split_once(".v1.") {
        Some(p) => p,
        None => return vec![Event::Unknown(ce_type.to_owned())],
    };

    let out: Vec<Event> = match (resource, action) {
        ("message", "created" | "updated") => one("message")
            .and_then(parse)
            .map(Event::MessageUpserted)
            .into_iter()
            .collect(),
        ("message", "batchCreated" | "batchUpdated") => many("messages", "message")
            .into_iter()
            .filter_map(parse)
            .map(Event::MessageUpserted)
            .collect(),
        ("message", "deleted") => one("message").and_then(|m| msg_deleted(&m)).into_iter().collect(),
        ("message", "batchDeleted") => many("messages", "message").iter().filter_map(msg_deleted).collect(),
        ("reaction", "created") => one("reaction")
            .and_then(parse)
            .map(Event::ReactionCreated)
            .into_iter()
            .collect(),
        ("reaction", "batchCreated") => many("reactions", "reaction")
            .into_iter()
            .filter_map(parse)
            .map(Event::ReactionCreated)
            .collect(),
        ("reaction", "deleted") => one("reaction")
            .and_then(parse)
            .map(Event::ReactionDeleted)
            .into_iter()
            .collect(),
        ("reaction", "batchDeleted") => many("reactions", "reaction")
            .into_iter()
            .filter_map(parse)
            .map(Event::ReactionDeleted)
            .collect(),
        ("membership", "created" | "updated") => one("membership")
            .and_then(parse)
            .map(Event::MembershipUpserted)
            .into_iter()
            .collect(),
        ("membership", "batchCreated" | "batchUpdated") => many("memberships", "membership")
            .into_iter()
            .filter_map(parse)
            .map(Event::MembershipUpserted)
            .collect(),
        ("membership", "deleted") => one("membership")
            .and_then(parse)
            .map(Event::MembershipDeleted)
            .into_iter()
            .collect(),
        ("membership", "batchDeleted") => many("memberships", "membership")
            .into_iter()
            .filter_map(parse)
            .map(Event::MembershipDeleted)
            .collect(),
        ("space", "updated") => one("space")
            .and_then(parse)
            .map(Event::SpaceUpdated)
            .into_iter()
            .collect(),
        ("space", "batchUpdated") => many("spaces", "space")
            .into_iter()
            .filter_map(parse)
            .map(Event::SpaceUpdated)
            .collect(),
        ("space", "deleted") => one("space")
            .and_then(|s| s.get("name").and_then(Value::as_str).map(str::to_owned))
            .map(|name| Event::SpaceDeleted { name })
            .into_iter()
            .collect(),
        ("spaceReadState", "updated") => one("spaceReadState")
            .and_then(parse)
            .map(Event::SpaceReadState)
            .into_iter()
            .collect(),
        ("spaceReadState", "batchUpdated") => many("spaceReadStates", "spaceReadState")
            .into_iter()
            .filter_map(parse)
            .map(Event::SpaceReadState)
            .collect(),
        ("threadReadState", "updated") => one("threadReadState")
            .and_then(parse)
            .map(Event::ThreadReadState)
            .into_iter()
            .collect(),
        ("threadReadState", "batchUpdated") => many("threadReadStates", "threadReadState")
            .into_iter()
            .filter_map(parse)
            .map(Event::ThreadReadState)
            .collect(),
        _ => vec![],
    };
    if out.is_empty() {
        vec![Event::Unknown(ce_type.to_owned())]
    } else {
        out
    }
}

fn msg_deleted(m: &Value) -> Option<Event> {
    Some(Event::MessageDeleted {
        name: m.get("name")?.as_str()?.to_owned(),
        delete_time: m.get("deleteTime").and_then(Value::as_str).map(str::to_owned),
    })
}

/// `spaces.spaceEvents.list` entries carry the payload under `<resource>EventData`
/// (`messageCreatedEventData: {message}`), which has the same inner shape as Pub/Sub data.
pub fn decode_space_event(ev: &crate::chat::SpaceEvent) -> Vec<Event> {
    let data = ev
        .data
        .iter()
        .find(|(k, _)| k.ends_with("EventData"))
        .map(|(_, v)| v.clone())
        .unwrap_or(Value::Null);
    decode_event_data(&ev.event_type, &data)
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Received {
    pub ack_id: String,
    pub message: PubsubMessage,
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct PubsubMessage {
    pub data: Option<String>,
    pub attributes: std::collections::HashMap<String, String>,
    pub message_id: String,
    pub publish_time: Option<String>,
}

impl Received {
    pub fn ce_type(&self) -> &str {
        self.message
            .attributes
            .get("ce-type")
            .map(String::as_str)
            .unwrap_or_default()
    }

    pub fn decode(&self) -> Vec<Event> {
        let data = self
            .message
            .data
            .as_deref()
            .and_then(|d| STANDARD.decode(d).ok())
            .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
            .unwrap_or(Value::Null);
        decode_event_data(self.ce_type(), &data)
    }
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct Subscription {
    pub name: String,
    pub state: Option<String>,
    pub expire_time: Option<String>,
    pub target_resource: Option<String>,
}

pub struct EventsClient {
    g: Google,
    setup: SharedSetup,
}

impl EventsClient {
    pub fn new(g: Google, setup: SharedSetup) -> Self {
        Self { g, setup }
    }

    fn setup(&self) -> Result<GoogleSetup> {
        crate::config::current(&self.setup).ok_or(Error::NoOAuthClient)
    }

    /// This device's pull subscription on the user's topic.
    pub fn device_subscription(&self, device_id: &str) -> Result<String> {
        Ok(self.setup()?.subscription(device_id))
    }

    /// Creates the device's pull subscription if missing. Unused subscriptions expire after
    /// 7 days, so a device that disappears cleans up after itself.
    pub async fn ensure_pubsub(&self, device_id: &str) -> Result<String> {
        let setup = self.setup()?;
        let name = setup.subscription(device_id);
        let body = json!({
            "topic": setup.topic_path(),
            "ackDeadlineSeconds": 60,
            "expirationPolicy": { "ttl": "604800s" },
            "messageRetentionDuration": "86400s",
        });
        match self.g.put::<Value>(&format!("{PUBSUB}/{name}"), &body).await {
            Ok(_) => Ok(name),
            Err(e) if e.is_status("ALREADY_EXISTS") => Ok(name),
            Err(e) => Err(e),
        }
    }

    pub async fn delete_pubsub(&self, device_id: &str) -> Result<()> {
        match self
            .g
            .delete(&format!("{PUBSUB}/{}", self.device_subscription(device_id)?))
            .await
        {
            Err(e) if e.is_status("NOT_FOUND") => Ok(()),
            r => r,
        }
    }

    /// Long-polls for up to `max` messages. An empty vec after the server's wait is normal.
    pub async fn pull(&self, subscription: &str, max: u32) -> Result<Vec<Received>> {
        #[derive(Deserialize, Default)]
        #[serde(rename_all = "camelCase", default)]
        struct Resp {
            received_messages: Vec<Received>,
        }
        // Pull long-polls; a quiet minute ends in a timeout, which is an empty batch.
        let r: Resp = self
            .g
            .long_poll(
                &format!("{PUBSUB}/{subscription}:pull"),
                &json!({ "maxMessages": max }),
                std::time::Duration::from_secs(60),
            )
            .await?;
        Ok(r.received_messages)
    }

    pub async fn ack(&self, subscription: &str, ack_ids: &[String]) -> Result<()> {
        if ack_ids.is_empty() {
            return Ok(());
        }
        self.g
            .post::<Value>(
                &format!("{PUBSUB}/{subscription}:acknowledge"),
                &json!({ "ackIds": ack_ids }),
            )
            .await
            .map(|_| ())
    }

    pub async fn list(&self, target: &str, event_types: &[&str]) -> Result<Vec<Subscription>> {
        let filter = format!("event_types:\"{}\" AND target_resource=\"{}\"", event_types[0], target);
        #[derive(Deserialize, Default)]
        #[serde(default)]
        struct Resp {
            subscriptions: Vec<Subscription>,
        }
        let r: Resp = self
            .g
            .get(&format!("{EVENTS}/subscriptions?filter={}", q(&filter)))
            .await?;
        Ok(r.subscriptions)
    }

    /// Finds or creates the Events subscription for `target`. Several devices share it, so an
    /// existing one is renewed (and reactivated if suspended) rather than duplicated.
    pub async fn ensure_subscription(
        &self,
        target: &str,
        event_types: &[&str],
        include_resource: bool,
    ) -> Result<Subscription> {
        if let Some(s) = self.list(target, event_types).await?.into_iter().next() {
            if s.state.as_deref() == Some("SUSPENDED") {
                self.reactivate(&s.name).await?;
            }
            return self.renew(&s.name).await;
        }
        let mut body = json!({
            "targetResource": target,
            "eventTypes": event_types,
            "notificationEndpoint": { "pubsubTopic": self.setup()?.topic_path() },
            "payloadOptions": { "includeResource": include_resource },
        });
        if include_resource {
            body["ttl"] = json!(format!("{SUBSCRIPTION_TTL_SECS}s"));
        }
        match self.g.post::<Value>(&format!("{EVENTS}/subscriptions"), &body).await {
            Ok(op) => self.subscription_from_operation(op, target, event_types).await,
            Err(e) if e.is_status("ALREADY_EXISTS") => self
                .list(target, event_types)
                .await?
                .into_iter()
                .next()
                .ok_or_else(|| Error::Other("subscription exists but is not listed".into())),
            Err(e) => Err(e),
        }
    }

    async fn subscription_from_operation(&self, op: Value, target: &str, types: &[&str]) -> Result<Subscription> {
        if let Some(resp) = op.get("response")
            && let Ok(s) = serde_json::from_value::<Subscription>(resp.clone())
            && !s.name.is_empty()
        {
            return Ok(s);
        }
        // Long-running operation still running: poll a few times, then fall back to listing.
        if let Some(name) = op.get("name").and_then(Value::as_str) {
            for _ in 0..10 {
                tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                let o: Value = self.g.get(&format!("{EVENTS}/{name}")).await?;
                if o.get("done").and_then(Value::as_bool) == Some(true) {
                    if let Some(err) = o.get("error") {
                        return Err(Error::Other(format!("subscription create failed: {err}")));
                    }
                    if let Some(resp) = o.get("response") {
                        return Ok(serde_json::from_value(resp.clone())?);
                    }
                }
            }
        }
        self.list(target, types)
            .await?
            .into_iter()
            .next()
            .ok_or_else(|| Error::Other("subscription create did not finish".into()))
    }

    pub async fn renew(&self, name: &str) -> Result<Subscription> {
        let url = format!("{EVENTS}/{name}?updateMask=ttl");
        let op: Value = self
            .g
            .patch(&url, &json!({ "ttl": format!("{SUBSCRIPTION_TTL_SECS}s") }))
            .await?;
        if let Some(resp) = op.get("response")
            && let Ok(s) = serde_json::from_value::<Subscription>(resp.clone())
            && !s.name.is_empty()
        {
            return Ok(s);
        }
        Ok(Subscription {
            name: name.to_owned(),
            ..Default::default()
        })
    }

    pub async fn reactivate(&self, name: &str) -> Result<()> {
        self.g
            .post::<Value>(&format!("{EVENTS}/{name}:reactivate"), &json!({}))
            .await
            .map(|_| ())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_single_and_batch_messages() {
        let d = json!({"message": {"name": "spaces/A/messages/1", "text": "hi"}});
        let ev = decode_event_data("google.workspace.chat.message.v1.created", &d);
        assert!(matches!(&ev[..], [Event::MessageUpserted(m)] if m.text.as_deref() == Some("hi")));

        let d = json!({"messages": [{"message": {"name": "spaces/A/messages/1"}}, {"message": {"name": "spaces/A/messages/2"}}]});
        let ev = decode_event_data("google.workspace.chat.message.v1.batchCreated", &d);
        assert_eq!(ev.len(), 2);
    }

    #[test]
    fn decodes_delete_and_read_state() {
        let d = json!({"message": {"name": "spaces/A/messages/1", "deleteTime": "2024-01-01T00:00:00Z"}});
        let ev = decode_event_data("google.workspace.chat.message.v1.deleted", &d);
        assert_eq!(
            ev,
            vec![Event::MessageDeleted {
                name: "spaces/A/messages/1".into(),
                delete_time: Some("2024-01-01T00:00:00Z".into())
            }]
        );
        let d = json!({"spaceReadState": {"name": "users/1/spaces/A/spaceReadState", "lastReadTime": "2024-01-01T00:00:00Z"}});
        let ev = decode_event_data("google.workspace.chat.spaceReadState.v1.updated", &d);
        assert!(matches!(&ev[..], [Event::SpaceReadState(s)] if s.space_name().as_deref() == Some("spaces/A")));
    }

    #[test]
    fn decodes_lifecycle_and_unknown() {
        let d = json!({"subscription": {"name": "subscriptions/x"}});
        let ev = decode_event_data("google.workspace.events.subscription.v1.expirationReminder", &d);
        assert_eq!(
            ev,
            vec![Event::Lifecycle {
                kind: "expirationReminder".into(),
                subscription: "subscriptions/x".into()
            }]
        );
        let ev = decode_event_data("google.workspace.chat.availability.v1.updated", &json!({}));
        assert!(matches!(&ev[..], [Event::Unknown(_)]));
    }

    #[test]
    fn decodes_pubsub_envelope() {
        let data =
            STANDARD.encode(br#"{"reaction":{"name":"spaces/A/messages/M/reactions/R","emoji":{"unicode":"x"}}}"#);
        let r: Received = serde_json::from_value(json!({
            "ackId": "a1",
            "message": {"data": data, "attributes": {"ce-type": "google.workspace.chat.reaction.v1.created"}, "messageId": "1"}
        }))
        .unwrap();
        assert!(matches!(&r.decode()[..], [Event::ReactionCreated(x)] if x.message_name() == "spaces/A/messages/M"));
    }

    #[test]
    fn decodes_space_event_list_entry() {
        let ev: crate::chat::SpaceEvent = serde_json::from_value(json!({
            "name": "spaces/A/spaceEvents/E",
            "eventType": "google.workspace.chat.message.v1.updated",
            "messageUpdatedEventData": {"message": {"name": "spaces/A/messages/1"}}
        }))
        .unwrap();
        assert!(matches!(&decode_space_event(&ev)[..], [Event::MessageUpserted(_)]));
    }
}
