//! Engine tests against an in-memory fake of the Chat and People APIs.

use std::sync::{Arc, Mutex};

use async_trait::async_trait;

use super::*;
use crate::chat::*;
use crate::people::Person;

const ME: &str = "users/me";

#[derive(Default)]
struct World {
    spaces: Vec<Space>,
    messages: Vec<Message>,
    members: Vec<Membership>,
    read: HashMap<String, String>,
    events: Vec<SpaceEvent>,
    created: Vec<(String, String, Option<String>)>,
    fail_create: bool,
}

#[derive(Default, Clone)]
struct Fake(Arc<Mutex<World>>);

fn t(s: &str) -> Micros {
    ts::parse(s).unwrap()
}

#[async_trait]
impl ChatApi for Fake {
    async fn list_spaces(&self, _: Option<&str>) -> Result<Page<Space>> {
        Ok(Page {
            items: self.0.lock().unwrap().spaces.clone(),
            next_page_token: None,
        })
    }
    async fn get_space(&self, name: &str) -> Result<Space> {
        self.0
            .lock()
            .unwrap()
            .spaces
            .iter()
            .find(|s| s.name == name)
            .cloned()
            .ok_or(Error::Other("nf".into()))
    }
    async fn list_messages(&self, space: &str, o: &ListMessages) -> Result<Page<Message>> {
        let w = self.0.lock().unwrap();
        let mut v: Vec<Message> = w.messages.iter().filter(|m| m.space_name() == space).cloned().collect();
        if let Some(f) = &o.filter {
            if let Some(rest) = f.strip_prefix("createTime < \"") {
                let cutoff = t(rest.trim_end_matches('"'));
                v.retain(|m| t(m.create_time.as_deref().unwrap()) < cutoff);
            }
            if let Some(th) = f.strip_prefix("thread.name = ") {
                v.retain(|m| m.thread.as_ref().map(|x| x.name.as_str()) == Some(th));
            }
        }
        v.sort_by_key(|m| t(m.create_time.as_deref().unwrap()));
        if o.newest_first {
            v.reverse();
        }
        let more = v.len() > o.page_size as usize;
        v.truncate(o.page_size as usize);
        Ok(Page {
            items: v,
            next_page_token: more.then(|| "more".into()),
        })
    }
    async fn get_message(&self, name: &str) -> Result<Message> {
        self.0
            .lock()
            .unwrap()
            .messages
            .iter()
            .find(|m| m.name == name)
            .cloned()
            .ok_or(Error::Other("nf".into()))
    }
    async fn create_message(&self, space: &str, text: &str, thread: Option<&str>, client_id: &str) -> Result<Message> {
        let mut w = self.0.lock().unwrap();
        if w.fail_create {
            return Err(Error::Network("offline".into()));
        }
        w.created.push((space.into(), text.into(), thread.map(str::to_owned)));
        let n = w.messages.len();
        let m = Message {
            name: format!("{space}/messages/new{n}"),
            sender: Some(User {
                name: ME.into(),
                ..Default::default()
            }),
            create_time: Some("2024-02-01T00:00:00Z".into()),
            text: Some(text.into()),
            formatted_text: Some(text.into()),
            thread: Some(Thread {
                name: thread
                    .map(str::to_owned)
                    .unwrap_or_else(|| format!("{space}/threads/new{n}")),
            }),
            thread_reply: Some(thread.is_some()),
            client_assigned_message_id: Some(client_id.into()),
            ..Default::default()
        };
        w.messages.push(m.clone());
        Ok(m)
    }
    async fn update_message_text(&self, name: &str, text: &str) -> Result<Message> {
        let mut w = self.0.lock().unwrap();
        let m = w
            .messages
            .iter_mut()
            .find(|m| m.name == name)
            .ok_or(Error::Other("nf".into()))?;
        m.text = Some(text.into());
        m.formatted_text = Some(text.into());
        m.last_update_time = Some("2024-03-01T00:00:00Z".into());
        Ok(m.clone())
    }
    async fn delete_message(&self, _: &str) -> Result<()> {
        Ok(())
    }
    async fn create_reaction(&self, message: &str, emoji: &Emoji) -> Result<Reaction> {
        let mut w = self.0.lock().unwrap();
        if let Some(m) = w.messages.iter_mut().find(|m| m.name == message) {
            m.emoji_reaction_summaries = Some(vec![EmojiReactionSummary {
                emoji: emoji.clone(),
                reaction_count: Some(1),
            }]);
        }
        Ok(Reaction {
            name: format!("{message}/reactions/r1"),
            user: None,
            emoji: emoji.clone(),
        })
    }
    async fn delete_reaction(&self, _: &str) -> Result<()> {
        Ok(())
    }
    async fn list_members(&self, space: &str, _: Option<&str>) -> Result<Page<Membership>> {
        let w = self.0.lock().unwrap();
        Ok(Page {
            items: w.members.iter().filter(|m| m.space_name() == space).cloned().collect(),
            next_page_token: None,
        })
    }
    async fn get_space_read_state(&self, space: &str) -> Result<SpaceReadState> {
        let w = self.0.lock().unwrap();
        Ok(SpaceReadState {
            name: format!("users/me/{space}/spaceReadState"),
            last_read_time: w.read.get(space).cloned(),
        })
    }
    async fn update_space_read_state(&self, space: &str, at: &str) -> Result<SpaceReadState> {
        self.0.lock().unwrap().read.insert(space.into(), at.into());
        self.get_space_read_state(space).await
    }
    async fn get_thread_read_state(&self, thread: &str) -> Result<ThreadReadState> {
        Ok(ThreadReadState {
            name: format!("users/me/{thread}/threadReadState"),
            last_read_time: None,
        })
    }
    async fn list_space_events(&self, space: &str, start: &str, _: Option<&str>) -> Result<Page<SpaceEvent>> {
        let w = self.0.lock().unwrap();
        let from = t(start);
        Ok(Page {
            items: w
                .events
                .iter()
                .filter(|e| e.name.starts_with(space) && t(e.event_time.as_deref().unwrap()) >= from)
                .cloned()
                .collect(),
            next_page_token: None,
        })
    }
    async fn download_media(&self, _: &str) -> Result<(Vec<u8>, Option<String>)> {
        Ok((b"png".to_vec(), Some("image/png".into())))
    }
}

