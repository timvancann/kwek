//! Queries behind the views. Rows here are raw; `views.rs` turns them into what the UI shows.

use std::collections::HashMap;

use rusqlite::{Connection, OptionalExtension, Row, params, params_from_iter};

use crate::error::Result;
use crate::ts::Micros;

#[derive(Debug, Clone)]
pub struct Account {
    pub id: String,
    pub email: Option<String>,
    pub display_name: Option<String>,
}

pub fn account(c: &Connection) -> Result<Option<Account>> {
    Ok(
        c.query_row("SELECT id, email, display_name FROM account LIMIT 1", [], |r| {
            Ok(Account {
                id: r.get(0)?,
                email: r.get(1)?,
                display_name: r.get(2)?,
            })
        })
        .optional()?,
    )
}

#[derive(Debug, Clone)]
pub struct SpaceRow {
    pub name: String,
    pub space_type: String,
    pub display_name: Option<String>,
    pub threaded: bool,
    pub last_active_us: Option<Micros>,
    pub last_synced_us: Option<Micros>,
    pub member_count: Option<i64>,
    pub space_uri: Option<String>,
    pub history_complete: bool,
    pub dm_peer: Option<String>,
    pub muted: bool,
    pub starred: bool,
    pub unread: i64,
    pub mentions: i64,
    pub last_read_us: Option<Micros>,
    pub newest_us: Option<Micros>,
    pub low_water_us: Option<Micros>,
}

const SPACE_SELECT: &str = "
SELECT s.name, s.space_type, s.display_name, s.threaded, s.last_active_us, s.last_synced_us,
       s.member_count, s.space_uri, s.history_complete, s.dm_peer, s.muted, s.starred,
       (SELECT count(*) FROM message m
         WHERE m.space = s.name AND m.is_reply = 0 AND m.sender != ?1 AND m.delete_us IS NULL
           AND m.local_state = 'synced' AND r.last_read_us IS NOT NULL AND m.create_us > r.last_read_us),
       (SELECT count(*) FROM message m
         WHERE m.space = s.name AND m.mentions_me = 1 AND m.sender != ?1 AND m.delete_us IS NULL
           AND r.last_read_us IS NOT NULL AND m.create_us > r.last_read_us),
       r.last_read_us,
       (SELECT max(create_us) FROM message m WHERE m.space = s.name AND m.local_state != 'ghost'),
       s.low_water_us
FROM space s LEFT JOIN space_read_state r ON r.space = s.name";

fn space_row(r: &Row) -> rusqlite::Result<SpaceRow> {
    Ok(SpaceRow {
        name: r.get(0)?,
        space_type: r.get(1)?,
        display_name: r.get(2)?,
        threaded: r.get(3)?,
        last_active_us: r.get(4)?,
        last_synced_us: r.get(5)?,
        member_count: r.get(6)?,
        space_uri: r.get(7)?,
        history_complete: r.get(8)?,
        dm_peer: r.get(9)?,
        muted: r.get(10)?,
        starred: r.get(11)?,
        unread: r.get(12)?,
        mentions: r.get(13)?,
        last_read_us: r.get(14)?,
        newest_us: r.get(15)?,
        low_water_us: r.get(16)?,
    })
}

