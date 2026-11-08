//! Idempotent writes keyed by resource name. Every function takes a connection that is
//! normally inside a transaction opened by the caller; nothing here commits.
//!
//! Message upsert rules (the applier's convergence depends on them):
//! - An incoming version older than the stored one is ignored (`update_us` ordering).
//! - Once deleted, a message stays deleted: later versions refresh metadata but never text.
//! - A delete for an unknown message leaves a `ghost` row so a late "created" cannot revive it.
//! - Optimistic rows are matched by `clientAssignedMessageId`, never by name.

use rusqlite::{Connection, OptionalExtension, params};

use crate::chat::{Message, Space, User, space_of};
use crate::error::Result;
use crate::people::Person;
use crate::ts::{self, Micros};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Upsert {
    Inserted,
    Updated,
    Ignored,
}

fn json<T: serde::Serialize>(v: &Option<T>) -> Option<String> {
    v.as_ref().and_then(|v| serde_json::to_string(v).ok())
}

/// Whether the message mentions `me` (or @all), from annotations or formattedText.
pub fn mentions(m: &Message, me: &str) -> bool {
    if me.is_empty() {
        return false;
    }
    let by_annotation = m.annotations.iter().flatten().any(|a| {
        a.get("type").and_then(|t| t.as_str()) == Some("USER_MENTION")
            && matches!(
                a.pointer("/userMention/user/name").and_then(|n| n.as_str()),
                Some(n) if n == me || n == "users/all"
            )
    });
    let by_text = m.formatted_text.as_deref().is_some_and(|t| {
        // Both the plain `<users/ID>` form and Chat's Markdown export
        // `<chat-user data-user="users/ID">`.
        [me, "users/all"]
            .iter()
            .any(|u| t.contains(&format!("<{u}>")) || t.contains(&format!("data-user=\"{u}\"")))
    });
    by_annotation || by_text
}

pub fn upsert_message(c: &Connection, m: &Message, me: &str) -> Result<Upsert> {
    let Some(create_us) = m.create_time.as_deref().and_then(ts::parse) else {
        tracing::debug!(name = %m.name, "message without createTime ignored");
        return Ok(Upsert::Ignored);
    };
    if m.name.is_empty() {
        return Ok(Upsert::Ignored);
    }
    let update_us = m.last_update_time.as_deref().and_then(ts::parse).unwrap_or(create_us);
    let incoming_delete = m.delete_time.as_deref().and_then(ts::parse);

    if let Some(cid) = m.client_assigned_message_id.as_deref() {
        adopt_pending(c, cid, &m.name)?;
    }

    let existing: Option<(Micros, Option<Micros>, String)> = c
        .query_row(
            "SELECT update_us, delete_us, local_state FROM message WHERE name = ?1",
            [&m.name],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .optional()?;

    let mut delete_us = incoming_delete;
    if let Some((ex_update, ex_delete, state)) = &existing {
        let authoritative = state == "synced";
        if authoritative && update_us < *ex_update {
            return Ok(Upsert::Ignored);
        }
        delete_us = match (ex_delete, incoming_delete) {
            (Some(a), Some(b)) => Some((*a).min(b)),
            (a, b) => a.or(b),
        };
    }
    let deleted = delete_us.is_some();

    let space = m
        .space
        .as_ref()
        .map(|s| s.name.clone())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| space_of(&m.name));
    let sender = m.sender.as_ref().map(|s| s.name.clone()).unwrap_or_default();
    if let Some(u) = &m.sender {
        upsert_user(c, u)?;
    }
    let raw = serde_json::to_string(m)?;
    let keep = |v: Option<String>| if deleted { None } else { v };

    c.execute(
        "INSERT INTO message (name, space, thread, is_reply, sender, create_us, update_us, delete_us,
                              text, formatted_text, annotations, attachments, cards, reactions,
                              mentions_me, client_id, local_state, raw)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, 'synced', ?17)
         ON CONFLICT(name) DO UPDATE SET
           space = excluded.space, thread = excluded.thread, is_reply = excluded.is_reply,
           sender = excluded.sender, create_us = excluded.create_us, update_us = excluded.update_us,
           delete_us = excluded.delete_us, text = excluded.text,
           formatted_text = excluded.formatted_text, annotations = excluded.annotations,
           attachments = excluded.attachments, cards = excluded.cards, reactions = excluded.reactions,
           mentions_me = excluded.mentions_me,
           client_id = COALESCE(excluded.client_id, message.client_id),
           local_state = 'synced', raw = excluded.raw",
        params![
            m.name,
            space,
            m.thread.as_ref().map(|t| t.name.clone()),
            m.thread_reply.unwrap_or(false),
            sender,
            create_us,
            update_us,
            delete_us,
            keep(m.text.clone()),
            keep(m.formatted_text.clone()),
            keep(json(&m.annotations)),
            keep(json(&m.attachment)),
            keep(json(&m.cards_v2)),
            keep(json(&m.emoji_reaction_summaries)),
            !deleted && mentions(m, me),
            m.client_assigned_message_id,
            raw,
        ],
    )?;
    Ok(if existing.is_some() {
        Upsert::Updated
    } else {
        Upsert::Inserted
    })
}