struct FakePeople;

#[async_trait]
impl PeopleApi for FakePeople {
    async fn me(&self) -> Result<Person> {
        Ok(Person {
            user: ME.into(),
            display_name: Some("Me".into()),
            email: Some("me@x.nl".into()),
            avatar_url: None,
        })
    }
    async fn resolve(&self, users: &[String]) -> Result<Vec<Person>> {
        Ok(users
            .iter()
            .filter(|u| u.as_str() == "users/ada")
            .map(|u| Person {
                user: u.clone(),
                display_name: Some("Ada Lovelace".into()),
                email: None,
                avatar_url: None,
            })
            .collect())
    }
}

#[derive(Default)]
struct Rec {
    upserts: Mutex<Vec<(String, Vec<MessageView>)>>,
    deletes: Mutex<Vec<(String, Vec<String>)>>,
    notes: Mutex<Vec<Notification>>,
}

impl Sink for Rec {
    fn spaces_changed(&self, _: Vec<SpaceSummary>) {}
    fn messages_upserted(&self, space: &str, m: Vec<MessageView>) {
        self.upserts.lock().unwrap().push((space.into(), m));
    }
    fn messages_deleted(&self, space: &str, n: Vec<String>) {
        self.deletes.lock().unwrap().push((space.into(), n));
    }
    fn status(&self, _: SyncStatus) {}
    fn auth(&self, _: AuthState) {}
    fn notify(&self, n: Notification) {
        self.notes.lock().unwrap().push(n);
    }
    fn badge(&self, _: u32) {}
}

fn user(name: &str, display: Option<&str>) -> User {
    User {
        name: name.into(),
        display_name: display.map(str::to_owned),
        ..Default::default()
    }
}

fn message(space: &str, id: &str, sender: &str, at: &str, text: &str, thread: &str, reply: bool) -> Message {
    Message {
        name: format!("{space}/messages/{id}"),
        sender: Some(user(sender, Some(if sender == ME { "Me" } else { "Ada Lovelace" }))),
        create_time: Some(at.into()),
        text: Some(text.into()),
        formatted_text: Some(text.into()),
        thread: Some(Thread {
            name: format!("{space}/threads/{thread}"),
        }),
        thread_reply: Some(reply),
        space: Some(SpaceRef { name: space.into() }),
        ..Default::default()
    }
}

