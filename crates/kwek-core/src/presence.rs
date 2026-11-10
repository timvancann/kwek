//! Presence: your own Google Chat status, and colleagues' as their kwek broadcasts it.
//!
//! Google's availability API only covers the signed-in user, so every kwek reads its own
//! user's state, keeps it ACTIVE while they're at their computer (kwek's API calls don't
//! count as activity), and publishes it on a Pub/Sub topic the other kwek users read.
//! See docs/superpowers/specs/2026-11-10-presence-design.md.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::sync::Notify;
use unicode_segmentation::UnicodeSegmentation;

use crate::config::SharedSetup;
use crate::error::{Error, Result};
use crate::google::Google;
use crate::sync::Sink;
use crate::ts::{self, MINUTE, Micros, SECOND};
use crate::views::{CustomStatusView, OwnPresenceView, PresenceView};

/// How long one "active" mark lasts at Google.
pub const MARK_TTL: Micros = 5 * MINUTE;
/// Marks are renewed this often while the person is here.
pub const MARK_EVERY: Micros = 2 * MINUTE;
/// No keyboard or mouse for this long and the person is no longer here.
pub const IDLE_AFTER_SECS: u64 = 5 * 60;
/// A colleague's kwek quiet for this long: no dot, "last seen" instead.
pub const STALE_AFTER: Micros = 3 * MINUTE;
/// Messages older than this (by the sender's clock) are ignored.
const MAX_AGE: Micros = 10 * MINUTE;
/// ...and so are ones from more than this far in the future: a clock that's off.
const MAX_AHEAD: Micros = 2 * MINUTE;
const MAX_TEXT: usize = 100;
/// The longest real emoji sequences (families, flags with tags) stay well under this.
const MAX_EMOJI_CHARS: usize = 16;
/// A message is a few hundred bytes; anything far bigger isn't from kwek.
const MAX_MSG: usize = 2048;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    Active,
    Idle,
    Away,
    Dnd,
}

impl State {
    /// From the API's (and the topic's) names.
    pub fn from_api(s: &str) -> Option<Self> {
        Some(match s {
            "ACTIVE" => State::Active,
            "IDLE" => State::Idle,
            "AWAY" => State::Away,
            "DO_NOT_DISTURB" => State::Dnd,
            _ => return None,
        })
    }

    pub fn api(self) -> &'static str {
        match self {
            State::Active => "ACTIVE",
            State::Idle => "IDLE",
            State::Away => "AWAY",
            State::Dnd => "DO_NOT_DISTURB",
        }
    }

    /// For the webview.
    pub fn short(self) -> &'static str {
        match self {
            State::Active => "active",
            State::Idle => "idle",
            State::Away => "away",
            State::Dnd => "dnd",
        }
    }
}

// --------------------------------------------------------------- own presence

pub struct DecideIn {
    /// Keyboard or mouse used in the last few minutes, anywhere on the machine.
    pub here: bool,
    pub google: State,
    /// The person set away themselves; kwek leaves it alone.
    pub manual_away: bool,
    /// When kwek last marked them active.
    pub last_mark: Option<Micros>,
    pub now: Micros,
}

#[derive(Debug, PartialEq, Eq)]
pub struct DecideOut {
    pub mark_active: bool,
    pub manual_away: bool,
}

/// What to do about the person's own state, once per tick. kwek never touches do not
/// disturb or an away the person chose. It can't ask Google whether an away was manual, so
/// it infers it: an away that shows up while kwek's own active mark should still be running
/// was set by hand. Any other away (the mark ran out overnight, or after a break) is
/// automatic, and kwek marks active again once the person is back.
pub fn decide(i: &DecideIn) -> DecideOut {
    let mark_running = i.last_mark.is_some_and(|t| i.now < t + MARK_TTL);
    let due = i.last_mark.is_none_or(|t| i.now - t >= MARK_EVERY);
    match i.google {
        // Do not disturb replaces any away; when it ends Google moves to away by itself,
        // and that away is automatic.
        State::Dnd => DecideOut {
            mark_active: false,
            manual_away: false,
        },
        State::Away if i.manual_away || mark_running => DecideOut {
            mark_active: false,
            manual_away: true,
        },
        State::Away => DecideOut {
            mark_active: i.here,
            manual_away: false,
        },
        State::Active | State::Idle => DecideOut {
            mark_active: i.here && due,
            manual_away: false,
        },
    }
}

// --------------------------------------------------------------- the topic

/// One message on the presence topic (spec §3.3).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PresenceMsg {
    pub v: u32,
    pub user: String,
    pub state: String,
    #[serde(default)]
    pub dnd_until: Option<String>,
    #[serde(default)]
    pub custom: Option<CustomMsg>,
    pub sent_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CustomMsg {
    pub emoji: String,
    pub text: String,
    #[serde(default)]
    pub expire_time: Option<String>,
}

/// The person's own availability as Google reports it.
#[derive(Debug, Clone, PartialEq)]
pub struct Own {
    pub state: State,
    pub dnd_until: Option<Micros>,
    pub custom: Option<CustomStatusView>,
}

impl PresenceMsg {
    pub fn own(me: &str, own: &Own, now: Micros) -> Self {
        PresenceMsg {
            v: 1,
            user: me.to_owned(),
            state: own.state.api().to_owned(),
            dnd_until: own.dnd_until.map(ts::format),
            custom: own.custom.as_ref().map(|c| CustomMsg {
                emoji: c.emoji.clone(),
                text: c.text.clone(),
                expire_time: c.until_us.map(ts::format),
            }),
            sent_at: ts::format(now),
        }
    }
}

