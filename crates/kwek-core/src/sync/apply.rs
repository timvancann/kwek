//! The applier: events in, local state out, inside the caller's transaction. Live Pub/Sub,
//! `spaceEvents.list` and REST list calls all go through here.

use std::collections::BTreeSet;

use rusqlite::Connection;

use crate::db::write::{self, Upsert};
use crate::error::Result;
use crate::events::Event;
use crate::ts;

#[derive(Debug, Default)]
pub struct Applied {
    pub spaces: BTreeSet<String>,
    /// (space, message name) upserted or changed.
    pub upserted: Vec<(String, String)>,
    /// Names of messages inserted for the first time (notification candidates).
    pub inserted: Vec<String>,
    pub deleted: Vec<(String, String)>,
    pub read: Vec<(String, Option<String>)>,
    /// Messages whose reaction summary changed; the engine refetches them.
    pub stale_reactions: BTreeSet<String>,
    /// Spaces the user joined that are not known locally yet.
    pub joined_spaces: BTreeSet<String>,
    pub removed_spaces: BTreeSet<String>,
}

impl Applied {
    pub fn is_empty(&self) -> bool {
        self.spaces.is_empty()
            && self.upserted.is_empty()
            && self.deleted.is_empty()
            && self.read.is_empty()
            && self.stale_reactions.is_empty()
            && self.joined_spaces.is_empty()
            && self.removed_spaces.is_empty()
    }
}

pub fn apply(c: &Connection, events: &[Event], me: &str) -> Result<Applied> {
    let mut a = Applied::default();
    for ev in events {
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
                        a.upserted.push((space.clone(), m.name.clone()));
                        a.spaces.insert(space);
                    }
                }
            }
            Event::MessageDeleted { name, delete_time } => {
                // Without a time the delete still has to be final; 1 µs sorts before any edit.
                let at = delete_time.as_deref().and_then(ts::parse).unwrap_or(1);
                write::mark_deleted(c, name, at)?;
                let space = crate::chat::space_of(name);
                a.deleted.push((space.clone(), name.clone()));
                a.spaces.insert(space);
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
                        if member.name == me && !space_exists(c, &space)? {
                            a.joined_spaces.insert(space.clone());
                        }
                    } else {
                        write::delete_membership(c, &space, &member.name)?;
                    }
                }
                a.spaces.insert(space);
            }
            Event::MembershipDeleted(mb) => {
                let space = mb.space_name();
                if let Some(member) = &mb.member {
                    write::delete_membership(c, &space, &member.name)?;
                    if member.name == me {
                        write::delete_space(c, &space)?;
                        a.removed_spaces.insert(space.clone());
                    }
                }
                a.spaces.insert(space);
            }
            Event::SpaceUpdated(s) => {
                if !s.name.is_empty() {
                    write::upsert_space(c, s)?;
                    a.spaces.insert(s.name.clone());
                }
            }
            Event::SpaceDeleted { name } => {
                write::delete_space(c, name)?;
                a.removed_spaces.insert(name.clone());
            }
            Event::SpaceReadState(rs) => {
                if let Some(space) = rs.space_name() {
                    write::set_space_read(c, &space, rs.last_read_time.as_deref().and_then(ts::parse))?;
                    a.read.push((space.clone(), None));
                    a.spaces.insert(space);
                }
            }
            Event::ThreadReadState(tr) => {
                if let Some(thread) = tr.thread_name() {
                    write::set_thread_read(c, &thread, tr.last_read_time.as_deref().and_then(ts::parse))?;
                    let space = crate::chat::space_of(&thread);
                    a.read.push((space.clone(), Some(thread)));
                    a.spaces.insert(space);
                }
            }
            Event::Lifecycle { .. } | Event::Unknown(_) => {}
        }
    }
    Ok(a)
}

