//! The sync engine: the only component that talks to Google. Events are hints, REST is
//! truth: every write is an idempotent upsert, and a reconcile pass closes any gap.

pub mod apply;

use std::collections::{BTreeSet, HashMap};
use std::sync::{Arc, Mutex as StdMutex};
use std::time::{Duration, Instant, SystemTime};

use futures::StreamExt;
use rusqlite::Connection;
use tokio::sync::{Mutex, Notify};

use crate::auth::TokenManager;
use crate::chat::{ChatApi, Emoji, EmojiReactionSummary, ListMessages, older_than_filter, thread_filter};
use crate::db::read;
use crate::db::{Db, kv_get, kv_set, write};
use crate::error::{Error, Result};
use crate::events::{
    Event, EventsClient, SPACES_EVENT_TYPES, SPACES_TARGET, USER_EVENT_TYPES, decode_space_event, user_target,
};
use crate::people::PeopleApi;
use crate::render;
use crate::search::{self, embed::Embedder};
use crate::ts::{self, DAY, MINUTE, Micros};
use crate::views::{self, AuthState, Builder, MeaningStatus, MessageView, SearchResult, SpaceSummary, SyncStatus};

pub use apply::{Applied, apply};

const INITIAL_PAGE: u32 = 200;
const OLDER_PAGE: u32 = 100;
/// Background history download: big pages, a pause between them, a recheck now and then
/// for spaces that appeared since.
const BACKFILL_PAGE: u32 = 500;
const BACKFILL_PAUSE: Duration = Duration::from_millis(300);
const BACKFILL_RECHECK: Duration = Duration::from_secs(30 * 60);
/// Stretches fetched per indexing round; each is embedded on its own.
const EMBED_FETCH: u32 = 32;
const RENEW_EVERY: Duration = Duration::from_secs(3 * 3600);
const RECONCILE_EVERY: Duration = Duration::from_secs(15 * 60);
const POLL_EVERY: Duration = Duration::from_secs(60);
const RETRY_LIVE_EVERY: Duration = Duration::from_secs(10 * 60);
/// `spaceEvents.list` only reaches back 28 days; stay clear of the edge.
const EVENT_RETENTION: Micros = 27 * DAY;

#[derive(Debug, Clone)]
pub struct Notification {
    pub title: String,
    pub body: String,
    pub space: String,
    pub message: String,
    pub thread: Option<String>,
}

/// Where the engine pushes changes, always after the DB commit.
pub trait Sink: Send + Sync {
    fn spaces_changed(&self, spaces: Vec<SpaceSummary>);
    fn messages_upserted(&self, space: &str, messages: Vec<MessageView>);
    fn messages_deleted(&self, space: &str, names: Vec<String>);
    fn status(&self, status: SyncStatus);
    fn auth(&self, state: AuthState);
    fn notify(&self, n: Notification);
    fn badge(&self, count: u32);
}

#[derive(Default)]
struct Focus {
    space: Option<String>,
    thread: Option<String>,
    window_focused: bool,
}

pub struct Engine {
    pub db: Db,
    chat: Arc<dyn ChatApi>,
    people: Arc<dyn PeopleApi>,
    events: Option<EventsClient>,
    tokens: Option<Arc<TokenManager>>,
    sink: Arc<dyn Sink>,
    me: StdMutex<String>,
    focus: StdMutex<Focus>,
    status: StdMutex<SyncStatus>,
    sync_lock: Mutex<()>,
    wake: Notify,
    read_marks: StdMutex<HashMap<String, Micros>>,
    /// Names of our Workspace Events subscriptions; events from others are dropped.
    own_subs: StdMutex<Vec<String>>,
    /// Downloaded attachments; deleted on logout.
    media_dir: Option<std::path::PathBuf>,
    /// Search by meaning; None when the model is not part of this build.
    embedder: Option<Arc<dyn Embedder>>,
    /// Stretch vectors by text hash, loaded on first use. Lock order: db, then this.
    vectors: StdMutex<Option<HashMap<String, Vec<f32>>>>,
    /// Messages changed: stretches and vectors need an update.
    index_wake: Notify,
}

impl Engine {
    #[allow(clippy::too_many_arguments)] // one per collaborator; a builder would only add ceremony
    pub fn new(
        db: Db,
        chat: Arc<dyn ChatApi>,
        people: Arc<dyn PeopleApi>,
        events: Option<EventsClient>,
        tokens: Option<Arc<TokenManager>>,
        sink: Arc<dyn Sink>,
        media_dir: Option<std::path::PathBuf>,
        embedder: Option<Arc<dyn Embedder>>,
    ) -> Arc<Self> {
        let me = db
            .with(|c| read::account(c))
            .ok()
            .flatten()
            .map(|a| a.id)
            .unwrap_or_default();
        // A send in flight when the app quit was never confirmed: offer Retry instead of
        // leaving it "sending" forever.
        if let Err(e) = db.with(|c| write::fail_stale_pending(c)) {
            tracing::warn!("could not reset pending sends: {e}");
        }
        Arc::new(Self {
            db,
            chat,
            people,
            events,
            tokens,
            sink,
            me: StdMutex::new(me),
            focus: StdMutex::new(Focus::default()),
            status: StdMutex::new(SyncStatus {
                state: "starting".into(),
                detail: None,
            }),
            sync_lock: Mutex::new(()),
            wake: Notify::new(),
            read_marks: StdMutex::new(HashMap::new()),
            own_subs: StdMutex::new(Vec::new()),
            media_dir,
            embedder,
            vectors: StdMutex::new(None),
            index_wake: Notify::new(),
        })
    }

    pub fn me(&self) -> String {
        self.me.lock().unwrap().clone()
    }

    fn set_status(&self, state: &str, detail: Option<String>) {
        let s = SyncStatus {
            state: state.into(),
            detail,
        };
        let mut cur = self.status.lock().unwrap();
        if *cur != s {
            *cur = s.clone();
            drop(cur);
            self.sink.status(s);
        }
    }

    pub fn sync_status(&self) -> SyncStatus {
        self.status.lock().unwrap().clone()
    }

    // ---------------------------------------------------------------- auth

    pub fn auth_state(&self) -> AuthState {
        let signed_in = self.tokens.as_ref().is_some_and(|t| t.has_refresh_token());
        match (signed_in, self.db.with(|c| read::account(c)).ok().flatten()) {
            (true, Some(a)) => AuthState::SignedIn {
                email: a.email,
                name: a.display_name,
                user: a.id,
            },
            (true, None) => AuthState::SigningIn,
            _ => AuthState::SignedOut,
        }
    }

    pub async fn login(self: &Arc<Self>, open: impl FnOnce(&str) -> Result<()>) -> Result<()> {
        let tokens = self.tokens.clone().ok_or(Error::NoOAuthClient)?;
        self.sink.auth(AuthState::SigningIn);
        if let Err(e) = tokens.login(open).await {
            self.sink.auth(self.auth_state());
            return Err(e);
        }
        self.ensure_identity().await?;
        self.sink.auth(self.auth_state());
        self.wake.notify_one();
        Ok(())
    }

    pub async fn logout(&self) -> Result<()> {
        let _g = self.sync_lock.lock().await;
        if let (Some(ev), Some(device)) = (&self.events, self.db.with(|c| kv_get(c, "device_id"))?)
            && let Err(e) = ev.delete_pubsub(&device).await
        {
            tracing::warn!("could not delete device subscription: {e}");
        }
        if let Some(t) = &self.tokens {
            t.logout().await?;
        }
        self.db.wipe()?;
        self.clear_media();
        *self.vectors.lock().unwrap() = None;
        *self.me.lock().unwrap() = String::new();
        self.own_subs.lock().unwrap().clear();
        self.sink.spaces_changed(vec![]);
        self.sink.auth(AuthState::SignedOut);
        self.sink.badge(0);
        self.set_status("authRequired", None);
        Ok(())
    }

    fn clear_media(&self) {
        if let Some(dir) = &self.media_dir
            && let Err(e) = std::fs::remove_dir_all(dir)
            && e.kind() != std::io::ErrorKind::NotFound
        {
            tracing::warn!("could not clear media cache: {e}");
        }
    }