fn world() -> Fake {
    let f = Fake::default();
    {
        let mut w = f.0.lock().unwrap();
        w.spaces = vec![
            Space {
                name: "spaces/team".into(),
                space_type: Some("SPACE".into()),
                display_name: Some("Team".into()),
                space_threading_state: Some("THREADED_MESSAGES".into()),
                last_active_time: Some("2024-01-01T12:00:00Z".into()),
                ..Default::default()
            },
            Space {
                name: "spaces/dm".into(),
                space_type: Some("DIRECT_MESSAGE".into()),
                last_active_time: Some("2024-01-01T11:00:00Z".into()),
                ..Default::default()
            },
        ];
        w.messages = vec![
            message(
                "spaces/team",
                "1",
                "users/ada",
                "2024-01-01T10:00:00Z",
                "hello **team**",
                "t1",
                false,
            ),
            message("spaces/team", "2", ME, "2024-01-01T10:30:00Z", "reply one", "t1", true),
            message(
                "spaces/team",
                "3",
                "users/ada",
                "2024-01-01T12:00:00Z",
                "deploy is green",
                "t3",
                false,
            ),
            message(
                "spaces/dm",
                "4",
                "users/ada",
                "2024-01-01T11:00:00Z",
                "psst",
                "t4",
                false,
            ),
        ];
        w.members = vec![
            Membership {
                name: "spaces/dm/members/me".into(),
                member: Some(user(ME, None)),
                ..Default::default()
            },
            Membership {
                name: "spaces/dm/members/ada".into(),
                member: Some(user("users/ada", None)),
                ..Default::default()
            },
        ];
        w.read.insert("spaces/team".into(), "2024-01-01T10:15:00Z".into());
        w.read.insert("spaces/dm".into(), "2024-01-01T09:00:00Z".into());
    }
    f
}

async fn engine(f: &Fake) -> (Arc<Engine>, Arc<Rec>) {
    let rec = Arc::new(Rec::default());
    let e = Engine::new(
        Db::open_in_memory().unwrap(),
        Arc::new(f.clone()),
        Arc::new(FakePeople),
        None,
        None,
        rec.clone(),
        None,
    );
    e.ensure_identity().await.unwrap();
    (e, rec)
}

#[tokio::test]
async fn bootstrap_fills_cache_with_unread_and_dm_names() {
    let f = world();
    let (e, _) = engine(&f).await;
    e.bootstrap().await.unwrap();

    let spaces = e.list_spaces().unwrap();
    assert_eq!(spaces.len(), 2);
    let team = spaces.iter().find(|s| s.name == "spaces/team").unwrap();
    // Only top-level messages from others after the read mark count.
    assert_eq!(team.unread, 1);
    let dm = spaces.iter().find(|s| s.name == "spaces/dm").unwrap();
    assert_eq!(dm.kind, "dm");
    assert_eq!(dm.display_name, "Ada Lovelace");
    assert_eq!(dm.unread, 1);

    let msgs = e.get_messages("spaces/team", None, 50).unwrap();
    assert_eq!(msgs.len(), 2, "replies stay out of the channel view");
    let root = &msgs[0];
    assert_eq!(root.sender.display_name, "Ada Lovelace");
    assert_eq!(root.thread_summary.as_ref().unwrap().reply_count, 1);
    assert!(matches!(&root.blocks[0], render::Block::Paragraph { .. }));

    let thread = e.get_thread_cached("spaces/team/threads/t1").unwrap();
    assert_eq!(thread.len(), 2);
    assert!(!thread[0].is_reply && thread[1].is_reply && thread[1].is_own);
}