/// What kwek holds for a colleague.
#[derive(Debug, Clone, PartialEq)]
pub struct Held {
    pub user: String,
    pub state: State,
    pub dnd_until: Option<Micros>,
    pub custom: Option<CustomStatusView>,
    /// By the sender's clock: orders their messages.
    pub sent: Micros,
    /// By this machine's clock: says whether their kwek is still running.
    pub received: Micros,
}

/// A message from the topic, if it is one to keep. Everything in it is untrusted: anyone
/// with publish rights on the topic could have sent it (spec D5).
pub fn accept(data: &[u8], knows: impl Fn(&str) -> bool, me: &str, now: Micros, held: Option<&Held>) -> Option<Held> {
    if data.len() > MAX_MSG {
        return None;
    }
    let m: PresenceMsg = serde_json::from_slice(data).ok()?;
    if m.v != 1 || m.user == me || !knows(&m.user) {
        return None;
    }
    let state = State::from_api(&m.state)?;
    let sent = ts::parse(&m.sent_at)?;
    if sent < now - MAX_AGE || sent > now + MAX_AHEAD || held.is_some_and(|h| sent <= h.sent) {
        return None;
    }
    let custom = m.custom.and_then(|c| {
        // One grapheme, and a sane one: a base with thousands of combining marks is one too.
        let emoji = c
            .emoji
            .graphemes(true)
            .next()
            .filter(|g| g.chars().count() <= MAX_EMOJI_CHARS)
            .unwrap_or("")
            .to_owned();
        let text: String = c.text.chars().take(MAX_TEXT).collect();
        let text = text.trim().to_owned();
        (!text.is_empty() || !emoji.is_empty()).then(|| CustomStatusView {
            emoji,
            text,
            until_us: c.expire_time.as_deref().and_then(ts::parse),
        })
    });
    Some(Held {
        user: m.user,
        state,
        dnd_until: m.dnd_until.as_deref().and_then(ts::parse),
        custom,
        sent,
        received: now,
    })
}

/// How a colleague shows right now: no dot once their kwek has gone quiet, no custom status
/// once it has expired.
pub fn view_of(h: &Held, now: Micros) -> PresenceView {
    let fresh = now - h.received < STALE_AFTER;
    PresenceView {
        user: h.user.clone(),
        state: fresh.then(|| h.state.short().to_owned()),
        last_seen_us: h.received,
        dnd_until_us: h.dnd_until.filter(|_| fresh && h.state == State::Dnd),
        custom: h.custom.clone().filter(|c| c.until_us.is_none_or(|u| now < u)),
    }
}

/// Idle seconds to here or not.
pub fn is_here(idle_secs: u64) -> bool {
    idle_secs < IDLE_AFTER_SECS
}

// --------------------------------------------------------------- the outside world

/// A custom status to set. Google requires both an emoji and text.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NewCustom {
    pub emoji: String,
    pub text: String,
    /// Clears itself after this long; None keeps it until changed.
    pub ttl_secs: Option<i64>,
}

/// The availability the person picked.
#[derive(Debug, Clone, Copy, PartialEq, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Choice {
    /// Active while you're around, as kwek and Google see it.
    Automatic,
    Away,
    #[serde(rename_all = "camelCase")]
    Dnd {
        until_us: Micros,
    },
}

/// Google Chat's availability API, for the signed-in user only.
#[async_trait]
pub trait AvailabilityApi: Send + Sync {
    async fn get(&self) -> Result<Own>;
    async fn mark_active(&self, ttl: Micros) -> Result<()>;
    async fn mark_away(&self) -> Result<()>;
    async fn mark_dnd(&self, until: Micros) -> Result<()>;
    /// None clears it.
    async fn set_custom(&self, custom: Option<&NewCustom>) -> Result<()>;
}

/// The presence topic and this device's subscription to it.
#[async_trait]
pub trait PresenceBus: Send + Sync {
    /// Creates the device's subscription if missing. NOT_FOUND or PERMISSION_DENIED means
    /// there's no team presence in this project.
    async fn ensure(&self, device: &str) -> Result<()>;
    async fn publish(&self, data: Vec<u8>) -> Result<()>;
    /// Long-polls; returns (ack id, data) pairs.
    async fn pull(&self, device: &str) -> Result<Vec<(String, Vec<u8>)>>;
    async fn ack(&self, device: &str, ack_ids: &[String]) -> Result<()>;
    async fn delete(&self, device: &str) -> Result<()>;
}

/// Seconds since the last keyboard or mouse input, anywhere on the machine.
pub trait IdleSource: Send + Sync {
    fn idle_secs(&self) -> u64;
}

/// What presence needs from the rest of kwek.
pub trait Roster: Send + Sync {
    /// The signed-in user (`users/{id}`), None while signed out.
    fn me(&self) -> Option<String>;
    fn device_id(&self) -> Option<String>;
    /// Someone kwek has seen: a DM peer, a member, a sender.
    fn knows(&self, user: &str) -> bool;
    fn get(&self, key: &str) -> Option<String>;
    fn set(&self, key: &str, value: &str);
}

const CHAT: &str = "https://chat.googleapis.com/v1/users/me/availability";

pub struct HttpAvailability {
    g: Google,
}

impl HttpAvailability {
    pub fn new(g: Google) -> Self {
        Self { g }
    }
}

/// The Availability resource as far as kwek reads it.
pub fn parse_own(v: &Value) -> Own {
    let state = v
        .get("state")
        .and_then(|s| s.as_str())
        .and_then(State::from_api)
        .unwrap_or(State::Away);
    let custom = v.get("customStatus").and_then(|c| {
        let text = c.get("text").and_then(|t| t.as_str()).unwrap_or("").trim().to_owned();
        let emoji = c
            .pointer("/emoji/unicode")
            .and_then(|t| t.as_str())
            .unwrap_or("")
            .to_owned();
        (!text.is_empty() || !emoji.is_empty()).then(|| CustomStatusView {
            emoji,
            text,
            until_us: c.get("expireTime").and_then(|t| t.as_str()).and_then(ts::parse),
        })
    });
    Own {
        state,
        dnd_until: v
            .pointer("/doNotDisturbMetadata/expirationTime")
            .and_then(|t| t.as_str())
            .and_then(ts::parse),
        custom,
    }
}