    async fn ensure_identity(&self) -> Result<String> {
        let p = self.people.me().await?;
        let id = p.user.clone();
        // Signing in as someone else: nothing of the previous account may stay (reactions,
        // read state and mention flags are per user).
        let previous = self.db.call(|c| read::account(c)).await?.map(|a| a.id);
        if previous.as_deref().is_some_and(|prev| prev != id) {
            tracing::info!("different account signed in; wiping the local cache");
            let db = self.db.clone();
            tokio::task::spawn_blocking(move || db.wipe())
                .await
                .map_err(|e| Error::Other(e.to_string()))??;
            self.clear_media();
            *self.vectors.lock().unwrap() = None;
        }
        self.db
            .call(move |c| {
                write::set_account(c, &p.user, p.email.as_deref(), p.display_name.as_deref())?;
                write::upsert_person(c, &p, ts::now())
            })
            .await?;
        *self.me.lock().unwrap() = id.clone();
        Ok(id)
    }

    // ---------------------------------------------------------------- main loop

    /// Runs forever: sign-in gate, bootstrap or reconcile, then the live loop.
    pub async fn run(self: Arc<Self>) {
        let mut backoff = Duration::from_secs(2);
        loop {
            let signed_in = self.tokens.as_ref().is_some_and(|t| t.has_refresh_token());
            if !signed_in {
                self.set_status("authRequired", None);
                self.sink.auth(AuthState::SignedOut);
                self.wake.notified().await;
                continue;
            }
            match self.session().await {
                Ok(()) => {
                    // A session ends Ok to restart live (subscription recreated, polling
                    // recovered). Never spin.
                    backoff = Duration::from_secs(2);
                    tokio::time::sleep(Duration::from_secs(2)).await;
                }
                Err(Error::AuthRequired) => {
                    self.sink.auth(AuthState::SignedOut);
                    self.set_status("authRequired", None);
                    continue;
                }
                Err(e) => {
                    let offline = e.is_offline();
                    tracing::warn!("sync session ended: {e}");
                    self.set_status(
                        if offline { "offline" } else { "error" },
                        Some(if offline { "No connection".into() } else { short(&e) }),
                    );
                    tokio::select! {
                        _ = tokio::time::sleep(backoff) => {}
                        _ = self.wake.notified() => {}
                    }
                    backoff = (backoff * 2).min(Duration::from_secs(120));
                }
            }
        }
    }

    async fn session(self: &Arc<Self>) -> Result<()> {
        if self.me().is_empty() {
            self.ensure_identity().await?;
        }
        let bootstrapped = self.db.call(|c| kv_get(c, "bootstrapped")).await?.is_some();
        if !bootstrapped {
            self.bootstrap().await?;
        }
        // live() sets up the subscriptions first and then reconciles, so nothing that
        // happens in between is missed.
        self.live().await
    }

    /// Ask the loop to reconcile now (wake from sleep, network back, manual refresh).
    pub fn poke(&self) {
        self.wake.notify_one();
    }

    // ---------------------------------------------------------------- bootstrap / reconcile

    pub async fn bootstrap(self: &Arc<Self>) -> Result<()> {
        let _g = self.sync_lock.lock().await;
        self.set_status("bootstrapping", Some("Loading spaces".into()));
        let spaces = self.refresh_space_list().await?;
        let total = spaces.len();
        let me = self.me();
        let mut done = 0usize;
        let mut stream = futures::stream::iter(spaces.into_iter().map(|s| {
            let this = self.clone();
            let me = me.clone();
            async move { this.initial_fetch(&s, &me, false).await.map(|_| s) }
        }))
        .buffer_unordered(4);
        while let Some(r) = stream.next().await {
            done += 1;
            match r {
                Ok(_) => {}
                Err(e @ Error::AuthRequired) => return Err(e),
                Err(e) => tracing::warn!("initial fetch failed: {e}"),
            }
            self.set_status("bootstrapping", Some(format!("Synced {done} of {total} conversations")));
            self.emit_spaces();
        }
        drop(stream);
        self.resolve_users().await?;
        self.db.with(|c| kv_set(c, "bootstrapped", "1"))?;
        self.emit_spaces();
        Ok(())
    }

    /// Lists all spaces, upserts them, drops local spaces the user left. Returns names,
    /// most recently active first.
    async fn refresh_space_list(&self) -> Result<Vec<String>> {
        let mut all = Vec::new();
        let mut token: Option<String> = None;
        loop {
            let page = self.chat.list_spaces(token.as_deref()).await?;
            token = page.next().map(str::to_owned);
            all.extend(page.items);
            if token.is_none() {
                break;
            }
        }
        all.retain(|s| s.single_user_bot_dm != Some(true));
        all.sort_by_key(|s| std::cmp::Reverse(s.last_active_time.as_deref().and_then(ts::parse).unwrap_or(0)));
        let names: Vec<String> = all.iter().map(|s| s.name.clone()).collect();
        let keep: BTreeSet<String> = names.iter().cloned().collect();
        self.db
            .call(move |c| {
                let tx = c.transaction()?;
                for s in &all {
                    write::upsert_space(&tx, s)?;
                }
                for local in read::space_names(&tx)? {
                    if !keep.contains(&local) {
                        write::delete_space(&tx, &local)?;
                    }
                }
                tx.commit()?;
                Ok(())
            })
            .await?;
        Ok(names)
    }

    /// Latest messages, members and read state of one space. Resets the contiguous-history
    /// mark to the oldest message of this page: older cache may have a gap behind it.
    async fn initial_fetch(&self, space: &str, me: &str, notify: bool) -> Result<()> {
        let started = ts::now();
        let page = self
            .chat
            .list_messages(
                space,
                &ListMessages {
                    page_size: INITIAL_PAGE,
                    newest_first: true,
                    ..Default::default()
                },
            )
            .await?;
        let complete = page.next().is_none();
        let low_water = page
            .items
            .iter()
            .filter_map(|m| m.create_time.as_deref().and_then(ts::parse))
            .min();
        let read_state = self.chat.get_space_read_state(space).await.ok();
        // On failure keep the members we have rather than wiping them.
        let members = match self.all_members(space).await {
            Ok(m) => Some(m),
            Err(e) => {
                tracing::debug!(%space, "members not refreshed: {e}");
                None
            }
        };
        let mut events: Vec<Event> = page.items.into_iter().map(Event::MessageUpserted).collect();
        if let Some(rs) = read_state {
            events.push(Event::SpaceReadState(rs));
        }
        let (space_owned, me_owned) = (space.to_owned(), me.to_owned());
        let applied = self
            .db
            .call(move |c| {
                let tx = c.transaction()?;
                // Measured before applying: does this page overlap what is cached?
                let newest_cached: Option<Micros> = tx.query_row(
                    "SELECT max(create_us) FROM message WHERE space = ?1 AND local_state = 'synced' AND create_us > 0",
                    [&space_owned],
                    |r| r.get(0),
                )?;
                let (old_low, old_complete) = read::history_bounds(&tx, &space_owned)?;
                let a = apply(&tx, &events, &me_owned)?;
                if let Some(members) = &members {
                    write::clear_memberships(&tx, &space_owned)?;
                    let mut peer = None;
                    for m in members {
                        if let Some(u) = &m.member {
                            write::upsert_membership(&tx, &space_owned, u, m.role.as_deref())?;
                            if u.name != me_owned && u.kind.as_deref() != Some("BOT") {
                                peer.get_or_insert(u.name.clone());
                            }
                        }
                    }
                    if let Some(p) = peer {
                        write::set_dm_peer(&tx, &space_owned, &p)?;
                    }
                }
                // Overlap with the cache keeps the contiguous history already earned (this
                // runs every minute while polling); only a real gap resets the mark.
                let page_low = low_water.unwrap_or(started);
                let overlaps = matches!((newest_cached, low_water), (Some(n), Some(l)) if n >= l) || complete;
                let (low, done) = match old_low {
                    Some(prev) if overlaps => (prev.min(page_low), old_complete || complete),
                    _ => (page_low, complete),
                };
                write::set_history_complete(&tx, &space_owned, done)?;
                write::set_low_water(&tx, &space_owned, low)?;
                write::set_space_synced(&tx, &space_owned, started)?;
                tx.commit()?;
                Ok(a)
            })
            .await?;
        self.after_apply(applied, notify).await;
        Ok(())
    }