pub fn spaces(c: &Connection, me: &str) -> Result<Vec<SpaceRow>> {
    let mut st = c.prepare(SPACE_SELECT)?;
    let rows = st.query_map([me], space_row)?.collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

pub fn space_exists(c: &Connection, name: &str) -> Result<bool> {
    Ok(c.query_row("SELECT 1 FROM space WHERE name = ?1", [name], |_| Ok(()))
        .optional()?
        .is_some())
}

pub fn space_names(c: &Connection) -> Result<Vec<String>> {
    let mut st = c.prepare("SELECT name FROM space")?;
    Ok(st.query_map([], |r| r.get(0))?.collect::<rusqlite::Result<Vec<_>>>()?)
}

#[derive(Debug, Clone)]
pub struct MessageRow {
    pub id: i64,
    pub name: String,
    pub space: String,
    pub thread: Option<String>,
    pub is_reply: bool,
    pub sender: String,
    pub create_us: Micros,
    pub update_us: Micros,
    pub delete_us: Option<Micros>,
    pub text: Option<String>,
    pub formatted_text: Option<String>,
    pub annotations: Option<String>,
    pub attachments: Option<String>,
    pub cards: Option<String>,
    pub reactions: Option<String>,
    pub mentions_me: bool,
    pub local_state: String,
}

const MSG_COLS: &str = "id, name, space, thread, is_reply, sender, create_us, update_us, delete_us, text,
    formatted_text, annotations, attachments, cards, reactions, mentions_me, local_state";

fn msg_row(r: &Row) -> rusqlite::Result<MessageRow> {
    Ok(MessageRow {
        id: r.get(0)?,
        name: r.get(1)?,
        space: r.get(2)?,
        thread: r.get(3)?,
        is_reply: r.get(4)?,
        sender: r.get(5)?,
        create_us: r.get(6)?,
        update_us: r.get(7)?,
        delete_us: r.get(8)?,
        text: r.get(9)?,
        formatted_text: r.get(10)?,
        annotations: r.get(11)?,
        attachments: r.get(12)?,
        cards: r.get(13)?,
        reactions: r.get(14)?,
        mentions_me: r.get(15)?,
        local_state: r.get(16)?,
    })
}

/// Top-level messages of a space, oldest first, the newest `limit` before `before_us`.
pub fn top_level(c: &Connection, space: &str, before_us: Option<Micros>, limit: u32) -> Result<Vec<MessageRow>> {
    let sql = format!(
        "SELECT {MSG_COLS} FROM message
         WHERE space = ?1 AND is_reply = 0 AND local_state != 'ghost' AND create_us < ?2
         ORDER BY create_us DESC, id DESC LIMIT ?3"
    );
    let mut st = c.prepare(&sql)?;
    let mut rows = st
        .query_map(params![space, before_us.unwrap_or(i64::MAX), limit], msg_row)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    rows.reverse();
    Ok(rows)
}

/// Root plus replies of a thread, oldest first.
pub fn thread(c: &Connection, thread: &str) -> Result<Vec<MessageRow>> {
    let sql = format!(
        "SELECT {MSG_COLS} FROM message WHERE thread = ?1 AND local_state != 'ghost'
         ORDER BY is_reply ASC, create_us ASC, id ASC"
    );
    let mut st = c.prepare(&sql)?;
    Ok(st.query_map([thread], msg_row)?.collect::<rusqlite::Result<Vec<_>>>()?)
}

pub fn message(c: &Connection, name: &str) -> Result<Option<MessageRow>> {
    let sql = format!("SELECT {MSG_COLS} FROM message WHERE name = ?1");
    Ok(c.query_row(&sql, [name], msg_row).optional()?)
}

pub fn messages_by_name(c: &Connection, names: &[String]) -> Result<Vec<MessageRow>> {
    if names.is_empty() {
        return Ok(vec![]);
    }
    let sql = format!(
        "SELECT {MSG_COLS} FROM message WHERE name IN ({}) AND local_state != 'ghost'",
        placeholders(names.len())
    );
    let mut st = c.prepare(&sql)?;
    Ok(st
        .query_map(params_from_iter(names), msg_row)?
        .collect::<rusqlite::Result<Vec<_>>>()?)
}

/// The newest own message in a space (or thread), for "up arrow edits last".
pub fn last_own(c: &Connection, space: &str, thread: Option<&str>, me: &str) -> Result<Option<MessageRow>> {
    let sql = format!(
        "SELECT {MSG_COLS} FROM message
         WHERE space = ?1 AND sender = ?2 AND delete_us IS NULL AND local_state = 'synced'
           AND ((?3 IS NULL AND is_reply = 0) OR thread = ?3)
         ORDER BY create_us DESC LIMIT 1"
    );
    Ok(c.query_row(&sql, params![space, me, thread], msg_row).optional()?)
}

/// The contiguous-history low-water mark and whether the start of history is cached.
pub fn history_bounds(c: &Connection, space: &str) -> Result<(Option<Micros>, bool)> {
    Ok(c.query_row(
        "SELECT low_water_us, history_complete FROM space WHERE name = ?1",
        [space],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )
    .optional()?
    .unwrap_or((None, false)))
}

/// An attachment resource name that appears in a cached message: the only media the
/// webview may ask Rust to fetch with the user's token.
pub fn is_known_attachment(c: &Connection, resource: &str) -> Result<bool> {
    // Match the resourceName field itself, not any string (a sender-chosen file name could
    // otherwise be made to look like a resource).
    let needle = format!("\"resourceName\":{}", serde_json::to_string(resource)?);
    Ok(c.query_row(
        "SELECT 1 FROM message WHERE attachments IS NOT NULL AND instr(attachments, ?1) > 0 LIMIT 1",
        [needle],
        |_| Ok(()),
    )
    .optional()?
    .is_some())
}

#[derive(Debug, Clone, Default)]
pub struct ThreadStats {
    pub reply_count: i64,
    pub last_reply_us: Micros,
    pub participants: Vec<String>,
    pub unread: bool,
}

pub fn thread_stats(c: &Connection, threads: &[String], me: &str) -> Result<HashMap<String, ThreadStats>> {
    let mut out: HashMap<String, ThreadStats> = HashMap::new();
    if threads.is_empty() {
        return Ok(out);
    }
    let sql = format!(
        "SELECT m.thread, m.sender, m.create_us, tr.last_read_us
         FROM message m LEFT JOIN thread_read_state tr ON tr.thread = m.thread
         WHERE m.thread IN ({}) AND m.is_reply = 1 AND m.local_state != 'ghost' AND m.delete_us IS NULL
         ORDER BY m.create_us DESC",
        placeholders(threads.len())
    );
    let mut st = c.prepare(&sql)?;
    let rows = st.query_map(params_from_iter(threads), |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, Micros>(2)?,
            r.get::<_, Option<Micros>>(3)?,
        ))
    })?;
    for row in rows {
        let (thread, sender, at, read) = row?;
        let s = out.entry(thread).or_default();
        s.reply_count += 1;
        s.last_reply_us = s.last_reply_us.max(at);
        if !s.participants.contains(&sender) && s.participants.len() < 3 {
            s.participants.push(sender.clone());
        }
        if sender != me && read.is_some_and(|r| at > r) {
            s.unread = true;
        }
    }
    Ok(out)
}