#[tokio::test]
async fn reconcile_replays_missed_events_without_duplicates() {
    let f = world();
    let (e, _) = engine(&f).await;
    e.bootstrap().await.unwrap();

    {
        let mut w = f.0.lock().unwrap();
        let mut edited = w.messages[0].clone();
        edited.text = Some("hello edited".into());
        edited.formatted_text = Some("hello edited".into());
        edited.last_update_time = Some("2030-01-01T00:00:00Z".into());
        let new = message(
            "spaces/team",
            "5",
            "users/ada",
            "2030-01-01T00:01:00Z",
            "after sleep",
            "t5",
            false,
        );
        let now = ts::format(ts::now() + MINUTE);
        w.events = vec![
            SpaceEvent {
                name: "spaces/team/spaceEvents/1".into(),
                event_time: Some(now.clone()),
                event_type: "google.workspace.chat.message.v1.updated".into(),
                data: serde_json::from_value(serde_json::json!({"messageUpdatedEventData": {"message": edited}}))
                    .unwrap(),
            },
            SpaceEvent {
                name: "spaces/team/spaceEvents/2".into(),
                event_time: Some(now.clone()),
                event_type: "google.workspace.chat.message.v1.created".into(),
                data: serde_json::from_value(serde_json::json!({"messageCreatedEventData": {"message": new}})).unwrap(),
            },
            SpaceEvent {
                name: "spaces/team/spaceEvents/3".into(),
                event_time: Some(now),
                event_type: "google.workspace.chat.message.v1.created".into(),
                data: serde_json::from_value(serde_json::json!({"messageCreatedEventData": {"message": new}})).unwrap(),
            },
        ];
        w.spaces[0].last_active_time = Some("2030-01-01T00:01:00Z".into());
    }
    e.reconcile(false).await.unwrap();
    e.reconcile(false).await.unwrap();

    let msgs = e.get_messages("spaces/team", None, 50).unwrap();
    assert_eq!(msgs.len(), 3);
    assert!(msgs[0].edited);
    assert_eq!(render::plain(&msgs[0].blocks), "hello edited");
    assert_eq!(render::plain(&msgs[2].blocks), "after sleep");
}

/// Deletes and edits do not move a space's lastActiveTime; a full reconcile (startup, wake,
/// polling) must still replay them.
#[tokio::test]
async fn full_reconcile_catches_deletes_without_new_activity() {
    let f = world();
    let (e, _) = engine(&f).await;
    e.bootstrap().await.unwrap();
    {
        let mut w = f.0.lock().unwrap();
        let now = ts::format(ts::now() + MINUTE);
        w.events = vec![SpaceEvent {
            name: "spaces/team/spaceEvents/9".into(),
            event_time: Some(now.clone()),
            event_type: "google.workspace.chat.message.v1.deleted".into(),
            data: serde_json::from_value(serde_json::json!({
                "messageDeletedEventData": {"message": {"name": "spaces/team/messages/3", "deleteTime": now}}
            }))
            .unwrap(),
        }];
    }
    // The gated pass skips the space: no newer activity.
    e.reconcile(false).await.unwrap();
    assert!(!e.message_view("spaces/team/messages/3").unwrap().unwrap().deleted);
    e.reconcile(true).await.unwrap();
    let m = e.message_view("spaces/team/messages/3").unwrap().unwrap();
    assert!(m.deleted);
    assert!(m.blocks.is_empty());
    assert_eq!(
        e.search("deploy", 20, 0).unwrap().hits.len(),
        0,
        "deleted text leaves the search index"
    );
}

#[tokio::test]
async fn interrupted_send_becomes_failed_on_restart() {
    let f = world();
    let rec = Arc::new(Rec::default());
    let db = Db::open_in_memory().unwrap();
    db.with(|c| write::insert_pending(c, "spaces/team", None, "half sent", "client-x", ME, 5).map(|_| ()))
        .unwrap();
    let e = Engine::new(db, Arc::new(f.clone()), Arc::new(FakePeople), None, None, rec, None);
    assert_eq!(
        e.message_view("pending:client-x").unwrap().unwrap().local_state,
        "failed"
    );
}

#[tokio::test]
async fn media_refuses_resources_not_in_any_message() {
    let f = world();
    let (e, _) = engine(&f).await;
    e.bootstrap().await.unwrap();
    let dir = std::env::temp_dir().join(format!("kwek-media-{}", uuid::Uuid::new_v4()));
    assert!(e.media(&dir, "../spaces?pageSize=1000").await.is_err());
    assert!(e.media(&dir, "unknownresource").await.is_err());
}