    async fn all_members(&self, space: &str) -> Result<Vec<crate::chat::Membership>> {
        let mut out = Vec::new();
        let mut token: Option<String> = None;
        loop {
            let page = self.chat.list_members(space, token.as_deref()).await?;
            token = page.next().map(str::to_owned);
            out.extend(page.items);
            if token.is_none() || out.len() > 5000 {
                return Ok(out);
            }
        }
    }

    /// Closes gaps: new spaces, missed events, read state.
    ///
    /// `full` replays `spaceEvents` for every space. It is needed whenever live events may
    /// have been missed (startup, wake, a recreated subscription, polling): edits, deletes,
    /// reactions and read changes do not move a space's `lastActiveTime`, so a gated pass
    /// would never see them. The gated pass (only spaces with newer activity) serves the
    /// periodic safety net while live events are flowing.
    pub async fn reconcile(self: &Arc<Self>, full: bool) -> Result<()> {
        let _g = self.sync_lock.lock().await;
        self.set_status("reconciling", None);
        let started = ts::now();
        let me = self.me();
        self.refresh_space_list().await?;
        let rows = self
            .db
            .call({
                let me = me.clone();
                move |c| read::spaces(c, &me)
            })
            .await?;
        for s in rows {
            let changed = match (s.last_synced_us, s.last_active_us) {
                (None, _) => true,
                (Some(synced), Some(active)) => active > synced,
                (Some(_), None) => false,
            };
            let result = if !(full || changed) {
                if s.unread > 0 {
                    self.refresh_read_state(&s.name).await
                } else {
                    Ok(())
                }
            } else {
                match s.last_synced_us {
                    // Five minutes of overlap absorbs clock skew between this Mac and
                    // Google; replays are idempotent.
                    Some(since) if started - since < EVENT_RETENTION => {
                        match self.replay_events(&s.name, since - 5 * MINUTE, &me, started).await {
                            // 400: start time outside retention. 403/404: spaceEvents needs
                            // the Chat app too; until it is configured, refetch the latest
                            // page (it includes edits and deletes via showDeleted).
                            Err(e) if e.http_status() == Some(400) || is_config_error(&e) => {
                                self.initial_fetch(&s.name, &me, true).await
                            }
                            r => r,
                        }
                    }
                    _ => self.initial_fetch(&s.name, &me, true).await,
                }
            };
            match result {
                Err(e @ Error::AuthRequired) => return Err(e),
                Err(e) if e.is_offline() => return Err(e),
                Err(e) => tracing::warn!(space = %s.name, "reconcile failed: {e}"),
                Ok(()) => {}
            }
        }
        self.resolve_users().await?;
        self.emit_spaces();
        Ok(())
    }

    async fn replay_events(&self, space: &str, since: Micros, me: &str, started: Micros) -> Result<()> {
        let start = ts::format(since);
        let mut token: Option<String> = None;
        let mut events = Vec::new();
        loop {
            let page = self.chat.list_space_events(space, &start, token.as_deref()).await?;
            token = page.next().map(str::to_owned);
            for e in &page.items {
                events.extend(decode_space_event(e));
            }
            if token.is_none() {
                break;
            }
        }
        let read_state = self.chat.get_space_read_state(space).await.ok();
        if let Some(rs) = read_state {
            events.push(Event::SpaceReadState(rs));
        }
        let (space_owned, me_owned) = (space.to_owned(), me.to_owned());
        let applied = self
            .db
            .call(move |c| {
                let tx = c.transaction()?;
                let a = apply(&tx, &events, &me_owned)?;
                write::set_space_synced(&tx, &space_owned, started)?;
                tx.commit()?;
                Ok(a)
            })
            .await?;
        // Replayed messages notify too; the five-minute freshness rule keeps old ones quiet.
        self.after_apply(applied, true).await;
        Ok(())
    }

    /// Read state straight from REST. A missing time leaves the known mark alone.
    async fn refresh_read_state(&self, space: &str) -> Result<()> {
        let rs = self.chat.get_space_read_state(space).await?;
        let (Some(name), Some(t)) = (rs.space_name(), rs.last_read_time.as_deref().and_then(ts::parse)) else {
            return Ok(());
        };
        self.db.call(move |c| write::set_space_read(c, &name, Some(t))).await?;
        self.emit_spaces();
        Ok(())
    }

    /// Names for users seen only as IDs (DM peers, members).
    async fn resolve_users(&self) -> Result<()> {
        for _ in 0..10 {
            let pending = self.db.call(|c| read::unresolved_users(c, 200)).await?;
            if pending.is_empty() {
                return Ok(());
            }
            let found = match self.people.resolve(&pending).await {
                Ok(f) => f,
                Err(e @ Error::AuthRequired) => return Err(e),
                Err(e) => {
                    tracing::warn!("people lookup failed: {e}");
                    vec![]
                }
            };
            self.db
                .call(move |c| {
                    let tx = c.transaction()?;
                    let now = ts::now();
                    for p in &found {
                        write::upsert_person(&tx, p, now)?;
                    }
                    write::mark_resolved(&tx, &pending, now)?;
                    tx.commit()?;
                    Ok(())
                })
                .await?;
        }
        Ok(())
    }

    // ---------------------------------------------------------------- live

    async fn device_id(&self) -> Result<String> {
        self.db
            .call(|c| match kv_get(c, "device_id")? {
                Some(d) => Ok(d),
                None => {
                    let d = uuid::Uuid::new_v4().simple().to_string()[..12].to_owned();
                    kv_set(c, "device_id", &d)?;
                    Ok(d)
                }
            })
            .await
    }

    async fn ensure_subscriptions(&self) -> Result<String> {
        let ev = self
            .events
            .as_ref()
            .ok_or_else(|| Error::Other("no events client".into()))?;
        let device = self.device_id().await?;
        let sub = ev.ensure_pubsub(&device).await?;
        let spaces = ev.ensure_subscription(SPACES_TARGET, SPACES_EVENT_TYPES, true).await?;
        // Only a rejection of `includeResource` justifies the fallback; a transient error must
        // not leave us with a resource-less subscription for good.
        let user = match ev
            .ensure_subscription(&user_target(&self.me()), USER_EVENT_TYPES, true)
            .await
        {
            Ok(s) => s,
            Err(e) if e.is_status("INVALID_ARGUMENT") => {
                tracing::info!("user subscription with resource refused ({e}); retrying without");
                ev.ensure_subscription(&user_target(&self.me()), USER_EVENT_TYPES, false)
                    .await?
            }
            Err(e) => return Err(e),
        };
        *self.own_subs.lock().unwrap() = vec![spaces.name, user.name];
        Ok(sub)
    }

    /// The topic is shared by every kwek user in the project, so a device subscription sees
    /// everyone's events. Only events from our own Events subscriptions are applied.
    fn is_ours(&self, r: &crate::events::Received) -> bool {
        let own = self.own_subs.lock().unwrap();
        let source = r.message.attributes.get("ce-source").map(String::as_str).unwrap_or("");
        let ours = !source.is_empty() && own.iter().any(|s| !s.is_empty() && source.ends_with(s.as_str()));
        if !ours {
            tracing::debug!(%source, "dropping event from a subscription that is not ours");
        }
        ours
    }

