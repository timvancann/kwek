//! The applier: events in, local state out, inside the caller's transaction. Live Pub/Sub,
//! `spaceEvents.list` and REST list calls all go through here.

use std::collections::BTreeSet;

use rusqlite::Connection;

use crate::db::read;
use crate::db::write::{self, Upsert};
use crate::error::Result;
use crate::events::Event;
use crate::ts;

#[derive(Debug, Default)]
pub struct Applied {
    /// Something changed that the sidebar shows (read state, membership, spaces).
    pub changed: bool,
    /// (space, message name) upserted or changed.
    pub upserted: Vec<(String, String)>,
    /// Names of messages inserted for the first time (notification candidates).
    pub inserted: Vec<String>,
    pub deleted: Vec<(String, String)>,
    /// Messages whose reaction summary changed; the engine refetches them.
    pub stale_reactions: BTreeSet<String>,
    /// Spaces whose read state arrived without a time; the engine refetches it.
    pub stale_read: BTreeSet<String>,
    /// Spaces the user joined that are not known locally yet.
    pub joined_spaces: BTreeSet<String>,
}

impl Applied {
    pub fn is_empty(&self) -> bool {
        !self.changed
            && self.upserted.is_empty()
            && self.deleted.is_empty()
            && self.stale_reactions.is_empty()
            && self.stale_read.is_empty()
            && self.joined_spaces.is_empty()
    }
}

/// Applies events to spaces the user is known to belong to (the `space` table, filled from
/// the user's own `spaces.list`). Events about any other space are dropped: the Pub/Sub
/// topic is shared, so this is a second line of defence behind the subscription filter.
/// The one exception is a membership event saying the user joined a space, which is how
/// new spaces are discovered.
pub fn apply(c: &Connection, events: &[Event], me: &str) -> Result<Applied> {
    let mut a = Applied::default();
    let mut known_cache: std::collections::HashMap<String, bool> = std::collections::HashMap::new();
    let mut known = |space: &str| -> Result<bool> {
        if let Some(k) = known_cache.get(space) {
            return Ok(*k);
        }
        let k = read::space_exists(c, space)?;
        known_cache.insert(space.to_owned(), k);
        Ok(k)
    };
    for ev in events {
        let space = event_space(ev);
        let joins_me = matches!(ev, Event::MembershipUpserted(mb)
            if mb.member.as_ref().is_some_and(|u| u.name == me)
                && matches!(mb.state.as_deref(), None | Some("JOINED")));
        if let Some(s) = &space
            && !joins_me
            && !known(s)?
        {
            tracing::debug!(space = %s, "dropping event for a space that is not ours");
            continue;
        }
        match ev {
            Event::MessageUpserted(m) => {
                let space = m.space_name();
                match write::upsert_message(c, m, me)? {
                    Upsert::Ignored => {}
                    outcome => {
                        if let Some(t) = m.create_time.as_deref().and_then(ts::parse) {
                            write::bump_space_activity(c, &space, t)?;
                        }
                        if outcome == Upsert::Inserted {
                            a.inserted.push(m.name.clone());
                        }
                        a.upserted.push((space, m.name.clone()));
                    }
                }
            }
            Event::MessageDeleted { name, delete_time } => {
                // Without a time the delete still has to be final; 1 µs sorts before any edit.
                let at = delete_time.as_deref().and_then(ts::parse).unwrap_or(1);
                write::mark_deleted(c, name, at)?;
                a.deleted.push((crate::chat::space_of(name), name.clone()));
            }
            Event::ReactionCreated(r) => {
                let msg = r.message_name();
                if r.user.as_ref().is_some_and(|u| u.name == me) {
                    write::set_my_reaction(c, &msg, &r.emoji.key(), &r.name)?;
                }
                a.stale_reactions.insert(msg);
            }
            Event::ReactionDeleted(r) => {
                let msg = r.message_name();
                write::clear_my_reaction_by_name(c, &r.name)?;
                a.stale_reactions.insert(msg);
            }
            Event::MembershipUpserted(mb) => {
                let space = mb.space_name();
                if let Some(member) = &mb.member {
                    let joined = matches!(mb.state.as_deref(), None | Some("JOINED"));
                    if joined {
                        write::upsert_membership(c, &space, member, mb.role.as_deref())?;
                        if member.name == me && !read::space_exists(c, &space)? {
                            a.joined_spaces.insert(space.clone());
                        }
                    } else {
                        write::delete_membership(c, &space, &member.name)?;
                    }
                }
                a.changed = true;
            }
            Event::MembershipDeleted(mb) => {
                let space = mb.space_name();
                if let Some(member) = &mb.member {
                    write::delete_membership(c, &space, &member.name)?;
                    if member.name == me {
                        write::delete_space(c, &space)?;
                    }
                }
                a.changed = true;
            }
            Event::SpaceUpdated(s) => {
                if !s.name.is_empty() {
                    write::upsert_space(c, s)?;
                    a.changed = true;
                }
            }
            Event::SpaceDeleted { name } => {
                write::delete_space(c, name)?;
                a.changed = true;
            }
            // A read state without a time (e.g. an event without the resource) must never
            // wipe the known mark: that would zero the unread counts. Refetch it instead.
            Event::SpaceReadState(rs) => {
                if let Some(space) = rs.space_name() {
                    match rs.last_read_time.as_deref().and_then(ts::parse) {
                        Some(t) => {
                            write::set_space_read(c, &space, Some(t))?;
                            a.changed = true;
                        }
                        None => {
                            a.stale_read.insert(space);
                        }
                    }
                }
            }
            Event::ThreadReadState(tr) => {
                if let (Some(thread), Some(t)) = (tr.thread_name(), tr.last_read_time.as_deref().and_then(ts::parse)) {
                    write::set_thread_read(c, &thread, Some(t))?;
                    a.changed = true;
                }
            }
            Event::Lifecycle { .. } | Event::Unknown(_) => {}
        }
    }
    Ok(a)
}