fn space_exists(c: &Connection, space: &str) -> Result<bool> {
    use rusqlite::OptionalExtension;
    Ok(c.query_row("SELECT 1 FROM space WHERE name = ?1", [space], |_| Ok(()))
        .optional()?
        .is_some())
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
            thread: Some(Thread { name: format!("spaces/S/threads/{id}") }),
            space: Some(SpaceRef { name: "spaces/S".into() }),
            ..Default::default()
        }
    }

    /// Everything observable about messages, excluding the rowid.
    fn snapshot(db: &Db) -> Vec<(String, i64, Option<i64>, Option<String>, String, bool)> {
        db.with(|c| {
            let mut st = c.prepare(
                "SELECT name, update_us, delete_us, text, local_state, mentions_me FROM message ORDER BY name",
            )?;
            let rows = st
                .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?)))?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            Ok(rows)
        })
        .unwrap()
    }

    fn fts_hits(db: &Db, q: &str) -> i64 {
        db.with(|c| Ok(c.query_row("SELECT count(*) FROM message_fts WHERE message_fts MATCH ?1", [q], |r| r.get(0))?))
            .unwrap()
    }

    fn run(events: &[Event]) -> Db {
        let db = Db::open_in_memory().unwrap();
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
            Event::MessageUpserted(msg(1, "first edited", "2024-01-01T10:00:00Z", Some("2024-01-01T10:05:00Z"))),
            Event::MessageUpserted(msg(1, "first edited again", "2024-01-01T10:00:00Z", Some("2024-01-01T10:06:00.5Z"))),
            Event::MessageUpserted(msg(2, "second", "2024-01-01T11:00:00Z", None)),
            Event::MessageDeleted { name: "spaces/S/messages/2".into(), delete_time: Some("2024-01-01T11:30:00Z".into()) },
            Event::MessageUpserted(msg(2, "second edited", "2024-01-01T11:00:00Z", Some("2024-01-01T11:10:00Z"))),
            Event::MessageUpserted(msg(3, "third", "2024-01-01T12:00:00Z", None)),
            Event::MessageDeleted { name: "spaces/S/messages/4".into(), delete_time: Some("2024-01-01T13:00:00Z".into()) },
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
        let get = |n: u32| snap.iter().find(|r| r.0 == format!("spaces/S/messages/{n}")).unwrap().clone();
        assert_eq!(get(1).3.as_deref(), Some("first edited again"));
        assert!(get(2).2.is_some() && get(2).3.is_none());
        assert_eq!(get(4).4, "synced");
        assert!(get(4).2.is_some());
    }

    #[test]
    fn optimistic_send_dedups_by_client_id_in_both_orders() {
        for echo_first in [false, true] {
            let db = Db::open_in_memory().unwrap();
            db.with(|c| {
                write::insert_pending(c, "spaces/S", None, "hello", "client-1", ME, 5)?;
                Ok(())
            })
            .unwrap();
            let mut server = msg(9, "hello", "2024-01-01T10:00:00Z", None);
            server.sender = Some(User { name: ME.into(), ..Default::default() });
            server.client_assigned_message_id = Some("client-1".into());
            let evs = vec![Event::MessageUpserted(server)];
            db.with(|c| {
                if echo_first {
                    // The echo lands as its own row first (e.g. pending row was renamed by
                    // another path), then the pending row must be dropped, not duplicated.
                    c.execute("UPDATE message SET client_id = NULL WHERE name = 'pending:client-1'", [])?;
                    apply(c, &evs, ME)?;
                    c.execute("UPDATE message SET client_id = 'client-1' WHERE name = 'pending:client-1'", [])?;
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
        let id1: i64 = db.with(|c| Ok(c.query_row("SELECT id FROM message", [], |r| r.get(0))?)).unwrap();
        db.with(|c| {
            apply(c, &[Event::MessageUpserted(msg(1, "beta", "2024-01-01T10:00:00Z", Some("2024-01-01T10:01:00Z")))], ME)?;
            Ok(())
        })
        .unwrap();
        let id2: i64 = db.with(|c| Ok(c.query_row("SELECT id FROM message", [], |r| r.get(0))?)).unwrap();
        assert_eq!(id1, id2);
        assert_eq!(fts_hits(&db, "alpha"), 0);
        assert_eq!(fts_hits(&db, "beta"), 1);
    }

    #[test]
    fn mentions_detected_from_formatted_text() {
        let mut m = msg(1, "hi <users/me>", "2024-01-01T10:00:00Z", None);
        m.formatted_text = Some("hi <users/me>".into());
        let db = run(&[Event::MessageUpserted(m)]);
        assert!(snapshot(&db)[0].5);
    }
}