    async fn live(self: &Arc<Self>) -> Result<()> {
        let Some(ev) = &self.events else {
            self.set_status("live", None);
            self.wake.notified().await;
            return Ok(());
        };
        let sub = match self.ensure_subscriptions().await {
            Ok(s) => s,
            // Polling is for configuration problems that will not fix themselves soon (no
            // Chat app, no permission). Anything else ends the session and retries.
            Err(e) if is_config_error(&e) => return self.poll(e).await,
            Err(e) => return Err(e),
        };
        // Subscriptions first, then catch up: nothing falls between the two.
        self.reconcile(true).await?;
        self.set_status("live", None);
        let mut renewed = Instant::now();
        let mut reconciled = Instant::now();
        loop {
            let wall_before = SystemTime::now();
            let mono_before = Instant::now();
            let batch = tokio::select! {
                r = ev.pull(&sub, 100) => r,
                _ = self.wake.notified() => {
                    self.reconcile(true).await?;
                    self.set_status("live", None);
                    continue;
                }
            };
            // Wall clock jumping far past monotonic time means the machine slept.
            let wall = SystemTime::now().duration_since(wall_before).unwrap_or_default();
            let slept = wall > mono_before.elapsed() + Duration::from_secs(60);
            let batch = match batch {
                Ok(b) => b,
                Err(e) if e.is_status("NOT_FOUND") => {
                    tracing::warn!("device subscription vanished; recreating");
                    return Ok(());
                }
                Err(e) => return Err(e),
            };
            if !batch.is_empty() {
                let mut events = Vec::new();
                let mut lifecycle = Vec::new();
                for r in batch.iter().filter(|r| self.is_ours(r)) {
                    for e in r.decode() {
                        match e {
                            Event::Lifecycle { kind, subscription } => lifecycle.push((kind, subscription)),
                            Event::Unknown(t) => tracing::debug!("ignoring event {t}"),
                            e => events.push(e),
                        }
                    }
                }
                self.adopt_unknown_spaces(&events).await?;
                let me = self.me();
                let applied = self
                    .db
                    .call(move |c| {
                        let tx = c.transaction()?;
                        let a = apply(&tx, &events, &me)?;
                        tx.commit()?;
                        Ok(a)
                    })
                    .await?;
                // Ack only after the commit.
                let acks: Vec<String> = batch.iter().map(|r| r.ack_id.clone()).collect();
                ev.ack(&sub, &acks).await?;
                self.after_apply(applied, true).await;
                for (kind, name) in lifecycle {
                    tracing::info!(%kind, %name, "subscription lifecycle event");
                    match kind.as_str() {
                        "expirationReminder" => {
                            ev.renew(&name).await.ok();
                        }
                        "expired" | "suspended" => {
                            self.ensure_subscriptions().await?;
                            self.reconcile(true).await?;
                            self.set_status("live", None);
                        }
                        _ => {}
                    }
                }
            }
            if slept || reconciled.elapsed() > RECONCILE_EVERY {
                if slept {
                    tracing::info!("wake from sleep detected");
                    self.ensure_subscriptions().await?;
                }
                self.reconcile(slept).await?;
                self.set_status("live", None);
                reconciled = Instant::now();
            }
            if renewed.elapsed() > RENEW_EVERY {
                self.ensure_subscriptions().await?;
                renewed = Instant::now();
            }
        }
    }

    /// Fallback when live events cannot be set up (for example, the Chat app is not
    /// configured): reconcile every minute and retry the subscriptions every ten. Returning
    /// Ok ends the session, so the next one starts live.
    async fn poll(self: &Arc<Self>, cause: Error) -> Result<()> {
        tracing::warn!("live events unavailable, polling instead: {cause}");
        let mut last_try = Instant::now();
        loop {
            // Polling is the only source of changes here, so every pass is a full one.
            self.reconcile(true).await?;
            self.set_status("polling", Some(short(&cause)));
            tokio::select! {
                _ = tokio::time::sleep(POLL_EVERY) => {}
                _ = self.wake.notified() => {}
            }
            if last_try.elapsed() > RETRY_LIVE_EVERY {
                last_try = Instant::now();
                match self.ensure_subscriptions().await {
                    Ok(_) => return Ok(()),
                    Err(e) if is_config_error(&e) => tracing::debug!("live events still unavailable: {e}"),
                    Err(e) => return Err(e),
                }
            }
        }
    }

    /// An event that passed the subscription filter but names a space we do not know yet:
    /// usually the first message in a new DM, which can arrive before (or without) the
    /// membership event. Fetching the space succeeds only if the user is a member, so this
    /// keeps the security property of the applier's foreign-space filter.
    async fn adopt_unknown_spaces(&self, events: &[Event]) -> Result<()> {
        let mut spaces: Vec<String> = events.iter().filter_map(apply::event_space).collect();
        spaces.sort();
        spaces.dedup();
        for space in spaces {
            let s2 = space.clone();
            if self.db.call(move |c| read::space_exists(c, &s2)).await? {
                continue;
            }
            match self.chat.get_space(&space).await {
                // Bot DMs are filtered out of the space list; adopting them would make them
                // flap in and out on every event.
                Ok(s) if s.single_user_bot_dm == Some(true) => {}
                Ok(s) => {
                    self.db.call(move |c| write::upsert_space(c, &s)).await?;
                    let me = self.me();
                    // Members and read state; the triggering event itself is applied next.
                    if let Err(e) = Box::pin(self.initial_fetch(&space, &me, true)).await {
                        tracing::warn!(%space, "new space fetch failed: {e}");
                    }
                }
                Err(e) => tracing::debug!(%space, "event for an inaccessible space dropped: {e}"),
            }
        }
        Ok(())
    }

    /// Follow-ups after a commit: reaction and read-state refetches, new spaces, UI events,
    /// notifications (`notify`: false only during bootstrap).
    async fn after_apply(&self, applied: Applied, notify: bool) {
        if applied.is_empty() {
            return;
        }
        let mut applied = applied;
        // Reaction events do not carry the new summary: refetch the messages (REST is truth).
        for name in std::mem::take(&mut applied.stale_reactions) {
            match self.chat.get_message(&name).await {
                Ok(m) => {
                    let me = self.me();
                    if let Ok(a) = self.db.call(move |c| apply(c, &[Event::MessageUpserted(m)], &me)).await {
                        applied.upserted.extend(a.upserted);
                    }
                }
                Err(e) => tracing::debug!("reaction refetch failed: {e}"),
            }
        }
        for space in std::mem::take(&mut applied.stale_read) {
            if let Err(e) = self.refresh_read_state(&space).await {
                tracing::debug!(%space, "read state refetch failed: {e}");
            }
        }
        for space in std::mem::take(&mut applied.joined_spaces) {
            if let Ok(s) = self.chat.get_space(&space).await {
                let me = self.me();
                let _ = self.db.call(move |c| write::upsert_space(c, &s)).await;
                // Boxed: initial_fetch calls back into after_apply.
                let _ = Box::pin(self.initial_fetch(&space, &me, notify)).await;
            }
        }

        let mut by_space: HashMap<String, Vec<String>> = HashMap::new();
        for (space, name) in &applied.upserted {
            by_space.entry(space.clone()).or_default().push(name.clone());
        }
        for (space, names) in by_space {
            if let Ok(views) = self.message_views_by_name(&names) {
                self.sink.messages_upserted(&space, views);
            }
        }
        let mut deleted: HashMap<String, Vec<String>> = HashMap::new();
        for (space, name) in &applied.deleted {
            deleted.entry(space.clone()).or_default().push(name.clone());
        }
        for (space, names) in deleted {
            self.sink.messages_deleted(&space, names);
        }
        if notify {
            self.notify_new(&applied.inserted);
        }
        self.emit_spaces();
        self.index_wake.notify_one();
    }

    fn notify_new(&self, inserted: &[String]) {
        if inserted.is_empty() {
            return;
        }
        let me = self.me();
        let (focused_space, window_focused) = {
            let f = self.focus.lock().unwrap();
            (f.space.clone(), f.window_focused)
        };
        let names = inserted.to_vec();
        let Ok((rows, spaces, users)) = self.db.with(|c| {
            let rows = read::messages_by_name(c, &names)?;
            let spaces = read::spaces(c, &me)?;
            let senders: Vec<String> = rows.iter().map(|r| r.sender.clone()).collect();
            Ok((rows, spaces, read::users(c, &senders)?))
        }) else {
            return;
        };
        let now = ts::now();
        let b = Builder { me: &me, users: &users };
        let mut out: Vec<Notification> = Vec::new();
        for r in rows {
            let Some(space) = spaces.iter().find(|s| s.name == r.space) else {
                continue;
            };
            let is_dm = space.space_type == "DIRECT_MESSAGE";
            let fresh = now - r.create_us < 5 * MINUTE;
            let open_here = window_focused && focused_space.as_deref() == Some(r.space.as_str());
            if r.sender == me || space.muted || !fresh || open_here || r.delete_us.is_some() {
                continue;
            }
            if !(is_dm || r.mentions_me) {
                continue;
            }
            let view = b.message(&r, None, &[], None);
            let sender = view.sender.display_name.clone();
            let title = if is_dm {
                sender.clone()
            } else {
                let summary = b.space(space, &[], None);
                format!("{sender} in {}", summary.display_name)
            };
            let mut body = render::plain(&view.blocks);
            if body.is_empty() && !view.attachments.is_empty() {
                body = "Sent an attachment".into();
            }
            if body.chars().count() > 180 {
                body = body.chars().take(177).collect::<String>() + "…";
            }
            out.push(Notification {
                title,
                body,
                space: r.space.clone(),
                message: r.name.clone(),
                thread: r.thread.clone(),
            });
        }
        // A burst (catching up after sleep) becomes one notification for the newest.
        if out.len() > 3 {
            let n = out.len();
            let latest = out.pop().expect("non-empty");
            out = vec![Notification {
                title: format!("{n} new messages"),
                body: format!("{}: {}", latest.title, latest.body),
                ..latest
            }];
        }
        for n in out {
            self.sink.notify(n);
        }
    }