#[tokio::test]
async fn send_is_optimistic_then_replaced_by_server_row() {
    let f = world();
    let (e, rec) = engine(&f).await;
    e.bootstrap().await.unwrap();
    let v = e.send_message("spaces/team", None, "shipping it").await.unwrap();
    assert_eq!(v.local_state, "pending");
    assert!(v.name.starts_with("pending:"));
    for _ in 0..50 {
        if !rec.deletes.lock().unwrap().is_empty() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    let msgs = e.get_messages("spaces/team", None, 50).unwrap();
    let mine: Vec<_> = msgs
        .iter()
        .filter(|m| render::plain(&m.blocks) == "shipping it")
        .collect();
    assert_eq!(mine.len(), 1);
    assert_eq!(mine[0].local_state, "synced");
    assert!(!mine[0].name.starts_with("pending:"));
}

#[tokio::test]
async fn failed_send_stays_visible_and_retries() {
    let f = world();
    let (e, _) = engine(&f).await;
    e.bootstrap().await.unwrap();
    f.0.lock().unwrap().fail_create = true;
    let v = e.send_message("spaces/team", None, "offline msg").await.unwrap();
    let mut state = String::new();
    for _ in 0..50 {
        state = e.message_view(&v.name).unwrap().unwrap().local_state;
        if state == "failed" {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert_eq!(state, "failed");
    f.0.lock().unwrap().fail_create = false;
    e.retry_send(&v.name).await.unwrap();
    for _ in 0..50 {
        if e.message_view(&v.name).unwrap().is_none() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert!(
        e.message_view(&v.name).unwrap().is_none(),
        "pending row renamed to the server name"
    );
    assert_eq!(f.0.lock().unwrap().created.len(), 1);
}

#[tokio::test]
async fn reactions_are_optimistic_and_marked_mine() {
    let f = world();
    let (e, _) = engine(&f).await;
    e.bootstrap().await.unwrap();
    e.toggle_reaction("spaces/team/messages/3", "👍").await.unwrap();
    let m = e.message_view("spaces/team/messages/3").unwrap().unwrap();
    assert_eq!(m.reactions.len(), 1);
    assert!(m.reactions[0].mine);
    assert_eq!(m.reactions[0].count, 1);
}

#[tokio::test]
async fn mark_read_clears_unread_locally() {
    let f = world();
    let (e, _) = engine(&f).await;
    e.bootstrap().await.unwrap();
    e.mark_read("spaces/team", t("2024-01-01T12:00:00Z")).await.unwrap();
    let team = e
        .list_spaces()
        .unwrap()
        .into_iter()
        .find(|s| s.name == "spaces/team")
        .unwrap();
    assert_eq!(team.unread, 0);
}

#[tokio::test]
async fn search_with_filters_and_snippets() {
    let f = world();
    let (e, _) = engine(&f).await;
    e.bootstrap().await.unwrap();
    let r = e.search("deploy", 20, 0).unwrap();
    assert_eq!(r.hits.len(), 1);
    assert_eq!(r.hits[0].space_name, "Team");
    assert!(r.hits[0].snippet.iter().any(|p| p.hit && p.text == "deploy"));
    // Prefix match while typing, and filters.
    assert_eq!(e.search("depl", 20, 0).unwrap().hits.len(), 1);
    assert_eq!(e.search("from:ada psst", 20, 0).unwrap().hits.len(), 1);
    assert_eq!(e.search("in:team psst", 20, 0).unwrap().hits.len(), 0);
    assert_eq!(e.search("after:2024-01-02 deploy", 20, 0).unwrap().hits.len(), 0);
    // FTS syntax in user input is literal, never an error.
    assert!(e.search("NEAR( \"x OR", 20, 0).is_ok());
}

#[tokio::test]
async fn load_older_pages_back_and_marks_complete() {
    let f = world();
    {
        let mut w = f.0.lock().unwrap();
        for i in 0..250 {
            let at = ts::format(t("2023-01-01T00:00:00Z") + i as i64 * MINUTE);
            w.messages.push(message(
                "spaces/team",
                &format!("old{i}"),
                "users/ada",
                &at,
                "old",
                &format!("o{i}"),
                false,
            ));
        }
    }
    let (e, _) = engine(&f).await;
    e.bootstrap().await.unwrap();
    assert_eq!(
        e.get_messages("spaces/team", None, 500).unwrap().len(),
        INITIAL_PAGE as usize - 1
    );
    let mut total = 0;
    loop {
        let n = e.load_older("spaces/team").await.unwrap();
        if n == 0 {
            break;
        }
        total += n;
    }
    assert_eq!(total, 253 - INITIAL_PAGE);
    assert!(
        e.list_spaces()
            .unwrap()
            .iter()
            .find(|s| s.name == "spaces/team")
            .unwrap()
            .history_complete
    );
}

#[test]
fn search_parser() {
    let (q, f) = parse_search("from:ada in:tech before:2024-06-01 deploy green");
    assert_eq!(q, "\"deploy\" \"green\"*");
    assert_eq!(f.from.as_deref(), Some("ada"));
    assert_eq!(f.space.as_deref(), Some("tech"));
    assert!(f.before_us.is_some());
}