#[async_trait]
impl AvailabilityApi for HttpAvailability {
    async fn get(&self) -> Result<Own> {
        let v: Value = self.g.get(CHAT).await?;
        Ok(parse_own(&v))
    }

    async fn mark_active(&self, ttl: Micros) -> Result<()> {
        let body = json!({ "ttl": format!("{}s", ttl / SECOND) });
        self.g
            .post::<Value>(&format!("{CHAT}:markAsActive"), &body)
            .await
            .map(|_| ())
    }

    async fn mark_away(&self) -> Result<()> {
        self.g
            .post::<Value>(&format!("{CHAT}:markAsAway"), &json!({}))
            .await
            .map(|_| ())
    }

    async fn mark_dnd(&self, until: Micros) -> Result<()> {
        let body = json!({ "expireTime": ts::format(until) });
        self.g
            .post::<Value>(&format!("{CHAT}:markAsDoNotDisturb"), &body)
            .await
            .map(|_| ())
    }

    async fn set_custom(&self, custom: Option<&NewCustom>) -> Result<()> {
        // Leaving customStatus out of the body, with it in the mask, clears it.
        let body = match custom {
            Some(c) => {
                let mut status = json!({ "text": c.text, "emoji": { "unicode": c.emoji } });
                if let Some(ttl) = c.ttl_secs {
                    status["ttl"] = json!(format!("{ttl}s"));
                }
                json!({ "customStatus": status })
            }
            None => json!({}),
        };
        self.g
            .patch::<Value>(&format!("{CHAT}?updateMask=customStatus"), &body)
            .await
            .map(|_| ())
    }
}

const PUBSUB: &str = "https://pubsub.googleapis.com/v1";

pub struct HttpPresenceBus {
    g: Google,
    setup: SharedSetup,
}

impl HttpPresenceBus {
    pub fn new(g: Google, setup: SharedSetup) -> Self {
        Self { g, setup }
    }

    fn setup(&self) -> Result<crate::config::GoogleSetup> {
        crate::config::current(&self.setup).ok_or(Error::NoOAuthClient)
    }
}

#[async_trait]
impl PresenceBus for HttpPresenceBus {
    async fn ensure(&self, device: &str) -> Result<()> {
        let s = self.setup()?;
        let body = json!({
            "topic": s.presence_topic_path(),
            "ackDeadlineSeconds": 30,
            "expirationPolicy": { "ttl": "604800s" },
            "messageRetentionDuration": "600s",
        });
        match self
            .g
            .put::<Value>(&format!("{PUBSUB}/{}", s.presence_subscription(device)), &body)
            .await
        {
            Ok(_) => Ok(()),
            Err(e) if e.is_status("ALREADY_EXISTS") => Ok(()),
            Err(e) => Err(e),
        }
    }

    async fn publish(&self, data: Vec<u8>) -> Result<()> {
        let s = self.setup()?;
        let body = json!({ "messages": [{ "data": STANDARD.encode(data) }] });
        self.g
            .post::<Value>(&format!("{PUBSUB}/{}:publish", s.presence_topic_path()), &body)
            .await
            .map(|_| ())
    }

    async fn pull(&self, device: &str) -> Result<Vec<(String, Vec<u8>)>> {
        #[derive(Deserialize, Default)]
        #[serde(rename_all = "camelCase", default)]
        struct Resp {
            received_messages: Vec<crate::events::Received>,
        }
        let s = self.setup()?;
        let r: Resp = self
            .g
            .long_poll(
                &format!("{PUBSUB}/{}:pull", s.presence_subscription(device)),
                &json!({ "maxMessages": 100 }),
                std::time::Duration::from_secs(60),
            )
            .await?;
        Ok(r.received_messages
            .into_iter()
            .map(|m| {
                let data = m
                    .message
                    .data
                    .as_deref()
                    .and_then(|d| STANDARD.decode(d).ok())
                    .unwrap_or_default();
                (m.ack_id, data)
            })
            .collect())
    }

    async fn ack(&self, device: &str, ack_ids: &[String]) -> Result<()> {
        if ack_ids.is_empty() {
            return Ok(());
        }
        let s = self.setup()?;
        self.g
            .post::<Value>(
                &format!("{PUBSUB}/{}:acknowledge", s.presence_subscription(device)),
                &json!({ "ackIds": ack_ids }),
            )
            .await
            .map(|_| ())
    }

    async fn delete(&self, device: &str) -> Result<()> {
        let s = self.setup()?;
        match self
            .g
            .delete(&format!("{PUBSUB}/{}", s.presence_subscription(device)))
            .await
        {
            Err(e) if e.is_status("NOT_FOUND") => Ok(()),
            r => r,
        }
    }
}

// --------------------------------------------------------------- the runtime

/// How often your own state is checked.
const TICK: Duration = Duration::from_secs(30);
/// A heartbeat this often, even when nothing changed.
const HEARTBEAT: Micros = MINUTE;
/// With no presence topic, look again this often.
const TEAM_RETRY: Micros = 10 * MINUTE;

const KV_MANUAL_AWAY: &str = "presence_manual_away";
const KV_LAST_MARK: &str = "presence_last_mark_us";