    fn emit_spaces(&self) {
        if let Ok(spaces) = self.list_spaces() {
            let badge: u32 = spaces
                .iter()
                .filter(|s| !s.muted)
                .map(|s| if s.kind == "dm" { s.unread } else { s.mentions })
                .sum();
            self.sink.badge(badge);
            self.sink.spaces_changed(spaces);
        }
    }

    // ---------------------------------------------------------------- reads (UI)

    pub fn list_spaces(&self) -> Result<Vec<SpaceSummary>> {
        let me = self.me();
        self.db.with(|c| {
            let rows = read::spaces(c, &me)?;
            let users = read::all_users(c)?;
            let b = Builder { me: &me, users: &users };
            let mut out = Vec::with_capacity(rows.len());
            for s in &rows {
                let members = if s.display_name.is_none() && s.space_type != "DIRECT_MESSAGE" {
                    read::member_names(c, &s.name)?
                } else {
                    vec![]
                };
                out.push(b.space(s, &members, s.space_uri.clone()));
            }
            out.sort_by_key(|s| std::cmp::Reverse(s.last_active_us));
            Ok(out)
        })
    }

    fn build_messages(&self, c: &Connection, rows: Vec<read::MessageRow>) -> Result<Vec<MessageView>> {
        let me = self.me();
        let mut user_names: Vec<String> = rows.iter().map(|r| r.sender.clone()).collect();
        let threads: Vec<String> = rows
            .iter()
            .filter(|r| !r.is_reply)
            .filter_map(|r| r.thread.clone())
            .collect();
        let stats = read::thread_stats(c, &threads, &me)?;
        for s in stats.values() {
            user_names.extend(s.participants.iter().cloned());
        }
        // Mentions can name anyone; the user table is small, so load it whole.
        let users = read::all_users(c)?;
        let names: Vec<String> = rows.iter().map(|r| r.name.clone()).collect();
        let mine = read::my_reactions(c, &names)?;
        let mut uris: HashMap<String, Option<String>> = HashMap::new();
        let b = Builder { me: &me, users: &users };
        let mut out = Vec::with_capacity(rows.len());
        for r in &rows {
            let uri = uris
                .entry(r.space.clone())
                .or_insert_with(|| {
                    c.query_row("SELECT space_uri FROM space WHERE name = ?1", [&r.space], |x| x.get(0))
                        .ok()
                        .flatten()
                })
                .clone();
            let st = r.thread.as_ref().and_then(|t| stats.get(t));
            let my = mine.get(&r.name).map(Vec::as_slice).unwrap_or(&[]);
            out.push(b.message(r, st, my, uri.as_deref()));
        }
        Ok(out)
    }

    fn message_views_by_name(&self, names: &[String]) -> Result<Vec<MessageView>> {
        self.db.with(|c| {
            let rows = read::messages_by_name(c, names)?;
            // Replies changed: their roots' summaries changed too.
            let mut roots: Vec<String> = Vec::new();
            for r in rows.iter().filter(|r| r.is_reply) {
                if let Some(t) = &r.thread
                    && let Some(root) = read::thread(c, t)?.into_iter().find(|m| !m.is_reply)
                    && !names.contains(&root.name)
                    && !roots.contains(&root.name)
                {
                    roots.push(root.name);
                }
            }
            let mut rows = rows;
            rows.extend(read::messages_by_name(c, &roots)?);
            self.build_messages(c, rows)
        })
    }

    pub fn get_messages(&self, space: &str, before_us: Option<Micros>, limit: u32) -> Result<Vec<MessageView>> {
        self.db.with(|c| {
            let rows = read::top_level(c, space, before_us, limit.clamp(1, 500))?;
            self.build_messages(c, rows)
        })
    }

    pub fn get_thread_cached(&self, thread: &str) -> Result<Vec<MessageView>> {
        self.db.with(|c| {
            let rows = read::thread(c, thread)?;
            self.build_messages(c, rows)
        })
    }

    /// Fetches all replies of a thread from the API into the cache, then emits them.
    pub async fn fetch_thread(&self, thread: &str) -> Result<()> {
        let space = crate::chat::space_of(thread);
        let mut events = Vec::new();
        let mut token: Option<String> = None;
        loop {
            let page = self
                .chat
                .list_messages(
                    &space,
                    &ListMessages {
                        page_size: 1000,
                        page_token: token.clone(),
                        filter: Some(thread_filter(thread)),
                        newest_first: false,
                    },
                )
                .await?;
            token = page.next().map(str::to_owned);
            events.extend(page.items.into_iter().map(Event::MessageUpserted));
            if token.is_none() {
                break;
            }
        }
        if let Ok(rs) = self.chat.get_thread_read_state(thread).await {
            events.push(Event::ThreadReadState(rs));
        }
        let me = self.me();
        let applied = self
            .db
            .call(move |c| {
                let tx = c.transaction()?;
                let a = apply(&tx, &events, &me)?;
                tx.commit()?;
                Ok(a)
            })
            .await?;
        self.after_apply(applied, false).await;
        Ok(())
    }

    /// Pages one batch of older history into the cache. Returns how many messages arrived.
    pub async fn load_older(&self, space: &str) -> Result<u32> {
        self.page_back(space, OLDER_PAGE).await
    }

    async fn page_back(&self, space: &str, page_size: u32) -> Result<u32> {
        let space_owned = space.to_owned();
        let (low_water, complete) = self.db.call(move |c| read::history_bounds(c, &space_owned)).await?;
        if complete {
            return Ok(0);
        }
        // Page back from the contiguous mark, not the oldest cached message: stray old
        // messages (a reaction refetch, a search hit) must not hide the gap behind them.
        let page = self
            .chat
            .list_messages(
                space,
                &ListMessages {
                    page_size,
                    newest_first: true,
                    filter: low_water.map(|o| older_than_filter(&ts::format(o))),
                    ..Default::default()
                },
            )
            .await?;
        let n = page.items.len() as u32;
        let done = page.next().is_none();
        let new_low = page
            .items
            .iter()
            .filter_map(|m| m.create_time.as_deref().and_then(ts::parse))
            .min();
        let events: Vec<Event> = page.items.into_iter().map(Event::MessageUpserted).collect();
        let me = self.me();
        let space_owned = space.to_owned();
        self.db
            .call(move |c| {
                let tx = c.transaction()?;
                // Signed out or the space left while the page was in flight: keep nothing.
                if !read::space_exists(&tx, &space_owned)? {
                    return Ok(());
                }
                apply(&tx, &events, &me)?;
                // The page continues the history below the mark we started from. If the mark
                // moved meanwhile (a concurrent page-back, or a gap reset by a refetch), this
                // page proves nothing about contiguity: leave the mark alone.
                if read::history_bounds(&tx, &space_owned)?.0 == low_water {
                    if let Some(low) = new_low {
                        write::set_low_water(&tx, &space_owned, low)?;
                    }
                    if done {
                        write::set_history_complete(&tx, &space_owned, true)?;
                    }
                }
                tx.commit()?;
                Ok(())
            })
            .await?;
        if done {
            self.emit_spaces();
        }
        self.index_wake.notify_one();
        Ok(n)
    }