/// An optimistic row (`pending:` / failed) whose client ID matches a server message takes
/// the server name, or disappears if the server row already exists.
fn adopt_pending(c: &Connection, client_id: &str, server_name: &str) -> Result<()> {
    let pending: Option<i64> = c
        .query_row(
            "SELECT id FROM message WHERE client_id = ?1 AND name != ?2",
            params![client_id, server_name],
            |r| r.get(0),
        )
        .optional()?;
    let Some(id) = pending else { return Ok(()) };
    let server_exists: bool = c
        .query_row("SELECT 1 FROM message WHERE name = ?1", [server_name], |_| Ok(true))
        .optional()?
        .unwrap_or(false);
    if server_exists {
        c.execute("DELETE FROM message WHERE id = ?1", [id])?;
    } else {
        c.execute("UPDATE message SET name = ?1 WHERE id = ?2", params![server_name, id])?;
    }
    Ok(())
}

pub fn mark_deleted(c: &Connection, name: &str, delete_us: Micros) -> Result<()> {
    let n = c.execute(
        "UPDATE message SET
           delete_us = MIN(COALESCE(delete_us, ?2), ?2),
           text = NULL, formatted_text = NULL, annotations = NULL, attachments = NULL,
           cards = NULL, reactions = NULL, mentions_me = 0
         WHERE name = ?1",
        params![name, delete_us],
    )?;
    if n == 0 {
        c.execute(
            "INSERT INTO message (name, space, sender, create_us, update_us, delete_us, local_state, raw)
             VALUES (?1, ?2, '', 0, 0, ?3, 'ghost', '{}')",
            params![name, space_of(name), delete_us],
        )?;
    }
    c.execute("DELETE FROM my_reaction WHERE message = ?1", [name])?;
    Ok(())
}

pub fn insert_pending(
    c: &Connection,
    space: &str,
    thread: Option<&str>,
    text: &str,
    client_id: &str,
    me: &str,
    now: Micros,
) -> Result<String> {
    let name = format!("pending:{client_id}");
    let formatted = text.to_owned();
    c.execute(
        "INSERT INTO message (name, space, thread, is_reply, sender, create_us, update_us, text,
                              formatted_text, client_id, local_state, raw)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?6, ?7, ?8, ?9, 'pending', '{}')",
        params![
            name,
            space,
            thread,
            thread.is_some(),
            me,
            now,
            text,
            formatted,
            client_id
        ],
    )?;
    Ok(name)
}

/// Files going out with a pending message: `attachments` (names and types, so the pending
/// message shows them) and `raw` (where the staged files are, for delivery and retries).
pub fn set_pending_files(c: &Connection, name: &str, attachments: &str, raw: &str) -> Result<()> {
    c.execute(
        "UPDATE message SET attachments = ?2, raw = ?3 WHERE name = ?1 AND local_state != 'synced'",
        params![name, attachments, raw],
    )?;
    Ok(())
}

pub fn set_local_state(c: &Connection, name: &str, state: &str) -> Result<()> {
    c.execute(
        "UPDATE message SET local_state = ?2 WHERE name = ?1",
        params![name, state],
    )?;
    Ok(())
}

pub fn set_text_local(c: &Connection, name: &str, text: &str) -> Result<()> {
    c.execute(
        "UPDATE message SET text = ?2, formatted_text = ?2 WHERE name = ?1",
        params![name, text],
    )?;
    Ok(())
}

pub fn delete_row(c: &Connection, name: &str) -> Result<()> {
    c.execute("DELETE FROM message WHERE name = ?1", [name])?;
    Ok(())
}

pub fn set_reactions(c: &Connection, message: &str, summaries: &str) -> Result<()> {
    c.execute(
        "UPDATE message SET reactions = ?2 WHERE name = ?1",
        params![message, summaries],
    )?;
    Ok(())
}