/// The space an event is about, if it is about one.
pub(crate) fn event_space(ev: &Event) -> Option<String> {
    use crate::chat::space_of;
    match ev {
        Event::MessageUpserted(m) => Some(m.space_name()),
        Event::MessageDeleted { name, .. } => Some(space_of(name)),
        Event::ReactionCreated(r) | Event::ReactionDeleted(r) => Some(space_of(&r.name)),
        Event::MembershipUpserted(mb) | Event::MembershipDeleted(mb) => Some(mb.space_name()),
        Event::SpaceUpdated(s) => Some(s.name.clone()),
        Event::SpaceDeleted { name } => Some(name.clone()),
        Event::SpaceReadState(rs) => rs.space_name(),
        Event::ThreadReadState(tr) => tr.thread_name().map(|t| space_of(&t)),
        Event::Lifecycle { .. } | Event::Unknown(_) => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chat::{Message, SpaceRef, Thread, User};
    use crate::db::Db;
    use proptest::prelude::*;

    const ME: &str = "users/me";

    fn msg(id: u32, text: &str, created: &str, updated: Option<&str>) -> Message {
        Message {
            name: format!("spaces/S/messages/{id}"),
            sender: Some(User {
                name: "users/other".into(),
                display_name: Some("Other".into()),
                ..Default::default()
            }),
            create_time: Some(created.into()),
            last_update_time: updated.map(str::to_owned),
            text: Some(text.into()),
            formatted_text: Some(text.into()),
            thread: Some(Thread {
                name: format!("spaces/S/threads/{id}"),
            }),
            space: Some(SpaceRef {
                name: "spaces/S".into(),
            }),
            ..Default::default()
        }
    }

    /// Everything observable about messages, excluding the rowid.
    /// (name, update_us, delete_us, text, local_state, mentions_me)
    type Row = (String, i64, Option<i64>, Option<String>, String, bool);

    fn snapshot(db: &Db) -> Vec<Row> {
        db.with(|c| {
            let mut st = c.prepare(
                "SELECT name, update_us, delete_us, text, local_state, mentions_me FROM message ORDER BY name",
            )?;
            let rows = st
                .query_map([], |r| {
                    Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?))
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            Ok(rows)
        })
        .unwrap()
    }

    fn fts_hits(db: &Db, q: &str) -> i64 {
        db.with(|c| {
            Ok(c.query_row(
                "SELECT count(*) FROM message_fts WHERE message_fts MATCH ?1",
                [q],
                |r| r.get(0),
            )?)
        })
        .unwrap()
    }

    fn db_with_space() -> Db {
        let db = Db::open_in_memory().unwrap();
        db.with(|c| {
            write::upsert_space(
                c,
                &crate::chat::Space {
                    name: "spaces/S".into(),
                    ..Default::default()
                },
            )
        })
        .unwrap();
        db
    }

    fn run(events: &[Event]) -> Db {
        let db = db_with_space();
        db.with(|c| {
            let tx = c.transaction()?;
            apply(&tx, events, ME)?;
            tx.commit()?;
            Ok(())
        })
        .unwrap();
        db
    }

    /// A small universe of events about three messages: creates, edits, deletes.
    fn universe() -> Vec<Event> {
        vec![
            Event::MessageUpserted(msg(1, "first", "2024-01-01T10:00:00Z", None)),
            Event::MessageUpserted(msg(
                1,
                "first edited",
                "2024-01-01T10:00:00Z",
                Some("2024-01-01T10:05:00Z"),
            )),
            Event::MessageUpserted(msg(
                1,
                "first edited again",
                "2024-01-01T10:00:00Z",
                Some("2024-01-01T10:06:00.5Z"),
            )),
            Event::MessageUpserted(msg(2, "second", "2024-01-01T11:00:00Z", None)),
            Event::MessageDeleted {
                name: "spaces/S/messages/2".into(),
                delete_time: Some("2024-01-01T11:30:00Z".into()),
            },
            Event::MessageUpserted(msg(
                2,
                "second edited",
                "2024-01-01T11:00:00Z",
                Some("2024-01-01T11:10:00Z"),
            )),
            Event::MessageUpserted(msg(3, "third", "2024-01-01T12:00:00Z", None)),
            Event::MessageDeleted {
                name: "spaces/S/messages/4".into(),
                delete_time: Some("2024-01-01T13:00:00Z".into()),
            },
            Event::MessageUpserted(msg(4, "fourth", "2024-01-01T12:30:00Z", None)),
        ]
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(200))]
        #[test]
        fn any_order_and_duplication_converges(
            order in Just((0..9usize).collect::<Vec<_>>()).prop_shuffle(),
            dups in proptest::collection::vec(0..9usize, 0..6),
        ) {
            let u = universe();
            let reference = snapshot(&run(&u));
            let mut evs: Vec<Event> = order.iter().map(|&i| u[i].clone()).collect();
            for d in dups {
                evs.push(u[d].clone());
            }
            let db = run(&evs);
            prop_assert_eq!(snapshot(&db), reference);
            // The search index agrees with the table, whatever the order.
            prop_assert_eq!(fts_hits(&db, "again"), 1);
            prop_assert_eq!(fts_hits(&db, "second"), 0);
            prop_assert_eq!(fts_hits(&db, "fourth"), 0);
        }
    }

    #[test]
    fn delete_is_final_and_newest_edit_wins() {
        let db = run(&universe());
        let snap = snapshot(&db);
        let get = |n: u32| {
            snap.iter()
                .find(|r| r.0 == format!("spaces/S/messages/{n}"))
                .unwrap()
                .clone()
        };
        assert_eq!(get(1).3.as_deref(), Some("first edited again"));
        assert!(get(2).2.is_some() && get(2).3.is_none());
        assert_eq!(get(4).4, "synced");
        assert!(get(4).2.is_some());
    }

    #[test]
    fn optimistic_send_dedups_by_client_id_in_both_orders() {
        for echo_first in [false, true] {
            let db = db_with_space();
            db.with(|c| {
                write::insert_pending(c, "spaces/S", None, "hello", "client-1", ME, 5)?;
                Ok(())
            })
            .unwrap();
            let mut server = msg(9, "hello", "2024-01-01T10:00:00Z", None);
            server.sender = Some(User {
                name: ME.into(),
                ..Default::default()
            });
            server.client_assigned_message_id = Some("client-1".into());
            let evs = vec![Event::MessageUpserted(server)];
            db.with(|c| {
                if echo_first {
                    // The echo lands as its own row first (e.g. pending row was renamed by
                    // another path), then the pending row must be dropped, not duplicated.
                    c.execute(
                        "UPDATE message SET client_id = NULL WHERE name = 'pending:client-1'",
                        [],
                    )?;
                    apply(c, &evs, ME)?;
                    c.execute(
                        "UPDATE message SET client_id = 'client-1' WHERE name = 'pending:client-1'",
                        [],
                    )?;
                }
                apply(c, &evs, ME)?;
                Ok(())
            })
            .unwrap();
            let snap = snapshot(&db);
            assert_eq!(snap.len(), 1, "echo_first={echo_first}: {snap:?}");
            assert_eq!(snap[0].0, "spaces/S/messages/9");
            assert_eq!(snap[0].4, "synced");
            assert_eq!(fts_hits(&db, "hello"), 1);
        }
    }

    #[test]
    fn upsert_keeps_rowid_so_fts_stays_consistent() {
        let db = run(&[Event::MessageUpserted(msg(1, "alpha", "2024-01-01T10:00:00Z", None))]);
        let id1: i64 = db
            .with(|c| Ok(c.query_row("SELECT id FROM message", [], |r| r.get(0))?))
            .unwrap();
        db.with(|c| {
            apply(
                c,
                &[Event::MessageUpserted(msg(
                    1,
                    "beta",
                    "2024-01-01T10:00:00Z",
                    Some("2024-01-01T10:01:00Z"),
                ))],
                ME,
            )?;
            Ok(())
        })
        .unwrap();
        let id2: i64 = db
            .with(|c| Ok(c.query_row("SELECT id FROM message", [], |r| r.get(0))?))
            .unwrap();
        assert_eq!(id1, id2);
        assert_eq!(fts_hits(&db, "alpha"), 0);
        assert_eq!(fts_hits(&db, "beta"), 1);
    }

    #[test]
    fn events_for_foreign_spaces_are_dropped_except_my_join() {
        let mut foreign = msg(1, "not yours", "2024-01-01T10:00:00Z", None);
        foreign.name = "spaces/OTHER/messages/1".into();
        foreign.space = Some(SpaceRef {
            name: "spaces/OTHER".into(),
        });
        let join = Event::MembershipUpserted(crate::chat::Membership {
            name: "spaces/NEW/members/me".into(),
            state: Some("JOINED".into()),
            member: Some(User {
                name: ME.into(),
                ..Default::default()
            }),
            ..Default::default()
        });
        let someone_else = Event::MembershipUpserted(crate::chat::Membership {
            name: "spaces/OTHER/members/x".into(),
            state: Some("JOINED".into()),
            member: Some(User {
                name: "users/x".into(),
                ..Default::default()
            }),
            ..Default::default()
        });
        let db = db_with_space();
        let a = db
            .with(|c| apply(c, &[Event::MessageUpserted(foreign), join, someone_else], ME))
            .unwrap();
        assert!(snapshot(&db).is_empty());
        assert_eq!(
            a.joined_spaces.into_iter().collect::<Vec<_>>(),
            vec!["spaces/NEW".to_string()]
        );
        let members: i64 = db
            .with(|c| {
                Ok(c.query_row(
                    "SELECT count(*) FROM membership WHERE space = 'spaces/OTHER'",
                    [],
                    |r| r.get(0),
                )?)
            })
            .unwrap();
        assert_eq!(members, 0);
    }

    #[test]
    fn mentions_detected_from_formatted_text() {
        let mut m = msg(1, "hi <users/me>", "2024-01-01T10:00:00Z", None);
        m.formatted_text = Some("hi <users/me>".into());
        let db = run(&[Event::MessageUpserted(m)]);
        assert!(snapshot(&db)[0].5);
    }
}