    /// Everyone who wrote a cached message, most recent writer first: the search box's
    /// suggestions for `from:`.
    pub fn list_senders(&self) -> Result<Vec<views::UserRef>> {
        let me = self.me();
        self.db.with(|c| {
            let rows = read::senders(c)?;
            let users: HashMap<String, read::UserRow> = rows.iter().map(|u| (u.name.clone(), u.clone())).collect();
            let b = Builder { me: &me, users: &users };
            Ok(rows.iter().map(|u| b.user(&u.name)).collect())
        })
    }

    pub fn list_members(&self, space: &str) -> Result<Vec<views::UserRef>> {
        let me = self.me();
        self.db.with(|c| {
            let rows = read::members(c, space)?;
            let users: HashMap<String, read::UserRow> = rows.iter().map(|u| (u.name.clone(), u.clone())).collect();
            let b = Builder { me: &me, users: &users };
            Ok(rows.iter().map(|u| b.user(&u.name)).collect())
        })
    }

    /// Blocking: runs the embedding model for the query. Call from the blocking pool.
    pub fn search(&self, query: &str, limit: u32, offset: u32) -> Result<SearchResult> {
        let q = search::parse(query);
        // The query vector first, outside the DB lock. Meaning waits until the indexer has
        // loaded the model: typing never blocks on a load.
        let embedder = self.embedder.as_ref().filter(|e| e.ready());
        let query_vec = match embedder {
            Some(e) if q.text().chars().count() >= 3 => e
                .query(&q.text())
                .map_err(|err| tracing::debug!("query embedding failed: {err}"))
                .ok(),
            _ => None,
        };
        let me = self.me();
        self.db.with(|c| {
            // Results must reflect the latest messages; a status-only call (empty query) need
            // not.
            let asks = !q.terms.is_empty() || q.has_filter();
            if asks {
                search::rebuild_dirty(c)?;
            }
            let ranked = if !asks {
                vec![]
            } else {
                let mut vectors = self.vectors.lock().unwrap();
                if let (Some(e), Some(_), None) = (embedder, &query_vec, vectors.as_ref()) {
                    *vectors = Some(search::load_vectors(c, e.model())?);
                }
                search::rank(c, &q, query_vec.as_deref(), vectors.as_ref())?
            };
            let page: Vec<search::Ranked> = ranked
                .into_iter()
                .skip(offset as usize)
                .take(limit.clamp(1, 100) as usize)
                .collect();
            Ok(SearchResult {
                hits: search::hits(c, &me, &q, &page)?,
                covered_from_us: read::search_coverage(c)?,
                history_left: search::history_left(c)?,
                meaning: self.meaning_status(c)?,
            })
        })
    }

    fn meaning_status(&self, c: &Connection) -> Result<MeaningStatus> {
        let Some(e) = self.embedder.as_ref().filter(|e| e.available()) else {
            return Ok(MeaningStatus {
                available: false,
                ready: false,
                indexed: 0,
                total: 0,
            });
        };
        let (total, indexed) = search::counts(c, e.model())?;
        Ok(MeaningStatus {
            available: true,
            ready: e.ready(),
            indexed,
            total,
        })
    }

    // ---------------------------------------------------------------- background work

    /// Downloads every conversation's older history, so search reaches back to the start.
    /// Runs forever; once everything is complete it only rechecks for new conversations.
    pub async fn backfill(self: Arc<Self>) {
        loop {
            let signed_in = self.tokens.as_ref().is_some_and(|t| t.has_refresh_token());
            let bootstrapped = self
                .db
                .call(|c| kv_get(c, "bootstrapped"))
                .await
                .ok()
                .flatten()
                .is_some();
            let wait = if !(signed_in && bootstrapped) {
                Duration::from_secs(20)
            } else {
                match self.backfill_pass().await {
                    Ok(()) => BACKFILL_RECHECK,
                    Err(e) => {
                        tracing::info!("history download paused: {e}");
                        Duration::from_secs(120)
                    }
                }
            };
            tokio::time::sleep(wait).await;
        }
    }

    /// Pages back every incomplete space, most recently active first. Stops early (Err) when
    /// offline or signed out; a space that keeps failing is skipped until the next pass.
    pub async fn backfill_pass(&self) -> Result<()> {
        let spaces: Vec<String> = self
            .db
            .call(|c| {
                let mut st =
                    c.prepare("SELECT name FROM space WHERE history_complete = 0 ORDER BY last_active_us DESC")?;
                Ok(st.query_map([], |r| r.get(0))?.collect::<rusqlite::Result<_>>()?)
            })
            .await?;
        for space in spaces {
            let mut stalls = 0;
            loop {
                let s = space.clone();
                let before = self.db.call(move |c| read::history_bounds(c, &s)).await?;
                match self.page_back(&space, BACKFILL_PAGE).await {
                    Ok(_) => {}
                    Err(e @ Error::AuthRequired) => return Err(e),
                    Err(e) if e.is_offline() => return Err(e),
                    Err(e) => {
                        tracing::warn!(%space, "history download failed: {e}");
                        break;
                    }
                }
                let s = space.clone();
                let after = self.db.call(move |c| read::history_bounds(c, &s)).await?;
                if after.1 {
                    break;
                }
                // The mark must move down every page; if it does not, stop rather than spin.
                let moved = match (before.0, after.0) {
                    (_, None) => false,
                    (None, Some(_)) => true,
                    (Some(b), Some(a)) => a < b,
                };
                if moved {
                    stalls = 0;
                } else {
                    stalls += 1;
                    if stalls >= 3 {
                        tracing::warn!(%space, "history download made no progress; skipping");
                        break;
                    }
                }
                tokio::time::sleep(BACKFILL_PAUSE).await;
            }
        }
        Ok(())
    }

    /// Keeps search stretches and their vectors current. Runs forever; idle when nothing
    /// changed.
    pub async fn index(self: Arc<Self>) {
        loop {
            if let Err(e) = self.index_pass().await {
                tracing::warn!("search index: {e}");
            }
            tokio::select! {
                _ = self.index_wake.notified() => {}
                _ = tokio::time::sleep(Duration::from_secs(300)) => {}
            }
            // Let a burst of changes settle.
            tokio::time::sleep(Duration::from_secs(2)).await;
        }
    }

    pub async fn index_pass(self: &Arc<Self>) -> Result<()> {
        self.db.call(|c| search::rebuild_dirty(c).map(|_| ())).await?;
        let Some(e) = self.embedder.clone().filter(|e| e.available()) else {
            return Ok(());
        };
        // Searches use meaning only once the model is loaded; load it even with nothing to
        // embed (a restart with everything indexed).
        if !e.ready() {
            let e2 = e.clone();
            tokio::task::spawn_blocking(move || e2.load())
                .await
                .map_err(|e| Error::Other(format!("model load task: {e}")))??;
        }
        let model = e.model().to_owned();
        loop {
            let m = model.clone();
            let todo = self.db.call(move |c| search::unembedded(c, &m, EMBED_FETCH)).await?;
            if todo.is_empty() {
                break;
            }
            let e2 = e.clone();
            let vecs = tokio::task::spawn_blocking(move || {
                // One passage per model call: with dynamic int8 quantisation a vector depends
                // on its batch-mates. Too little language to place, or text the model fails
                // on: an empty vector, which matches nothing and is not retried until the
                // stretch changes. One bad stretch must not stop the rest from indexing. When
                // every stretch fails, a probe tells a broken model (store nothing, so the next
                // pass retries) from a batch of bad stretches (store them empty, move on).
                let (mut tried, mut failed) = (0, 0);
                let out: Vec<(String, Vec<f32>)> = todo
                    .into_iter()
                    .map(|(hash, text)| {
                        if search::chunk::too_short_for_meaning(&text) {
                            return (hash, Vec::new());
                        }
                        tried += 1;
                        match e2.passages(&[text]) {
                            Ok(mut v) if !v.is_empty() => (hash, v.remove(0)),
                            Ok(_) => (hash, Vec::new()),
                            Err(err) => {
                                failed += 1;
                                tracing::warn!(%hash, "embedding a stretch failed: {err}");
                                (hash, Vec::new())
                            }
                        }
                    })
                    .collect();
                if tried > 0
                    && failed == tried
                    && let Err(err) = e2.passages(&["a short probe sentence for the model".to_owned()])
                {
                    return Err(Error::Other(format!("the embedding model fails: {err}")));
                }
                Ok(out)
            })
            .await
            .map_err(|e| Error::Other(format!("embedding task: {e}")))??;
            // DB first, cache second, both under the DB lock (lock order: db, then vectors).
            let (this, m) = (self.clone(), model.clone());
            self.db
                .call(move |c| {
                    let kept = search::store_vectors(c, &m, vecs)?;
                    if let Some(map) = this.vectors.lock().unwrap().as_mut() {
                        map.extend(kept);
                    }
                    Ok(())
                })
                .await?;
        }
        // Vectors of stretches that changed or went.
        let this = self.clone();
        self.db
            .call(move |c| {
                let dropped = search::drop_orphan_vectors(c)?;
                if let Some(map) = this.vectors.lock().unwrap().as_mut() {
                    for h in &dropped {
                        map.remove(h);
                    }
                }
                Ok(())
            })
            .await?;
        Ok(())
    }