pub fn set_my_reaction(c: &Connection, message: &str, emoji_key: &str, reaction_name: &str) -> Result<()> {
    c.execute(
        "INSERT INTO my_reaction (message, emoji, reaction_name) VALUES (?1, ?2, ?3)
         ON CONFLICT(message, emoji) DO UPDATE SET reaction_name = excluded.reaction_name",
        params![message, emoji_key, reaction_name],
    )?;
    Ok(())
}

pub fn clear_my_reaction(c: &Connection, message: &str, emoji_key: &str) -> Result<()> {
    c.execute(
        "DELETE FROM my_reaction WHERE message = ?1 AND emoji = ?2",
        params![message, emoji_key],
    )?;
    Ok(())
}

pub fn clear_my_reaction_by_name(c: &Connection, reaction_name: &str) -> Result<()> {
    c.execute("DELETE FROM my_reaction WHERE reaction_name = ?1", [reaction_name])?;
    Ok(())
}

pub fn upsert_space(c: &Connection, s: &Space) -> Result<()> {
    c.execute(
        "INSERT INTO space (name, space_type, display_name, threaded, last_active_us, member_count, space_uri, raw)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
         ON CONFLICT(name) DO UPDATE SET
           space_type = excluded.space_type, display_name = excluded.display_name,
           threaded = excluded.threaded,
           last_active_us = MAX(COALESCE(excluded.last_active_us, 0), COALESCE(space.last_active_us, 0)),
           member_count = COALESCE(excluded.member_count, space.member_count),
           space_uri = COALESCE(excluded.space_uri, space.space_uri), raw = excluded.raw",
        params![
            s.name,
            s.kind(),
            s.display_name.as_deref().filter(|d| !d.is_empty()),
            s.threaded(),
            s.last_active_time.as_deref().and_then(ts::parse),
            s.membership_count
                .as_ref()
                .and_then(|m| m.joined_direct_human_user_count),
            s.space_uri,
            serde_json::to_string(s)?,
        ],
    )?;
    Ok(())
}

pub fn delete_space(c: &Connection, name: &str) -> Result<()> {
    let prefix = format!("{name}/%");
    c.execute("DELETE FROM my_reaction WHERE message LIKE ?1", [&prefix])?;
    c.execute("DELETE FROM message WHERE space = ?1", [name])?;
    c.execute("DELETE FROM membership WHERE space = ?1", [name])?;
    c.execute("DELETE FROM space_read_state WHERE space = ?1", [name])?;
    c.execute("DELETE FROM thread_read_state WHERE thread LIKE ?1", [&prefix])?;
    c.execute("DELETE FROM space WHERE name = ?1", [name])?;
    Ok(())
}

pub fn set_space_synced(c: &Connection, space: &str, us: Micros) -> Result<()> {
    c.execute(
        "UPDATE space SET last_synced_us = ?2 WHERE name = ?1",
        params![space, us],
    )?;
    Ok(())
}

pub fn bump_space_activity(c: &Connection, space: &str, us: Micros) -> Result<()> {
    c.execute(
        "UPDATE space SET last_active_us = MAX(COALESCE(last_active_us, 0), ?2) WHERE name = ?1",
        params![space, us],
    )?;
    Ok(())
}

pub fn set_history_complete(c: &Connection, space: &str, complete: bool) -> Result<()> {
    c.execute(
        "UPDATE space SET history_complete = ?2 WHERE name = ?1",
        params![space, complete],
    )?;
    Ok(())
}

/// Only meaningful for direct messages: the other person names the conversation.
pub fn set_dm_peer(c: &Connection, space: &str, peer: &str) -> Result<()> {
    c.execute(
        "UPDATE space SET dm_peer = ?2 WHERE name = ?1 AND space_type = 'DIRECT_MESSAGE'",
        params![space, peer],
    )?;
    Ok(())
}

pub fn set_space_prefs(c: &Connection, space: &str, muted: Option<bool>, starred: Option<bool>) -> Result<()> {
    if let Some(m) = muted {
        c.execute("UPDATE space SET muted = ?2 WHERE name = ?1", params![space, m])?;
    }
    if let Some(s) = starred {
        c.execute("UPDATE space SET starred = ?2 WHERE name = ?1", params![space, s])?;
    }
    Ok(())
}