#[derive(Debug, Clone)]
pub struct UserRow {
    pub name: String,
    pub display_name: Option<String>,
    pub email: Option<String>,
    pub avatar_url: Option<String>,
}

fn user_row(r: &Row) -> rusqlite::Result<UserRow> {
    Ok(UserRow {
        name: r.get(0)?,
        display_name: r.get(1)?,
        email: r.get(2)?,
        avatar_url: r.get(3)?,
    })
}

pub fn users(c: &Connection, names: &[String]) -> Result<HashMap<String, UserRow>> {
    if names.is_empty() {
        return Ok(HashMap::new());
    }
    let sql = format!(
        "SELECT name, display_name, email, avatar_url FROM user WHERE name IN ({})",
        placeholders(names.len())
    );
    let mut st = c.prepare(&sql)?;
    let rows = st.query_map(params_from_iter(names), user_row)?;
    let mut out = HashMap::new();
    for r in rows {
        let r = r?;
        out.insert(r.name.clone(), r);
    }
    Ok(out)
}

pub fn all_users(c: &Connection) -> Result<HashMap<String, UserRow>> {
    let mut st = c.prepare("SELECT name, display_name, email, avatar_url FROM user")?;
    let rows = st.query_map([], user_row)?;
    let mut out = HashMap::new();
    for r in rows {
        let r = r?;
        out.insert(r.name.clone(), r);
    }
    Ok(out)
}