    // ---------------------------------------------------------------- focus

    /// What the UI has open, for the "no notification for the open space" rule.
    pub fn set_open(&self, space: Option<String>, thread: Option<String>) {
        let mut f = self.focus.lock().unwrap();
        f.space = space;
        f.thread = thread;
    }

    pub fn set_window_focused(&self, focused: bool) {
        self.focus.lock().unwrap().window_focused = focused;
    }

    pub fn set_space_prefs(&self, space: &str, muted: Option<bool>, starred: Option<bool>) -> Result<()> {
        self.db.with(|c| write::set_space_prefs(c, space, muted, starred))?;
        self.emit_spaces();
        Ok(())
    }

    // ---------------------------------------------------------------- writes

    pub async fn send_message(self: &Arc<Self>, space: &str, thread: Option<&str>, text: &str) -> Result<MessageView> {
        let client_id = format!("client-{}", uuid::Uuid::new_v4().simple());
        let me = self.me();
        let (space_o, thread_o, text_o, cid) = (
            space.to_owned(),
            thread.map(str::to_owned),
            text.to_owned(),
            client_id.clone(),
        );
        let name = self
            .db
            .call(move |c| write::insert_pending(c, &space_o, thread_o.as_deref(), &text_o, &cid, &me, ts::now()))
            .await?;
        let view = self
            .message_views_by_name(std::slice::from_ref(&name))?
            .into_iter()
            .next()
            .ok_or_else(|| Error::Other("pending message vanished".into()))?;
        self.sink.messages_upserted(space, vec![view.clone()]);
        let this = self.clone();
        let (space, thread, text) = (space.to_owned(), thread.map(str::to_owned), text.to_owned());
        tokio::spawn(async move {
            this.deliver(&space, thread.as_deref(), &text, &client_id, &name).await;
        });
        Ok(view)
    }

    async fn deliver(&self, space: &str, thread: Option<&str>, text: &str, client_id: &str, pending: &str) {
        let created = match self.chat.create_message(space, text, thread, client_id).await {
            // An earlier attempt (retried after a timeout or 5xx) already landed: the client
            // ID is taken, so fetch that message instead of reporting a failure.
            Err(e) if e.is_status("ALREADY_EXISTS") => {
                self.chat.get_message(&format!("{space}/messages/{client_id}")).await
            }
            other => other,
        };
        match created {
            Ok(mut m) => {
                if m.client_assigned_message_id.is_none() {
                    m.client_assigned_message_id = Some(client_id.to_owned());
                }
                let me = self.me();
                if let Ok(a) = self
                    .db
                    .call(move |c| {
                        let tx = c.transaction()?;
                        let a = apply(&tx, &[Event::MessageUpserted(m)], &me)?;
                        tx.commit()?;
                        Ok(a)
                    })
                    .await
                {
                    // The pending row was renamed; tell the UI the old name is gone.
                    self.sink.messages_deleted(space, vec![pending.to_owned()]);
                    self.after_apply(a, false).await;
                }
            }
            Err(e) => {
                tracing::warn!("send failed: {e}");
                let p = pending.to_owned();
                let _ = self.db.call(move |c| write::set_local_state(c, &p, "failed")).await;
                if let Ok(v) = self.message_views_by_name(&[pending.to_owned()]) {
                    self.sink.messages_upserted(space, v);
                }
            }
        }
    }

    /// Retries a failed send with its original client ID.
    pub async fn retry_send(self: &Arc<Self>, pending: &str) -> Result<()> {
        let p = pending.to_owned();
        let row = self
            .db
            .call(move |c| read::message(c, &p))
            .await?
            .ok_or_else(|| Error::Other("not found".into()))?;
        let client_id: Option<String> = {
            let p = pending.to_owned();
            self.db
                .call(move |c| Ok(c.query_row("SELECT client_id FROM message WHERE name = ?1", [&p], |r| r.get(0))?))
                .await?
        };
        let client_id = client_id.ok_or_else(|| Error::Other("no client id".into()))?;
        let p = pending.to_owned();
        self.db.call(move |c| write::set_local_state(c, &p, "pending")).await?;
        if let Ok(v) = self.message_views_by_name(&[pending.to_owned()]) {
            self.sink.messages_upserted(&row.space, v);
        }
        let this = self.clone();
        let pending = pending.to_owned();
        tokio::spawn(async move {
            let text = row.text.clone().unwrap_or_default();
            this.deliver(&row.space, row.thread.as_deref(), &text, &client_id, &pending)
                .await;
        });
        Ok(())
    }

    pub async fn discard_failed(&self, pending: &str) -> Result<()> {
        let p = pending.to_owned();
        let row = self.db.call(move |c| read::message(c, &p)).await?;
        if let Some(r) = row.filter(|r| r.local_state == "failed") {
            let n = r.name.clone();
            self.db.call(move |c| write::delete_row(c, &n)).await?;
            self.sink.messages_deleted(&r.space, vec![r.name]);
        }
        Ok(())
    }

    pub async fn edit_message(&self, name: &str, text: &str) -> Result<()> {
        let n = name.to_owned();
        let old = self
            .db
            .call(move |c| read::message(c, &n))
            .await?
            .ok_or_else(|| Error::Other("not found".into()))?;
        let (n, t) = (name.to_owned(), text.to_owned());
        self.db.call(move |c| write::set_text_local(c, &n, &t)).await?;
        self.emit_messages(&old.space, &[name.to_owned()]);
        match self.chat.update_message_text(name, text).await {
            Ok(m) => {
                let me = self.me();
                let a = self
                    .db
                    .call(move |c| apply(c, &[Event::MessageUpserted(m)], &me))
                    .await?;
                self.after_apply(a, false).await;
                Ok(())
            }
            Err(e) => {
                let n = name.to_owned();
                let (ot, of) = (old.text.clone(), old.formatted_text.clone());
                self.db
                    .call(move |c| {
                        c.execute(
                            "UPDATE message SET text = ?2, formatted_text = ?3 WHERE name = ?1",
                            rusqlite::params![n, ot, of],
                        )?;
                        Ok(())
                    })
                    .await?;
                self.emit_messages(&old.space, &[name.to_owned()]);
                Err(e)
            }
        }
    }

    pub async fn delete_message(&self, name: &str) -> Result<()> {
        if name.starts_with("pending:") {
            return self.discard_failed(name).await;
        }
        self.chat.delete_message(name).await?;
        let n = name.to_owned();
        let me = self.me();
        let a = self
            .db
            .call(move |c| {
                apply(
                    c,
                    &[Event::MessageDeleted {
                        name: n,
                        delete_time: Some(ts::format(ts::now())),
                    }],
                    &me,
                )
            })
            .await?;
        // Deleted messages stay as a "Message deleted" row; re-emit it as an upsert.
        let names: Vec<String> = a.deleted.iter().map(|(_, n)| n.clone()).collect();
        let space = crate::chat::space_of(name);
        self.emit_messages(&space, &names);
        self.emit_spaces();
        Ok(())
    }