/// From a message sender or membership: fills gaps, never erases known values.
pub fn upsert_user(c: &Connection, u: &User) -> Result<()> {
    if u.name.is_empty() {
        return Ok(());
    }
    c.execute(
        "INSERT INTO user (name, display_name, email, avatar_url) VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT(name) DO UPDATE SET
           display_name = COALESCE(excluded.display_name, user.display_name),
           email = COALESCE(excluded.email, user.email),
           avatar_url = COALESCE(excluded.avatar_url, user.avatar_url)",
        params![
            u.name,
            u.display_name.as_deref().filter(|d| !d.is_empty()),
            u.email.as_deref().filter(|d| !d.is_empty()),
            u.avatar_url.as_deref().filter(|d| !d.is_empty()),
        ],
    )?;
    Ok(())
}

pub fn upsert_person(c: &Connection, p: &Person, now: Micros) -> Result<()> {
    c.execute(
        "INSERT INTO user (name, display_name, email, avatar_url, resolved_at_us) VALUES (?1, ?2, ?3, ?4, ?5)
         ON CONFLICT(name) DO UPDATE SET
           display_name = COALESCE(excluded.display_name, user.display_name),
           email = COALESCE(excluded.email, user.email),
           avatar_url = COALESCE(excluded.avatar_url, user.avatar_url),
           resolved_at_us = excluded.resolved_at_us",
        params![p.user, p.display_name, p.email, p.avatar_url, now],
    )?;
    Ok(())
}

/// Marks users the People API could not resolve, so they are not asked for again.
pub fn mark_resolved(c: &Connection, users: &[String], now: Micros) -> Result<()> {
    for u in users {
        c.execute(
            "INSERT INTO user (name, resolved_at_us) VALUES (?1, ?2)
             ON CONFLICT(name) DO UPDATE SET resolved_at_us = COALESCE(user.resolved_at_us, excluded.resolved_at_us)",
            params![u, now],
        )?;
    }
    Ok(())
}

pub fn upsert_membership(c: &Connection, space: &str, member: &User, role: Option<&str>) -> Result<()> {
    upsert_user(c, member)?;
    c.execute(
        "INSERT INTO membership (space, member, role) VALUES (?1, ?2, ?3)
         ON CONFLICT(space, member) DO UPDATE SET role = excluded.role",
        params![space, member.name, role],
    )?;
    Ok(())
}

pub fn delete_membership(c: &Connection, space: &str, member: &str) -> Result<()> {
    c.execute(
        "DELETE FROM membership WHERE space = ?1 AND member = ?2",
        params![space, member],
    )?;
    Ok(())
}

pub fn clear_memberships(c: &Connection, space: &str) -> Result<()> {
    c.execute("DELETE FROM membership WHERE space = ?1", [space])?;
    Ok(())
}

pub fn set_space_read(c: &Connection, space: &str, us: Option<Micros>) -> Result<()> {
    c.execute(
        "INSERT INTO space_read_state (space, last_read_us) VALUES (?1, ?2)
         ON CONFLICT(space) DO UPDATE SET last_read_us = excluded.last_read_us",
        params![space, us],
    )?;
    Ok(())
}

pub fn set_thread_read(c: &Connection, thread: &str, us: Option<Micros>) -> Result<()> {
    c.execute(
        "INSERT INTO thread_read_state (thread, last_read_us) VALUES (?1, ?2)
         ON CONFLICT(thread) DO UPDATE SET last_read_us = excluded.last_read_us",
        params![thread, us],
    )?;
    Ok(())
}

pub fn set_account(c: &Connection, id: &str, email: Option<&str>, name: Option<&str>) -> Result<()> {
    c.execute("DELETE FROM account WHERE id != ?1", [id])?;
    c.execute(
        "INSERT INTO account (id, email, display_name) VALUES (?1, ?2, ?3)
         ON CONFLICT(id) DO UPDATE SET email = excluded.email, display_name = excluded.display_name",
        params![id, email, name],
    )?;
    Ok(())
}

/// Sets the contiguous-history low-water mark (see migrate_002.sql).
pub fn set_low_water(c: &Connection, space: &str, us: Micros) -> Result<()> {
    c.execute("UPDATE space SET low_water_us = ?2 WHERE name = ?1", params![space, us])?;
    Ok(())
}

/// Sends interrupted by a quit were never confirmed; show them as failed with Retry.
pub fn fail_stale_pending(c: &Connection) -> Result<usize> {
    Ok(c.execute(
        "UPDATE message SET local_state = 'failed' WHERE local_state = 'pending'",
        [],
    )?)
}