/// Users seen anywhere (members, DM peers) whose name is unknown and never looked up.
pub fn unresolved_users(c: &Connection, limit: u32) -> Result<Vec<String>> {
    let mut st = c.prepare(
        "SELECT name FROM user
         WHERE display_name IS NULL AND resolved_at_us IS NULL AND name LIKE 'users/%' AND name != 'users/all'
         LIMIT ?1",
    )?;
    Ok(st
        .query_map([limit], |r| r.get(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?)
}

pub fn members(c: &Connection, space: &str) -> Result<Vec<UserRow>> {
    let mut st = c.prepare(
        "SELECT u.name, u.display_name, u.email, u.avatar_url
         FROM membership m JOIN user u ON u.name = m.member
         WHERE m.space = ?1 ORDER BY COALESCE(u.display_name, u.email, u.name) COLLATE NOCASE",
    )?;
    Ok(st.query_map([space], user_row)?.collect::<rusqlite::Result<Vec<_>>>()?)
}

pub fn member_names(c: &Connection, space: &str) -> Result<Vec<String>> {
    let mut st = c.prepare("SELECT member FROM membership WHERE space = ?1")?;
    Ok(st
        .query_map([space], |r| r.get(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?)
}

/// For each message, the emoji keys the signed-in user reacted with.
pub fn my_reactions(c: &Connection, messages: &[String]) -> Result<HashMap<String, Vec<(String, String)>>> {
    let mut out: HashMap<String, Vec<(String, String)>> = HashMap::new();
    if messages.is_empty() {
        return Ok(out);
    }
    let sql = format!(
        "SELECT message, emoji, reaction_name FROM my_reaction WHERE message IN ({})",
        placeholders(messages.len())
    );
    let mut st = c.prepare(&sql)?;
    let rows = st.query_map(params_from_iter(messages), |r| {
        Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?))
    })?;
    for row in rows {
        let (m, e, n) = row?;
        out.entry(m).or_default().push((e, n));
    }
    Ok(out)
}

pub fn my_reaction_name(c: &Connection, message: &str, emoji_key: &str) -> Result<Option<String>> {
    Ok(c.query_row(
        "SELECT reaction_name FROM my_reaction WHERE message = ?1 AND emoji = ?2",
        params![message, emoji_key],
        |r| r.get(0),
    )
    .optional()?)
}

pub fn space_read(c: &Connection, space: &str) -> Result<Option<Micros>> {
    Ok(c.query_row(
        "SELECT last_read_us FROM space_read_state WHERE space = ?1",
        [space],
        |r| r.get(0),
    )
    .optional()?
    .flatten())
}

/// Messages with reactions whose own-reaction state was never fetched.
pub fn needs_my_reactions(c: &Connection, names: &[String]) -> Result<Vec<String>> {
    if names.is_empty() {
        return Ok(vec![]);
    }
    let sql = format!(
        "SELECT name FROM message WHERE name IN ({}) AND reactions IS NOT NULL AND reactions != '[]'",
        placeholders(names.len())
    );
    let mut st = c.prepare(&sql)?;
    Ok(st
        .query_map(params_from_iter(names), |r| r.get(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?)
}

#[derive(Debug, Clone, Default)]
pub struct SearchFilter {
    pub from: Option<String>,
    pub space: Option<String>,
    pub before_us: Option<Micros>,
    pub after_us: Option<Micros>,
}

/// Full-text search; returns rows plus a snippet with `\u{1}`/`\u{2}` around hits.
pub fn search(
    c: &Connection,
    fts_query: &str,
    f: &SearchFilter,
    limit: u32,
    offset: u32,
) -> Result<Vec<(MessageRow, String)>> {
    let cols = MSG_COLS
        .split(',')
        .map(|s| format!("m.{}", s.trim()))
        .collect::<Vec<_>>()
        .join(", ");
    let sql = format!(
        "SELECT {cols}, snippet(message_fts, 0, char(1), char(2), '…', 16)
         FROM message_fts JOIN message m ON m.id = message_fts.rowid
         LEFT JOIN user u ON u.name = m.sender
         LEFT JOIN space s ON s.name = m.space
         LEFT JOIN user peer ON peer.name = s.dm_peer
         WHERE message_fts MATCH ?1 AND m.delete_us IS NULL AND m.local_state != 'ghost'
           AND (?2 IS NULL OR u.display_name LIKE '%' || ?2 || '%' OR u.email LIKE ?2 || '%')
           AND (?3 IS NULL OR s.display_name LIKE '%' || ?3 || '%' OR peer.display_name LIKE '%' || ?3 || '%')
           AND (?4 IS NULL OR m.create_us < ?4)
           AND (?5 IS NULL OR m.create_us >= ?5)
         ORDER BY m.create_us DESC LIMIT ?6 OFFSET ?7"
    );
    let mut st = c.prepare(&sql)?;
    let rows = st.query_map(
        params![fts_query, f.from, f.space, f.before_us, f.after_us, limit, offset],
        |r| Ok((msg_row(r)?, r.get::<_, String>(17)?)),
    )?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

/// The point from which search covers every conversation: the most recent contiguous
/// low-water mark among spaces whose history is not fully cached.
pub fn search_coverage(c: &Connection) -> Result<Option<Micros>> {
    Ok(c.query_row(
        "SELECT max(low_water_us) FROM space WHERE history_complete = 0 AND low_water_us IS NOT NULL",
        [],
        |r| r.get(0),
    )?)
}

fn placeholders(n: usize) -> String {
    vec!["?"; n].join(",")
}
