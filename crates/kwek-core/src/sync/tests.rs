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
    /// (space, file name, content type, bytes) per upload.
    uploads: Vec<(String, String, String, Vec<u8>)>,
    fail_upload: bool,
    /// Runs inside the next list_messages call: something else happening mid-request.
    on_list: Option<Box<dyn FnOnce() + Send>>,
    /// A space whose every page is empty yet claims there is more.
    hollow: Option<String>,
    list_calls: usize,
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
        let hook = self.0.lock().unwrap().on_list.take();
        if let Some(h) = hook {
            h();
        }
        let mut w = self.0.lock().unwrap();
        w.list_calls += 1;
        if w.hollow.as_deref() == Some(space) {
            return Ok(Page {
                items: vec![],
                next_page_token: Some("more".into()),
            });
        }
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
    async fn upload_attachment(
        &self,
        space: &str,
        file_name: &str,
        content_type: &str,
        data: &[u8],
    ) -> Result<serde_json::Value> {
        let mut w = self.0.lock().unwrap();
        if w.fail_upload {
            return Err(Error::Network("offline".into()));
        }
        w.uploads
            .push((space.into(), file_name.into(), content_type.into(), data.to_vec()));
        let n = w.uploads.len();
        Ok(serde_json::json!({ "resourceName": format!("uploaded/{n}"), "attachmentUploadToken": "tok" }))
    }
    async fn create_message(
        &self,
        space: &str,
        text: &str,
        thread: Option<&str>,
        client_id: &str,
        attachments: &[serde_json::Value],
    ) -> Result<Message> {
        let mut w = self.0.lock().unwrap();
        if w.fail_create {
            return Err(Error::Network("offline".into()));
        }
        let attachment: Vec<Attachment> = attachments
            .iter()
            .map(|r| Attachment {
                content_type: Some("image/png".into()),
                attachment_data_ref: Some(AttachmentDataRef {
                    resource_name: r["resourceName"].as_str().map(str::to_owned),
                }),
                ..Default::default()
            })
            .collect();
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
            attachment: (!attachment.is_empty()).then_some(attachment),
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
    let e = Engine::new(
        db,
        Arc::new(f.clone()),
        Arc::new(FakePeople),
        None,
        None,
        rec,
        None,
        None,
    );
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
    let v = e
        .send_message("spaces/team", None, "shipping it", vec![])
        .await
        .unwrap();
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
    let v = e
        .send_message("spaces/team", None, "offline msg", vec![])
        .await
        .unwrap();
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

async fn until(mut done: impl FnMut() -> bool) {
    for _ in 0..100 {
        if done() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

#[tokio::test]
async fn images_upload_with_the_message_and_survive_a_retry() {
    let f = world();
    let (e, _) = engine(&f).await;
    e.bootstrap().await.unwrap();
    let dir = std::env::temp_dir().join(format!("kwek-outbox-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("photo.png");
    std::fs::write(&path, b"\x89PNG fake").unwrap();
    let file = Outgoing {
        path: path.clone(),
        name: "photo.png".into(),
        content_type: "image/png".into(),
    };

    // The upload fails first: the message stays, failed, with its image named.
    f.0.lock().unwrap().fail_upload = true;
    let v = e.send_message("spaces/team", None, "", vec![file]).await.unwrap();
    assert_eq!(v.attachments.len(), 1);
    assert_eq!(v.attachments[0].name, "photo.png");
    until(|| e.message_view(&v.name).unwrap().unwrap().local_state == "failed").await;
    assert_eq!(e.message_view(&v.name).unwrap().unwrap().local_state, "failed");
    assert!(path.exists(), "the staged file waits for the retry");
    assert!(f.0.lock().unwrap().created.is_empty(), "no message without its image");

    // Retried: uploaded, attached, and the staged copy is gone.
    f.0.lock().unwrap().fail_upload = false;
    e.retry_send(&v.name).await.unwrap();
    until(|| e.message_view(&v.name).unwrap().is_none()).await;
    let w = f.0.lock().unwrap();
    assert_eq!(w.uploads.len(), 1);
    assert_eq!(
        (
            w.uploads[0].1.as_str(),
            w.uploads[0].2.as_str(),
            w.uploads[0].3.as_slice()
        ),
        ("photo.png", "image/png", b"\x89PNG fake".as_slice())
    );
    drop(w);
    let sent = e.get_messages("spaces/team", None, 50).unwrap();
    let mine = sent
        .iter()
        .find(|m| !m.attachments.is_empty())
        .expect("the sent message");
    assert_eq!(mine.attachments[0].kind, "image");
    assert!(!path.exists());
    std::fs::remove_dir_all(&dir).unwrap();
}

#[tokio::test]
async fn unsent_messages_keep_their_staged_files_from_the_sweep() {
    let f = world();
    let (e, _) = engine(&f).await;
    e.bootstrap().await.unwrap();
    let path = std::env::temp_dir().join(format!("kwek-keep-{}", uuid::Uuid::new_v4()));
    std::fs::write(&path, b"x").unwrap();
    assert!(e.staged_in_use().unwrap().is_empty());
    f.0.lock().unwrap().fail_upload = true;
    let file = Outgoing {
        path: path.clone(),
        name: "x.png".into(),
        content_type: "image/png".into(),
    };
    let v = e.send_message("spaces/team", None, "", vec![file]).await.unwrap();
    until(|| e.message_view(&v.name).unwrap().unwrap().local_state == "failed").await;
    assert_eq!(
        e.staged_in_use().unwrap(),
        vec![path.clone()],
        "a failed send still needs it"
    );
    f.0.lock().unwrap().fail_upload = false;
    e.retry_send(&v.name).await.unwrap();
    until(|| e.message_view(&v.name).unwrap().is_none()).await;
    assert!(e.staged_in_use().unwrap().is_empty(), "delivered: nothing left to keep");
}

#[tokio::test]
async fn a_retry_after_a_landed_send_does_not_upload_again() {
    let f = world();
    let (e, _) = engine(&f).await;
    e.bootstrap().await.unwrap();
    let path = std::env::temp_dir().join(format!("kwek-landed-{}", uuid::Uuid::new_v4()));
    std::fs::write(&path, b"x").unwrap();
    let file = Outgoing {
        path: path.clone(),
        name: "x.png".into(),
        content_type: "image/png".into(),
    };
    // The message lands at Google but the answer is lost: the row is marked failed.
    f.0.lock().unwrap().fail_create = true;
    let v = e.send_message("spaces/team", None, "", vec![file]).await.unwrap();
    until(|| e.message_view(&v.name).unwrap().unwrap().local_state == "failed").await;
    let client_id: String =
        e.db.with(|c| Ok(c.query_row("SELECT client_id FROM message WHERE name = ?1", [&v.name], |r| r.get(0))?))
            .unwrap();
    {
        let mut w = f.0.lock().unwrap();
        w.fail_create = false;
        w.messages.push(message(
            "spaces/team",
            &client_id,
            ME,
            "2024-02-01T00:00:00Z",
            "",
            "landed",
            false,
        ));
    }
    let uploads_before = f.0.lock().unwrap().uploads.len();
    e.retry_send(&v.name).await.unwrap();
    until(|| e.message_view(&v.name).unwrap().is_none()).await;
    assert!(
        e.message_view(&v.name).unwrap().is_none(),
        "the landed message replaced the row"
    );
    assert_eq!(f.0.lock().unwrap().uploads.len(), uploads_before, "no second upload");
    assert!(f.0.lock().unwrap().created.is_empty(), "no second message");
}

#[tokio::test]
async fn discarding_a_failed_send_removes_its_staged_files() {
    let f = world();
    let (e, _) = engine(&f).await;
    e.bootstrap().await.unwrap();
    let path = std::env::temp_dir().join(format!("kwek-staged-{}.png", uuid::Uuid::new_v4()));
    std::fs::write(&path, b"x").unwrap();
    f.0.lock().unwrap().fail_upload = true;
    let file = Outgoing {
        path: path.clone(),
        name: "x.png".into(),
        content_type: "image/png".into(),
    };
    let v = e.send_message("spaces/team", None, "look", vec![file]).await.unwrap();
    until(|| e.message_view(&v.name).unwrap().unwrap().local_state == "failed").await;
    e.discard_failed(&v.name).await.unwrap();
    assert!(e.message_view(&v.name).unwrap().is_none());
    assert!(!path.exists());
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

/// Polling refetches each space's newest page every minute; that must not undo history
/// already paged back.
#[tokio::test]
async fn repeated_initial_fetch_keeps_paged_history() {
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
    while e.load_older("spaces/team").await.unwrap() > 0 {}
    let before = e.db.with(|c| read::history_bounds(c, "spaces/team")).unwrap();
    assert!(before.1, "fully paged back");
    e.initial_fetch("spaces/team", ME, false).await.unwrap();
    let after = e.db.with(|c| read::history_bounds(c, "spaces/team")).unwrap();
    assert_eq!(after, before);
}

#[tokio::test]
async fn backfill_downloads_all_history() {
    let f = world();
    {
        let mut w = f.0.lock().unwrap();
        for i in 0..1200 {
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
    e.backfill_pass().await.unwrap();
    let left = e.db.with(|c| search::history_left(c)).unwrap();
    assert_eq!(left, 0, "every space complete");
    let cached: i64 =
        e.db.with(|c| {
            Ok(
                c.query_row("SELECT count(*) FROM message WHERE space = 'spaces/team'", [], |r| {
                    r.get(0)
                })?,
            )
        })
        .unwrap();
    assert_eq!(cached, 1203);
    // Nothing left to do: a second pass makes no calls that change anything.
    e.backfill_pass().await.unwrap();
}

#[tokio::test]
async fn backfill_gives_up_on_a_space_that_never_progresses() {
    let f = world();
    let (e, _) = engine(&f).await;
    e.bootstrap().await.unwrap();
    f.0.lock().unwrap().hollow = Some("spaces/team".into());
    e.db.with(|c| {
        c.execute(
            "UPDATE space SET history_complete = 0, low_water_us = NULL WHERE name = 'spaces/team'",
            [],
        )?;
        Ok(())
    })
    .unwrap();
    f.0.lock().unwrap().list_calls = 0;
    tokio::time::timeout(Duration::from_secs(10), e.backfill_pass())
        .await
        .expect("backfill must not page forever")
        .unwrap();
    assert_eq!(
        f.0.lock().unwrap().list_calls,
        3,
        "tried, then gave up after three stalls"
    );
    let (_, complete) = e.db.with(|c| read::history_bounds(c, "spaces/team")).unwrap();
    assert!(!complete, "giving up does not pretend the history is there");
}

#[tokio::test]
async fn filters_pick_lines_and_work_without_words() {
    let f = demo_world();
    f.0.lock().unwrap().messages.push(message(
        "spaces/dm",
        "mine",
        ME,
        "2024-01-05T09:06:00Z",
        "my dataset is ready too",
        "cmine",
        false,
    ));
    let (e, _) = engine(&f).await;
    e.bootstrap().await.unwrap();
    // The stretch holds both people's "dataset"; under from: only Ada's lines show.
    let r = e.search("from:users/ada dataset", 20, 0).unwrap();
    assert!(!r.hits.is_empty());
    for h in &r.hits {
        assert!(h.lines.iter().all(|l| l.sender.name == "users/ada"), "{:?}", h.lines);
    }
    // Filters alone list the matching stretches, newest first.
    let r = e.search("from:users/ada", 20, 0).unwrap();
    assert!(!r.hits.is_empty() && r.hits.iter().all(|h| h.words && !h.meaning));
    assert!(r.hits.windows(2).all(|w| w[0].last_us >= w[1].last_us));
    let r = e.search("in:spaces/dm", 20, 0).unwrap();
    assert!(!r.hits.is_empty() && r.hits.iter().all(|h| h.space == "spaces/dm"));
    assert!(e.search("in:spaces/nowhere", 20, 0).unwrap().hits.is_empty());
    assert!(
        e.search("from:a_a", 20, 0).unwrap().hits.is_empty(),
        "typed filters are literal, not LIKE patterns (a_a would match Ada)"
    );
    // Suggestions for from: are the people who wrote, most recent first.
    let people = e.list_senders().unwrap();
    assert_eq!(people.first().map(|u| u.name.as_str()), Some("users/ada"));
}

fn with_old_history(f: &Fake, n: usize) {
    let mut w = f.0.lock().unwrap();
    for i in 0..n {
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

#[tokio::test]
async fn page_back_leaves_a_mark_that_moved_meanwhile() {
    let f = world();
    with_old_history(&f, 300);
    let (e, _) = engine(&f).await;
    e.bootstrap().await.unwrap();
    let moved = t("2024-01-01T11:59:00Z");
    let db = e.db.clone();
    f.0.lock().unwrap().on_list = Some(Box::new(move || {
        db.with(|c| write::set_low_water(c, "spaces/team", moved)).unwrap();
    }));
    e.load_older("spaces/team").await.unwrap();
    let (low, complete) = e.db.with(|c| read::history_bounds(c, "spaces/team")).unwrap();
    assert_eq!(low, Some(moved), "a page fetched below the old mark proves nothing now");
    assert!(!complete);
}

#[tokio::test]
async fn page_back_keeps_nothing_for_a_space_that_went() {
    let f = world();
    with_old_history(&f, 300);
    let (e, _) = engine(&f).await;
    e.bootstrap().await.unwrap();
    let db = e.db.clone();
    f.0.lock().unwrap().on_list = Some(Box::new(move || {
        db.with(|c| write::delete_space(c, "spaces/team")).unwrap();
    }));
    e.load_older("spaces/team").await.unwrap();
    let left: i64 =
        e.db.with(|c| {
            Ok(
                c.query_row("SELECT count(*) FROM message WHERE space = 'spaces/team'", [], |r| {
                    r.get(0)
                })?,
            )
        })
        .unwrap();
    assert_eq!(left, 0);
}

/// Ada said "demo" in one message and "dataset" in the next; elsewhere "dataset" comes up
/// on its own. The stretch with both words ranks first.
fn demo_world() -> Fake {
    let f = world();
    {
        let mut w = f.0.lock().unwrap();
        let add = |w: &mut World, id: &str, at: &str, text: &str| {
            w.messages.push(message(
                "spaces/dm",
                id,
                "users/ada",
                at,
                text,
                &format!("c{id}"),
                false,
            ));
        };
        add(
            &mut w,
            "c1",
            "2024-01-05T09:00:00Z",
            "can we show the client a demo next week?",
        );
        add(
            &mut w,
            "c2",
            "2024-01-05T09:04:00Z",
            "I'll put together a dataset with fake customers",
        );
        add(
            &mut w,
            "c3",
            "2024-01-08T14:00:00Z",
            "the churn dataset has new columns",
        );
        add(&mut w, "c4", "2024-01-09T14:00:00Z", "another dataset question");
        add(&mut w, "c5", "2024-01-10T14:00:00Z", "the demo went well");
    }
    f
}

#[tokio::test]
async fn search_finds_words_spread_over_a_stretch() {
    let f = demo_world();
    let (e, _) = engine(&f).await;
    e.bootstrap().await.unwrap();
    let r = e.search("demo dataset", 20, 0).unwrap();
    let top = &r.hits[0];
    assert!(top.words && !top.meaning);
    assert_eq!(top.anchor, "spaces/dm/messages/c1");
    assert_eq!(top.lines.len(), 2, "both messages with a match are shown");
    assert!(top.lines[1].parts.iter().any(|p| p.hit && p.text == "dataset"));
    assert_eq!(top.thread, None);
    assert_eq!(top.space_name, "Ada Lovelace");
    // Stretches with one of the words still come, below.
    assert_eq!(r.hits.len(), 4);
    // Prefix match on every longer word: "data" finds "dataset".
    assert!(e.search("data demo", 20, 0).unwrap().hits[0].anchor == "spaces/dm/messages/c1");
}

#[tokio::test]
async fn search_filters_and_literal_syntax() {
    let f = world();
    let (e, _) = engine(&f).await;
    e.bootstrap().await.unwrap();
    let r = e.search("deploy", 20, 0).unwrap();
    assert_eq!(r.hits.len(), 1);
    assert_eq!(r.hits[0].space_name, "Team");
    assert!(r.hits[0].lines[0].parts.iter().any(|p| p.hit && p.text == "deploy"));
    assert_eq!(e.search("depl", 20, 0).unwrap().hits.len(), 1);
    assert_eq!(e.search("from:ada psst", 20, 0).unwrap().hits.len(), 1);
    assert_eq!(e.search("from:nobody psst", 20, 0).unwrap().hits.len(), 0);
    assert_eq!(e.search("in:team psst", 20, 0).unwrap().hits.len(), 0);
    assert_eq!(e.search("after:2024-01-02 deploy", 20, 0).unwrap().hits.len(), 0);
    assert!(e.search("NEAR( \"x OR", 20, 0).is_ok());
    // A reply is found in its thread's stretch, which opens the thread.
    let r = e.search("reply", 20, 0).unwrap();
    assert_eq!(r.hits[0].thread.as_deref(), Some("spaces/team/threads/t1"));
    assert_eq!(r.hits[0].anchor, "spaces/team/messages/2");
}

#[tokio::test]
async fn search_follows_edits_and_deletes() {
    let f = world();
    let (e, _) = engine(&f).await;
    e.bootstrap().await.unwrap();
    assert_eq!(e.search("deploy", 20, 0).unwrap().hits.len(), 1);
    e.db.with(|c| {
        apply(
            c,
            &[Event::MessageDeleted {
                name: "spaces/team/messages/3".into(),
                delete_time: Some("2024-01-02T00:00:00Z".into()),
            }],
            ME,
        )
        .map(|_| ())
    })
    .unwrap();
    assert_eq!(e.search("deploy", 20, 0).unwrap().hits.len(), 0);
}

/// Words map to concepts; texts sharing a concept are close in meaning.
struct Concepts;

impl Concepts {
    fn vec(text: &str) -> Vec<f32> {
        let concepts: [&[&str]; 3] = [
            &["demo", "sample", "showcase", "pitch"],
            &["lunch", "food", "pizza"],
            &["invoice", "billing"],
        ];
        let lower = text.to_lowercase();
        let mut v: Vec<f32> = concepts
            .iter()
            .map(|words| words.iter().filter(|w| lower.contains(*w)).count() as f32)
            .collect();
        v.push(0.1);
        search::embed::normalise(v)
    }
}

impl search::embed::Embedder for Concepts {
    fn model(&self) -> &str {
        "concepts"
    }
    fn available(&self) -> bool {
        true
    }
    fn ready(&self) -> bool {
        true
    }
    fn load(&self) -> Result<()> {
        Ok(())
    }
    fn passages(&self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
        if texts.iter().any(|t| t.contains("unembeddable")) {
            return Err(Error::Other("model failed".into()));
        }
        Ok(texts.iter().map(|t| Concepts::vec(t)).collect())
    }
    fn query(&self, text: &str) -> Result<Vec<f32>> {
        Ok(Concepts::vec(text))
    }
}

/// A model that fails on everything.
struct Broken;

impl search::embed::Embedder for Broken {
    fn model(&self) -> &str {
        "broken"
    }
    fn available(&self) -> bool {
        true
    }
    fn ready(&self) -> bool {
        true
    }
    fn load(&self) -> Result<()> {
        Ok(())
    }
    fn passages(&self, _: &[String]) -> Result<Vec<Vec<f32>>> {
        Err(Error::Other("model failed".into()))
    }
    fn query(&self, _: &str) -> Result<Vec<f32>> {
        Err(Error::Other("model failed".into()))
    }
}

#[tokio::test]
async fn a_broken_model_stores_nothing_and_says_so() {
    let f = demo_world();
    let e = Engine::new(
        Db::open_in_memory().unwrap(),
        Arc::new(f.clone()),
        Arc::new(FakePeople),
        None,
        None,
        Arc::new(Rec::default()),
        None,
        Some(Arc::new(Broken)),
    );
    e.bootstrap().await.unwrap();
    assert!(e.index_pass().await.is_err());
    let r = e.search("demo", 20, 0).unwrap();
    assert_eq!(
        r.meaning.indexed, 0,
        "nothing marked as indexed, so the next pass retries"
    );
    assert!(r.meaning.total > 0);
    assert!(!r.hits.is_empty(), "keyword search still works");
}

/// A working model that fails on every real stretch.
struct Picky;

impl search::embed::Embedder for Picky {
    fn model(&self) -> &str {
        "picky"
    }
    fn available(&self) -> bool {
        true
    }
    fn ready(&self) -> bool {
        true
    }
    fn load(&self) -> Result<()> {
        Ok(())
    }
    fn passages(&self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
        if texts.iter().all(|t| t.contains("probe")) {
            Ok(texts.iter().map(|t| Concepts::vec(t)).collect())
        } else {
            Err(Error::Other("cannot read this".into()))
        }
    }
    fn query(&self, text: &str) -> Result<Vec<f32>> {
        Ok(Concepts::vec(text))
    }
}

#[tokio::test]
async fn a_batch_of_bad_stretches_does_not_block_the_rest() {
    let f = demo_world();
    let e = Engine::new(
        Db::open_in_memory().unwrap(),
        Arc::new(f.clone()),
        Arc::new(FakePeople),
        None,
        None,
        Arc::new(Rec::default()),
        None,
        Some(Arc::new(Picky)),
    );
    e.bootstrap().await.unwrap();
    e.index_pass().await.unwrap();
    let r = e.search("demo", 20, 0).unwrap();
    assert_eq!(r.meaning.indexed, r.meaning.total, "stored empty, so indexing moves on");
}

#[tokio::test]
async fn a_stretch_the_model_fails_on_does_not_stop_indexing() {
    let f = demo_world();
    f.0.lock().unwrap().messages.push(message(
        "spaces/dm",
        "bad",
        "users/ada",
        "2024-03-01T09:00:00Z",
        "this one is unembeddable by the model",
        "cbad",
        false,
    ));
    let e = Engine::new(
        Db::open_in_memory().unwrap(),
        Arc::new(f.clone()),
        Arc::new(FakePeople),
        None,
        None,
        Arc::new(Rec::default()),
        None,
        Some(Arc::new(Concepts)),
    );
    e.bootstrap().await.unwrap();
    e.index_pass().await.unwrap();
    let r = e.search("showcase", 20, 0).unwrap();
    assert_eq!(
        r.meaning.indexed, r.meaning.total,
        "every stretch got a vector, empty or not"
    );
    assert!(r.hits.iter().any(|h| h.anchor == "spaces/dm/messages/c1"));
}

#[tokio::test]
async fn search_by_meaning_finds_other_words() {
    let f = demo_world();
    let rec = Arc::new(Rec::default());
    let e = Engine::new(
        Db::open_in_memory().unwrap(),
        Arc::new(f.clone()),
        Arc::new(FakePeople),
        None,
        None,
        rec,
        None,
        Some(Arc::new(Concepts)),
    );
    e.bootstrap().await.unwrap();
    e.index_pass().await.unwrap();
    let r = e.search("showcase", 20, 0).unwrap();
    assert!(r.meaning.ready && r.meaning.indexed == r.meaning.total && r.meaning.total > 0);
    let names: Vec<&str> = r.hits.iter().map(|h| h.anchor.as_str()).collect();
    assert!(names.contains(&"spaces/dm/messages/c1"), "{names:?}");
    assert!(r.hits.iter().all(|h| h.meaning && !h.words));
    // Keyword hits and meaning hits merge; a stretch with both ranks first.
    let r = e.search("demo", 20, 0).unwrap();
    assert!(r.hits[0].words && r.hits[0].meaning);

    // An edit re-indexes: the old vector goes, the new text gets one.
    e.db.with(|c| {
        let mut m = message(
            "spaces/dm",
            "c5",
            "users/ada",
            "2024-01-10T14:00:00Z",
            "pizza on friday",
            "cc5",
            false,
        );
        m.last_update_time = Some("2024-01-11T00:00:00Z".into());
        apply(c, &[Event::MessageUpserted(m)], ME).map(|_| ())
    })
    .unwrap();
    e.index_pass().await.unwrap();
    let r = e.search("food", 20, 0).unwrap();
    assert_eq!(r.hits[0].anchor, "spaces/dm/messages/c5");
    let orphans: i64 =
        e.db.with(|c| {
            Ok(c.query_row(
                "SELECT count(*) FROM embedding WHERE hash NOT IN (SELECT hash FROM chunk)",
                [],
                |r| r.get(0),
            )?)
        })
        .unwrap();
    assert_eq!(orphans, 0);
}