#[derive(Default)]
struct Inner {
    own: Option<Own>,
    access: &'static str,
    manual_away: bool,
    last_mark: Option<Micros>,
    loaded: bool,
    /// The topic works; None until tried.
    team: Option<bool>,
    team_tried: Option<Micros>,
    /// What was last published, and when.
    published: Option<(Own, Micros)>,
    held: HashMap<String, Held>,
    /// What the webview was last told, per colleague.
    shown: HashMap<String, PresenceView>,
    own_shown: Option<OwnPresenceView>,
}

/// Your own presence and your colleagues'. Runs beside chat sync and never gets in its way:
/// every error is logged and tried again next tick.
pub struct Presence {
    api: Arc<dyn AvailabilityApi>,
    bus: Arc<dyn PresenceBus>,
    idle: Arc<dyn IdleSource>,
    roster: Arc<dyn Roster>,
    sink: Arc<dyn Sink>,
    inner: Mutex<Inner>,
    wake: Notify,
}

/// A permission the sign-in doesn't include yet (the scopes came later).
fn needs_permission(e: &Error) -> bool {
    e.http_status() == Some(403)
}

/// No presence topic, or no right to use it: no team presence here.
fn no_team(e: &Error) -> bool {
    matches!(e.http_status(), Some(403 | 404))
}

impl Presence {
    pub fn new(
        api: Arc<dyn AvailabilityApi>,
        bus: Arc<dyn PresenceBus>,
        idle: Arc<dyn IdleSource>,
        roster: Arc<dyn Roster>,
        sink: Arc<dyn Sink>,
    ) -> Arc<Self> {
        Arc::new(Self {
            api,
            bus,
            idle,
            roster,
            sink,
            inner: Mutex::new(Inner {
                access: "off",
                ..Default::default()
            }),
            wake: Notify::new(),
        })
    }

    /// Check again now (after signing in, granting access or changing status).
    pub fn poke(&self) {
        self.wake.notify_waiters();
        self.wake.notify_one();
    }

    pub fn own(&self) -> OwnPresenceView {
        let i = self.inner.lock().unwrap();
        own_view(&i)
    }

    pub fn all(&self) -> Vec<PresenceView> {
        let now = ts::now();
        let i = self.inner.lock().unwrap();
        i.held.values().map(|h| view_of(h, now)).collect()
    }

    /// Own state, forever.
    pub async fn run(self: Arc<Self>) {
        loop {
            self.tick(ts::now()).await;
            tokio::select! {
                _ = self.wake.notified() => {}
                _ = tokio::time::sleep(TICK) => {}
            }
        }
    }

    /// Colleagues' state, forever.
    pub async fn listen(self: Arc<Self>) {
        loop {
            match self.listen_once().await {
                Ok(true) => {}
                Ok(false) => {
                    tokio::select! {
                        _ = self.wake.notified() => {}
                        _ = tokio::time::sleep(TICK) => {}
                    }
                }
                Err(e) => {
                    tracing::debug!("presence pull failed: {e}");
                    tokio::time::sleep(Duration::from_secs(10)).await;
                }
            }
        }
    }

    /// One pull. Ok(false) when there's nothing to listen to (signed out, no topic).
    async fn listen_once(&self) -> Result<bool> {
        let (Some(_), Some(device)) = (self.roster.me(), self.roster.device_id()) else {
            return Ok(false);
        };
        if !self.join_team(&device, ts::now()).await {
            return Ok(false);
        }
        match self.bus.pull(&device).await {
            Ok(batch) => {
                let ids: Vec<String> = batch.iter().map(|(id, _)| id.clone()).collect();
                self.receive(batch.iter().map(|(_, d)| d.as_slice()), ts::now());
                self.bus.ack(&device, &ids).await?;
                Ok(true)
            }
            // The subscription expired or was deleted: make it again next time.
            Err(e) if e.is_status("NOT_FOUND") => {
                self.inner.lock().unwrap().team = None;
                Ok(true)
            }
            Err(e) => Err(e),
        }
    }

    /// Makes sure the device subscription exists; false when there's no team presence.
    async fn join_team(&self, device: &str, now: Micros) -> bool {
        {
            let i = self.inner.lock().unwrap();
            match (i.team, i.team_tried) {
                (Some(true), _) => return true,
                (Some(false), Some(t)) if now - t < TEAM_RETRY => return false,
                _ => {}
            }
        }
        let result = self.bus.ensure(device).await;
        let team = match &result {
            Ok(()) => Some(true),
            Err(e) if no_team(e) => {
                tracing::info!("no team presence in this project: {e}");
                Some(false)
            }
            Err(e) => {
                tracing::debug!("presence subscription not ready: {e}");
                None
            }
        };
        {
            let mut i = self.inner.lock().unwrap();
            i.team = team;
            i.team_tried = Some(now);
        }
        self.show_own();
        team == Some(true)
    }