    pub async fn toggle_reaction(&self, message: &str, emoji: &str) -> Result<()> {
        let emoji = Emoji::unicode(emoji);
        let key = emoji.key();
        let (m, k) = (message.to_owned(), key.clone());
        let (existing, summaries) = self
            .db
            .call(move |c| {
                let existing = read::my_reaction_name(c, &m, &k)?;
                let row = read::message(c, &m)?;
                Ok((existing, row.and_then(|r| r.reactions)))
            })
            .await?;
        let mut sums: Vec<EmojiReactionSummary> = summaries
            .as_deref()
            .and_then(|s| serde_json::from_str(s).ok())
            .unwrap_or_default();
        let adding = existing.is_none();
        match sums.iter_mut().find(|s| s.emoji.key() == key) {
            Some(s) => {
                let n = s.reaction_count.unwrap_or(0) as i64 + if adding { 1 } else { -1 };
                s.reaction_count = Some(n.max(0) as u32);
            }
            None if adding => sums.push(EmojiReactionSummary {
                emoji: emoji.clone(),
                reaction_count: Some(1),
            }),
            None => {}
        }
        sums.retain(|s| s.reaction_count.unwrap_or(0) > 0);
        let json = serde_json::to_string(&sums)?;
        let (m, k) = (message.to_owned(), key.clone());
        self.db
            .call(move |c| {
                write::set_reactions(c, &m, &json)?;
                if adding {
                    write::set_my_reaction(c, &m, &k, "pending")
                } else {
                    write::clear_my_reaction(c, &m, &k)
                }
            })
            .await?;
        let space = crate::chat::space_of(message);
        self.emit_messages(&space, &[message.to_owned()]);

        let result = if adding {
            self.chat.create_reaction(message, &emoji).await.map(|r| Some(r.name))
        } else {
            match existing.as_deref() {
                Some(n) if n != "pending" => self.chat.delete_reaction(n).await.map(|_| None),
                _ => Ok(None),
            }
        };
        let (m, k) = (message.to_owned(), key.clone());
        match &result {
            Ok(Some(rname)) => {
                let r = rname.clone();
                self.db.call(move |c| write::set_my_reaction(c, &m, &k, &r)).await?;
            }
            Ok(None) => {}
            Err(_) => {
                if adding {
                    self.db.call(move |c| write::clear_my_reaction(c, &m, &k)).await?;
                }
            }
        }
        // Truth from the server either way.
        if let Ok(fresh) = self.chat.get_message(message).await {
            let me = self.me();
            let a = self
                .db
                .call(move |c| apply(c, &[Event::MessageUpserted(fresh)], &me))
                .await?;
            self.after_apply(a, false).await;
        }
        result.map(|_| ())
    }

    /// Marks a space read up to `up_to` locally now and at Google after a 1 s debounce.
    pub async fn mark_read(self: &Arc<Self>, space: &str, up_to: Micros) -> Result<()> {
        let s = space.to_owned();
        let current = self.db.call(move |c| read::space_read(c, &s)).await?;
        if current.is_some_and(|c| c >= up_to) {
            return Ok(());
        }
        let s = space.to_owned();
        self.db.call(move |c| write::set_space_read(c, &s, Some(up_to))).await?;
        self.emit_spaces();
        self.read_marks.lock().unwrap().insert(space.to_owned(), up_to);
        let this = self.clone();
        let space = space.to_owned();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_secs(1)).await;
            let Some(at) = this.read_marks.lock().unwrap().remove(&space) else {
                return;
            };
            // One retry: otherwise the next reconcile brings the old mark back and the
            // conversation shows unread again.
            for wait in [Duration::ZERO, Duration::from_secs(30)] {
                tokio::time::sleep(wait).await;
                // A newer mark set meanwhile goes out on its own; retrying this older one
                // would move the read position backwards at Google.
                if !wait.is_zero() {
                    let s = space.clone();
                    let local = this.db.call(move |c| read::space_read(c, &s)).await.ok().flatten();
                    if local.is_some_and(|l| l > at) {
                        return;
                    }
                }
                match this.chat.update_space_read_state(&space, &ts::format(at + 1)).await {
                    Ok(_) => return,
                    Err(e) => tracing::warn!("mark read failed: {e}"),
                }
            }
        });
        Ok(())
    }

    /// Local only: the Chat API has no thread read-state update (see api-findings.md).
    pub async fn mark_thread_read(&self, thread: &str, up_to: Micros) -> Result<()> {
        let t = thread.to_owned();
        self.db
            .call(move |c| write::set_thread_read(c, &t, Some(up_to)))
            .await?;
        Ok(())
    }

    fn emit_messages(&self, space: &str, names: &[String]) {
        if let Ok(v) = self.message_views_by_name(names) {
            self.sink.messages_upserted(space, v);
        }
    }

    /// Bytes for an attachment, from the disk cache or the API.
    /// Attachment bytes from the disk cache (a file per resource, named by its hash, with the
    /// content type beside it) or the API.
    ///
    /// The resource name comes from the webview, so it must be one that appears in a cached
    /// message: otherwise a crafted name (`../spaces?...`) would let the page read any Chat
    /// endpoint with the user's token.
    pub async fn media(&self, cache_dir: &std::path::Path, resource: &str) -> Result<(Vec<u8>, Option<String>)> {
        use sha2::{Digest, Sha256};
        let r = resource.to_owned();
        let known = self.db.call(move |c| read::is_known_attachment(c, &r)).await?;
        let shape_ok = !resource.is_empty()
            && !resource.contains("..")
            && resource
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || "/+-_.=".contains(c));
        if !known || !shape_ok {
            return Err(Error::Other("unknown attachment".into()));
        }
        let key = format!("{:x}", Sha256::digest(resource.as_bytes()));
        let path = cache_dir.join(&key);
        let mime_path = cache_dir.join(format!("{key}.type"));
        if let Ok(bytes) = tokio::fs::read(&path).await {
            let mime = tokio::fs::read_to_string(&mime_path).await.ok();
            let mime = sniff_image(&bytes).map(str::to_owned).or(mime);
            return Ok((bytes, mime));
        }
        let (bytes, mime) = self.chat.download_media(resource).await?;
        // Google serves media as application/octet-stream, which WebKit will not draw in an
        // <img>; the bytes say what the image really is.
        let mime = sniff_image(&bytes).map(str::to_owned).or(mime);
        tokio::fs::create_dir_all(cache_dir).await?;
        tokio::fs::write(&path, &bytes).await?;
        if let Some(m) = &mime {
            tokio::fs::write(&mime_path, m).await?;
        }
        Ok((bytes, mime))
    }

    pub fn last_own_message(&self, space: &str, thread: Option<&str>) -> Result<Option<MessageView>> {
        let me = self.me();
        let row = self.db.with(|c| read::last_own(c, space, thread, &me))?;
        match row {
            Some(r) => Ok(self.message_views_by_name(&[r.name])?.into_iter().next()),
            None => Ok(None),
        }
    }

    pub fn message_view(&self, name: &str) -> Result<Option<MessageView>> {
        Ok(self.message_views_by_name(&[name.to_owned()])?.into_iter().next())
    }
}

/// The image type from the file's first bytes.
fn sniff_image(b: &[u8]) -> Option<&'static str> {
    match b {
        [0x89, b'P', b'N', b'G', ..] => Some("image/png"),
        [0xFF, 0xD8, 0xFF, ..] => Some("image/jpeg"),
        [b'G', b'I', b'F', b'8', ..] => Some("image/gif"),
        [b'R', b'I', b'F', b'F', _, _, _, _, b'W', b'E', b'B', b'P', ..] => Some("image/webp"),
        _ => None,
    }
}

/// Errors that mean "live events are not set up for this project/user" rather than a
/// passing problem: the Chat app is missing (404), or permission is lacking (403).
fn is_config_error(e: &Error) -> bool {
    matches!(e.http_status(), Some(403 | 404))
        || e.is_status("PERMISSION_DENIED")
        || e.is_status("NOT_FOUND")
        || e.is_status("FAILED_PRECONDITION")
}

fn short(e: &Error) -> String {
    let s = e.to_string();
    if s.chars().count() > 120 {
        s.chars().take(117).collect::<String>() + "…"
    } else {
        s
    }
}

#[cfg(test)]
mod tests;