    /// Messages from the topic. Checked and parsed once each, outside the lock (`accept`
    /// refuses oversized ones before parsing); only the ordering needs what's held.
    pub fn receive<'a>(&self, batch: impl Iterator<Item = &'a [u8]>, now: Micros) {
        let Some(me) = self.roster.me() else { return };
        let fresh: Vec<Held> = batch
            .filter_map(|data| accept(data, |u| self.roster.knows(u), &me, now, None))
            .collect();
        {
            let mut i = self.inner.lock().unwrap();
            for h in fresh {
                if i.held.get(&h.user).is_none_or(|old| h.sent > old.sent) {
                    i.held.insert(h.user.clone(), h);
                }
            }
        }
        self.show_others(now);
    }

    /// One look at your own state: keep Google right, tell colleagues.
    pub async fn tick(&self, now: Micros) {
        let Some(me) = self.roster.me() else {
            self.signed_out();
            return;
        };
        self.load();
        let own = match self.api.get().await {
            Ok(o) => o,
            Err(e) => {
                let access = match &e {
                    Error::AuthRequired | Error::NoOAuthClient => "off",
                    e if needs_permission(e) => "needsPermission",
                    _ => {
                        tracing::debug!("availability unavailable: {e}");
                        return;
                    }
                };
                self.inner.lock().unwrap().access = access;
                self.show_own();
                return;
            }
        };
        let decision = {
            let i = self.inner.lock().unwrap();
            decide(&DecideIn {
                here: is_here(self.idle.idle_secs()),
                google: own.state,
                manual_away: i.manual_away,
                last_mark: i.last_mark,
                now,
            })
        };
        let mut own = own;
        if decision.mark_active {
            match self.api.mark_active(MARK_TTL).await {
                Ok(()) => {
                    own.state = State::Active;
                    self.inner.lock().unwrap().last_mark = Some(now);
                    self.roster.set(KV_LAST_MARK, &now.to_string());
                }
                Err(e) => tracing::debug!("could not mark active: {e}"),
            }
        }
        self.set_manual_away(decision.manual_away);
        {
            let mut i = self.inner.lock().unwrap();
            i.access = "ok";
            i.own = Some(own);
        }
        self.show_own();
        self.publish(&me, now).await;
        self.show_others(now);
    }

    /// The person picked a status in kwek.
    pub async fn set_status(&self, custom: Option<NewCustom>, choice: Choice) -> Result<()> {
        let now = ts::now();
        self.api.set_custom(custom.as_ref()).await?;
        match choice {
            Choice::Automatic => {
                self.api.mark_active(MARK_TTL).await?;
                self.inner.lock().unwrap().last_mark = Some(now);
                self.roster.set(KV_LAST_MARK, &now.to_string());
                self.set_manual_away(false);
            }
            Choice::Away => {
                self.api.mark_away().await?;
                self.set_manual_away(true);
            }
            Choice::Dnd { until_us } => {
                self.api.mark_dnd(until_us).await?;
                self.set_manual_away(false);
            }
        }
        // Read it back and tell everyone straight away.
        self.tick(now).await;
        Ok(())
    }

    /// Signing out: take the device's subscription down while the sign-in still works.
    pub async fn leave(&self) {
        if let Some(device) = self.roster.device_id()
            && let Err(e) = self.bus.delete(&device).await
        {
            tracing::debug!("could not delete presence subscription: {e}");
        }
        self.signed_out();
    }

    fn signed_out(&self) {
        let cleared: Vec<String> = {
            let mut i = self.inner.lock().unwrap();
            let users = i.held.keys().cloned().collect();
            *i = Inner {
                access: "off",
                ..Default::default()
            };
            users
        };
        if !cleared.is_empty() {
            self.sink.presence(vec![]);
        }
        self.show_own();
    }

    fn load(&self) {
        let mut i = self.inner.lock().unwrap();
        if i.loaded {
            return;
        }
        i.loaded = true;
        i.manual_away = self.roster.get(KV_MANUAL_AWAY).as_deref() == Some("1");
        i.last_mark = self.roster.get(KV_LAST_MARK).and_then(|v| v.parse().ok());
    }

    fn set_manual_away(&self, on: bool) {
        let changed = {
            let mut i = self.inner.lock().unwrap();
            let changed = i.manual_away != on;
            i.manual_away = on;
            changed
        };
        if changed {
            self.roster.set(KV_MANUAL_AWAY, if on { "1" } else { "0" });
        }
    }

    async fn publish(&self, me: &str, now: Micros) {
        let own = {
            let i = self.inner.lock().unwrap();
            if i.team != Some(true) {
                return;
            }
            let Some(own) = i.own.clone() else { return };
            let due = match &i.published {
                Some((last, at)) => *last != own || now - at >= HEARTBEAT,
                None => true,
            };
            if !due {
                return;
            }
            own
        };
        let data = match serde_json::to_vec(&PresenceMsg::own(me, &own, now)) {
            Ok(d) => d,
            Err(_) => return,
        };
        match self.bus.publish(data).await {
            Ok(()) => self.inner.lock().unwrap().published = Some((own, now)),
            Err(e) if no_team(&e) => {
                tracing::info!("presence topic unusable: {e}");
                self.inner.lock().unwrap().team = Some(false);
                self.show_own();
            }
            Err(e) => tracing::debug!("could not publish presence: {e}"),
        }
    }

    fn show_own(&self) {
        let view = {
            let mut i = self.inner.lock().unwrap();
            let view = own_view(&i);
            if i.own_shown.as_ref() == Some(&view) {
                return;
            }
            i.own_shown = Some(view.clone());
            view
        };
        self.sink.own_presence(view);
    }

    /// Tells the webview about colleagues whose view changed (a message, or going quiet).
    fn show_others(&self, now: Micros) {
        let changed: Vec<PresenceView> = {
            let mut i = self.inner.lock().unwrap();
            let views: Vec<PresenceView> = i.held.values().map(|h| view_of(h, now)).collect();
            let changed: Vec<PresenceView> = views.into_iter().filter(|v| i.shown.get(&v.user) != Some(v)).collect();
            for v in &changed {
                i.shown.insert(v.user.clone(), v.clone());
            }
            changed
        };
        if !changed.is_empty() {
            self.sink.presence(changed);
        }
    }
}

fn own_view(i: &Inner) -> OwnPresenceView {
    let now = ts::now();
    OwnPresenceView {
        access: i.access.to_owned(),
        state: i.own.as_ref().map(|o| o.state.short().to_owned()),
        manual_away: i.manual_away,
        dnd_until_us: i.own.as_ref().and_then(|o| o.dnd_until),
        custom: i
            .own
            .as_ref()
            .and_then(|o| o.custom.clone())
            .filter(|c| c.until_us.is_none_or(|u| now < u)),
        team: i.team == Some(true),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: Micros = 1_800_000_000 * SECOND;

    fn d(here: bool, google: State, manual_away: bool, last_mark: Option<Micros>) -> DecideOut {
        decide(&DecideIn {
            here,
            google,
            manual_away,
            last_mark,
            now: NOW,
        })
    }

    fn out(mark_active: bool, manual_away: bool) -> DecideOut {
        DecideOut {
            mark_active,
            manual_away,
        }
    }

    #[test]
    fn here_and_active_keeps_marking_but_not_every_tick() {
        assert_eq!(d(true, State::Active, false, None), out(true, false));
        assert_eq!(d(true, State::Idle, false, Some(NOW - 3 * MINUTE)), out(true, false));
        assert_eq!(
            d(true, State::Active, false, Some(NOW - MINUTE)),
            out(false, false),
            "marked a minute ago"
        );
    }

    #[test]
    fn gone_does_nothing() {
        assert_eq!(
            d(false, State::Active, false, Some(NOW - 3 * MINUTE)),
            out(false, false)
        );
        assert_eq!(d(false, State::Away, false, None), out(false, false));
    }

    #[test]
    fn away_while_the_mark_runs_was_set_by_hand_and_holds() {
        // kwek marked a minute ago; Google says away: only a person does that.
        assert_eq!(d(true, State::Away, false, Some(NOW - MINUTE)), out(false, true));
        // Hours later, still away and remembered as manual: left alone.
        assert_eq!(
            d(true, State::Away, true, Some(NOW - 3 * 60 * MINUTE)),
            out(false, true)
        );
    }

    #[test]
    fn away_after_the_night_is_automatic() {
        // kwek's last mark ran out long ago (the laptop slept); back at the keyboard.
        assert_eq!(
            d(true, State::Away, false, Some(NOW - 10 * 60 * MINUTE)),
            out(true, false)
        );
        assert_eq!(d(true, State::Away, false, None), out(true, false), "first start");
    }

    #[test]
    fn do_not_disturb_is_never_touched_and_ends_manual_away() {
        assert_eq!(d(true, State::Dnd, false, None), out(false, false));
        assert_eq!(d(true, State::Dnd, true, Some(NOW - 3 * MINUTE)), out(false, false));
    }

    #[test]
    fn active_in_a_google_client_clears_manual_away() {
        assert_eq!(d(true, State::Active, true, Some(NOW - 3 * MINUTE)), out(true, false));
    }

    const ME: &str = "users/me";
    const SAM: &str = "users/sam";

    fn knows(u: &str) -> bool {
        u == SAM || u == ME
    }

    fn msg(user: &str, state: &str, sent: Micros) -> Vec<u8> {
        serde_json::to_vec(&serde_json::json!({
            "v": 1, "user": user, "state": state, "sentAt": ts::format(sent),
            "custom": { "emoji": "🎧", "text": "Deep work", "expireTime": ts::format(NOW + 60 * MINUTE) }
        }))
        .unwrap()
    }

    #[test]
    fn keeps_a_good_message() {
        let h = accept(&msg(SAM, "DO_NOT_DISTURB", NOW - SECOND), knows, ME, NOW, None).unwrap();
        assert_eq!(h.state, State::Dnd);
        assert_eq!(h.received, NOW);
        let c = h.custom.unwrap();
        assert_eq!((c.emoji.as_str(), c.text.as_str()), ("🎧", "Deep work"));
        assert_eq!(c.until_us, Some(NOW + 60 * MINUTE));
    }

    #[test]
    fn drops_what_it_should_not_trust() {
        let none = |data: &[u8], held: Option<&Held>| accept(data, knows, ME, NOW, held).is_none();
        assert!(none(&msg("users/stranger", "ACTIVE", NOW), None), "unknown user");
        assert!(none(&msg(ME, "ACTIVE", NOW), None), "our own message comes back too");
        assert!(none(&msg(SAM, "ONLINE", NOW), None), "no such state");
        assert!(none(&msg(SAM, "ACTIVE", NOW - 11 * MINUTE), None), "too old");
        assert!(none(&msg(SAM, "ACTIVE", NOW + 5 * MINUTE), None), "from the future");
        assert!(none(b"{not json", None));
        assert!(none(
            br#"{"v":2,"user":"users/sam","state":"ACTIVE","sentAt":"2027-01-15T08:00:00Z"}"#,
            None
        ));
        let newer = accept(&msg(SAM, "ACTIVE", NOW - SECOND), knows, ME, NOW, None).unwrap();
        assert!(none(&msg(SAM, "AWAY", NOW - 2 * SECOND), Some(&newer)), "out of order");
    }

    #[test]
    fn caps_what_it_shows() {
        let long = "x".repeat(500);
        let data = serde_json::to_vec(&serde_json::json!({
            "v": 1, "user": SAM, "state": "ACTIVE", "sentAt": ts::format(NOW),
            "custom": { "emoji": "👩‍👩‍👧‍👦🎉🎉", "text": long }
        }))
        .unwrap();
        let c = accept(&data, knows, ME, NOW, None).unwrap().custom.unwrap();
        assert_eq!(c.text.chars().count(), MAX_TEXT);
        assert_eq!(c.emoji, "👩‍👩‍👧‍👦", "one emoji, even a joined one");

        let zalgo = format!("e{}", "\u{301}".repeat(1000));
        let data = serde_json::to_vec(&serde_json::json!({
            "v": 1, "user": SAM, "state": "ACTIVE", "sentAt": ts::format(NOW),
            "custom": { "emoji": zalgo, "text": "hi" }
        }))
        .unwrap();
        assert!(data.len() > MAX_MSG);
        assert!(accept(&data, knows, ME, NOW, None).is_none(), "oversized message");
        let zalgo = format!("e{}", "\u{301}".repeat(40));
        let data = serde_json::to_vec(&serde_json::json!({
            "v": 1, "user": SAM, "state": "ACTIVE", "sentAt": ts::format(NOW),
            "custom": { "emoji": zalgo, "text": "hi" }
        }))
        .unwrap();
        let c = accept(&data, knows, ME, NOW, None).unwrap().custom.unwrap();
        assert_eq!(
            (c.emoji.as_str(), c.text.as_str()),
            ("", "hi"),
            "a monster grapheme is dropped"
        );
    }

    #[test]
    fn goes_quiet_after_three_minutes_and_status_expires() {
        let h = accept(&msg(SAM, "ACTIVE", NOW), knows, ME, NOW, None).unwrap();
        assert_eq!(view_of(&h, NOW + STALE_AFTER - SECOND).state.as_deref(), Some("active"));
        let quiet = view_of(&h, NOW + STALE_AFTER);
        assert_eq!(quiet.state, None);
        assert_eq!(quiet.last_seen_us, NOW);
        assert!(view_of(&h, NOW + 59 * MINUTE).custom.is_some());
        assert!(view_of(&h, NOW + 60 * MINUTE).custom.is_none());
    }

    // ---------------------------------------------------------- the runtime, with fakes

    use crate::sync::Notification;
    use crate::views::{AuthState, MessageView, SpaceSummary, SyncStatus};
    use std::sync::atomic::{AtomicU64, Ordering};

    struct FakeApi {
        own: Mutex<Own>,
        fail: Mutex<Option<u16>>,
        calls: Mutex<Vec<String>>,
    }

    fn api_error(status: u16) -> Error {
        Error::Api {
            status,
            status_code: None,
            message: "no".into(),
        }
    }

    #[async_trait]
    impl AvailabilityApi for FakeApi {
        async fn get(&self) -> Result<Own> {
            if let Some(s) = *self.fail.lock().unwrap() {
                return Err(api_error(s));
            }
            Ok(self.own.lock().unwrap().clone())
        }
        async fn mark_active(&self, _: Micros) -> Result<()> {
            self.calls.lock().unwrap().push("active".into());
            self.own.lock().unwrap().state = State::Active;
            Ok(())
        }
        async fn mark_away(&self) -> Result<()> {
            self.calls.lock().unwrap().push("away".into());
            self.own.lock().unwrap().state = State::Away;
            Ok(())
        }
        async fn mark_dnd(&self, _: Micros) -> Result<()> {
            self.calls.lock().unwrap().push("dnd".into());
            self.own.lock().unwrap().state = State::Dnd;
            Ok(())
        }
        async fn set_custom(&self, c: Option<&NewCustom>) -> Result<()> {
            self.calls
                .lock()
                .unwrap()
                .push(format!("custom {:?}", c.map(|c| &c.text)));
            Ok(())
        }
    }

    #[derive(Default)]
    struct FakeBus {
        missing: bool,
        published: Mutex<Vec<PresenceMsg>>,
    }

    #[async_trait]
    impl PresenceBus for FakeBus {
        async fn ensure(&self, _: &str) -> Result<()> {
            if self.missing { Err(api_error(404)) } else { Ok(()) }
        }
        async fn publish(&self, data: Vec<u8>) -> Result<()> {
            self.published
                .lock()
                .unwrap()
                .push(serde_json::from_slice(&data).unwrap());
            Ok(())
        }
        async fn pull(&self, _: &str) -> Result<Vec<(String, Vec<u8>)>> {
            Ok(vec![])
        }
        async fn ack(&self, _: &str, _: &[String]) -> Result<()> {
            Ok(())
        }
        async fn delete(&self, _: &str) -> Result<()> {
            Ok(())
        }
    }

    struct FakeIdle(AtomicU64);
    impl IdleSource for FakeIdle {
        fn idle_secs(&self) -> u64 {
            self.0.load(Ordering::Relaxed)
        }
    }

    #[derive(Default)]
    struct FakeRoster {
        kv: Mutex<HashMap<String, String>>,
    }
    impl Roster for FakeRoster {
        fn me(&self) -> Option<String> {
            Some(ME.into())
        }
        fn device_id(&self) -> Option<String> {
            Some("d1".into())
        }
        fn knows(&self, user: &str) -> bool {
            user == SAM
        }
        fn get(&self, key: &str) -> Option<String> {
            self.kv.lock().unwrap().get(key).cloned()
        }
        fn set(&self, key: &str, value: &str) {
            self.kv.lock().unwrap().insert(key.into(), value.into());
        }
    }

    #[derive(Default)]
    struct RecSink {
        presence: Mutex<Vec<PresenceView>>,
        own: Mutex<Vec<OwnPresenceView>>,
    }
    impl Sink for RecSink {
        fn spaces_changed(&self, _: Vec<SpaceSummary>) {}
        fn messages_upserted(&self, _: &str, _: Vec<MessageView>) {}
        fn messages_deleted(&self, _: &str, _: Vec<String>) {}
        fn status(&self, _: SyncStatus) {}
        fn auth(&self, _: AuthState) {}
        fn notify(&self, _: Notification) {}
        fn badge(&self, _: u32) {}
        fn presence(&self, changed: Vec<PresenceView>) {
            self.presence.lock().unwrap().extend(changed);
        }
        fn own_presence(&self, view: OwnPresenceView) {
            self.own.lock().unwrap().push(view);
        }
    }

    struct World {
        p: Arc<Presence>,
        api: Arc<FakeApi>,
        bus: Arc<FakeBus>,
        roster: Arc<FakeRoster>,
        sink: Arc<RecSink>,
    }

    fn world(google: State, idle_secs: u64, bus: FakeBus) -> World {
        let api = Arc::new(FakeApi {
            own: Mutex::new(Own {
                state: google,
                dnd_until: None,
                custom: None,
            }),
            fail: Mutex::new(None),
            calls: Mutex::new(vec![]),
        });
        let bus = Arc::new(bus);
        let roster = Arc::new(FakeRoster::default());
        let sink = Arc::new(RecSink::default());
        let p = Presence::new(
            api.clone(),
            bus.clone(),
            Arc::new(FakeIdle(AtomicU64::new(idle_secs))),
            roster.clone(),
            sink.clone(),
        );
        World {
            p,
            api,
            bus,
            roster,
            sink,
        }
    }

    #[tokio::test]
    async fn here_keeps_google_active_and_tells_colleagues() {
        let w = world(State::Idle, 10, FakeBus::default());
        assert!(w.p.join_team("d1", NOW).await);
        w.p.tick(NOW).await;
        assert_eq!(*w.api.calls.lock().unwrap(), ["active"]);
        let published = w.bus.published.lock().unwrap();
        assert_eq!((published.len(), published[0].state.as_str()), (1, "ACTIVE"));
        let own = w.p.own();
        assert_eq!(
            (own.access.as_str(), own.state.as_deref(), own.team),
            ("ok", Some("active"), true)
        );
    }

    #[tokio::test]
    async fn do_not_disturb_is_left_alone_and_heartbeats_follow() {
        let w = world(State::Dnd, 10, FakeBus::default());
        w.p.join_team("d1", NOW).await;
        w.p.tick(NOW).await;
        w.p.tick(NOW + 30 * SECOND).await;
        assert!(w.api.calls.lock().unwrap().is_empty());
        assert_eq!(w.bus.published.lock().unwrap().len(), 1, "nothing new after 30 s");
        w.p.tick(NOW + 61 * SECOND).await;
        assert_eq!(w.bus.published.lock().unwrap().len(), 2, "a heartbeat after a minute");
    }

    #[tokio::test]
    async fn picking_away_holds_and_is_remembered() {
        let w = world(State::Active, 10, FakeBus::default());
        w.p.set_status(None, Choice::Away).await.unwrap();
        assert_eq!(*w.api.calls.lock().unwrap(), ["custom None", "away"]);
        assert_eq!(w.roster.get(KV_MANUAL_AWAY).as_deref(), Some("1"));
        assert!(w.p.own().manual_away);
        // Still at the keyboard an hour later: kwek doesn't mark active.
        w.p.tick(NOW + 60 * MINUTE).await;
        assert_eq!(w.api.calls.lock().unwrap().len(), 2);
    }

    #[tokio::test]
    async fn colleagues_are_shown_strangers_are_not() {
        let w = world(State::Active, 10, FakeBus::default());
        let sam = msg(SAM, "AWAY", NOW);
        let stranger = msg("users/stranger", "ACTIVE", NOW);
        w.p.receive([sam.as_slice(), stranger.as_slice()].into_iter(), NOW);
        {
            let shown = w.sink.presence.lock().unwrap();
            assert_eq!(shown.len(), 1);
            assert_eq!((shown[0].user.as_str(), shown[0].state.as_deref()), (SAM, Some("away")));
        }
        // An older message arriving late doesn't undo the newer one.
        let late = msg(SAM, "ACTIVE", NOW - SECOND);
        w.p.receive([late.as_slice()].into_iter(), NOW);
        assert_eq!(w.p.all()[0].state.as_deref(), Some("away"));
    }

    #[tokio::test]
    async fn a_missing_scope_asks_for_permission() {
        let w = world(State::Active, 10, FakeBus::default());
        *w.api.fail.lock().unwrap() = Some(403);
        w.p.tick(NOW).await;
        assert_eq!(w.p.own().access, "needsPermission");
        assert_eq!(w.sink.own.lock().unwrap().last().unwrap().access, "needsPermission");
    }

    #[tokio::test]
    async fn without_the_topic_nothing_is_published() {
        let w = world(
            State::Active,
            10,
            FakeBus {
                missing: true,
                ..Default::default()
            },
        );
        assert!(!w.p.join_team("d1", NOW).await);
        w.p.tick(NOW).await;
        assert!(w.bus.published.lock().unwrap().is_empty());
        assert!(!w.p.own().team);
        assert_eq!(w.p.own().access, "ok", "your own status still works");
    }

    #[test]
    fn reads_googles_availability() {
        let own = parse_own(&serde_json::json!({
            "name": "users/me/availability", "state": "DO_NOT_DISTURB",
            "customStatus": { "text": "Focus", "emoji": { "unicode": "🎧" }, "expireTime": "2027-01-15T09:00:00Z" },
            "doNotDisturbMetadata": { "expirationTime": "2027-01-15T10:00:00Z" }
        }));
        assert_eq!(own.state, State::Dnd);
        assert_eq!(own.dnd_until, ts::parse("2027-01-15T10:00:00Z"));
        let c = own.custom.unwrap();
        assert_eq!((c.emoji.as_str(), c.text.as_str()), ("🎧", "Focus"));
    }

    #[test]
    fn own_message_round_trips() {
        let own = Own {
            state: State::Away,
            dnd_until: None,
            custom: Some(CustomStatusView {
                emoji: "🌴".into(),
                text: "Back Monday".into(),
                until_us: Some(NOW + 60 * MINUTE),
            }),
        };
        let data = serde_json::to_vec(&PresenceMsg::own(SAM, &own, NOW)).unwrap();
        let h = accept(&data, knows, ME, NOW, None).unwrap();
        assert_eq!((h.state, h.custom), (State::Away, own.custom));
    }
}
