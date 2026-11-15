//! What kwek keeps for the user that Google Chat's API has no place for: saved messages,
//! reminders about a message, messages written now to send later, and the Activity list
//! (mentions, and replies in threads the user took part in). All local to this computer.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Duration;

use rusqlite::{Connection, OptionalExtension, params, params_from_iter};

use super::{Engine, Notification, Outgoing, remove_staged};
use crate::error::{Error, Result};
use crate::render;
use crate::ts::{self, Micros};
use crate::views::{ListedMessage, Mark, MessageView, ScheduledView};

/// How often due reminders and scheduled messages are looked for. Polling, not a timer per
/// item, so a computer waking from sleep catches up on the next pass.
const EVERY: Duration = Duration::from_secs(15);
const ACTIVITY_LIMIT: u32 = 200;

/// One pass at a time, and no cancel during one: the loop, Send now and Cancel would
/// otherwise read the same scheduled row and send it twice, or send one just cancelled.
static PASS: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// Which of `names` are saved, and the reminders among them.
pub(super) fn marks(c: &Connection, names: &[String]) -> Result<(HashSet<String>, HashMap<String, Micros>)> {
    if names.is_empty() {
        return Ok(Default::default());
    }
    let ph = vec!["?"; names.len()].join(",");
    let saved = c
        .prepare(&format!("SELECT message FROM saved WHERE message IN ({ph})"))?
        .query_map(params_from_iter(names), |r| r.get(0))?
        .collect::<rusqlite::Result<_>>()?;
    let reminders = c
        .prepare(&format!("SELECT message, at_us FROM reminder WHERE message IN ({ph})"))?
        .query_map(params_from_iter(names), |r| Ok((r.get(0)?, r.get(1)?)))?
        .collect::<rusqlite::Result<_>>()?;
    Ok((saved, reminders))
}

/// The staged files scheduled messages still need.
pub(super) fn scheduled_files(c: &Connection) -> Result<Vec<Outgoing>> {
    let mut st = c.prepare("SELECT files FROM scheduled")?;
    let all: Vec<String> = st.query_map([], |r| r.get(0))?.collect::<rusqlite::Result<_>>()?;
    Ok(all
        .iter()
        .filter_map(|f| serde_json::from_str::<Vec<Outgoing>>(f).ok())
        .flatten()
        .collect())
}

/// One line for a notification or a list.
fn preview(v: &MessageView) -> String {
    let mut body = render::plain(&v.blocks);
    if body.trim().is_empty() && !v.attachments.is_empty() {
        body = "An attachment".into();
    }
    if body.chars().count() > 180 {
        body = body.chars().take(177).collect::<String>() + "…";
    }
    body
}

impl Engine {
    fn space_names(&self) -> HashMap<String, String> {
        self.list_spaces()
            .map(|s| s.into_iter().map(|s| (s.name, s.display_name)).collect())
            .unwrap_or_default()
    }

    /// Views for `(message, why, at)` in that order, leaving out messages no longer cached.
    fn listed(&self, rows: Vec<(String, &'static str, Micros, bool)>) -> Result<Vec<ListedMessage>> {
        let names: Vec<String> = rows.iter().map(|r| r.0.clone()).collect();
        let mut views: HashMap<String, MessageView> = self
            .message_views_by_name(&names)?
            .into_iter()
            .map(|v| (v.name.clone(), v))
            .collect();
        let spaces = self.space_names();
        Ok(rows
            .into_iter()
            .filter_map(|(name, why, at_us, unread)| {
                let message = views.remove(&name).filter(|v| !v.deleted)?;
                Some(ListedMessage {
                    space_name: spaces.get(&message.space).cloned().unwrap_or_default(),
                    message,
                    why: why.into(),
                    at_us,
                    unread,
                })
            })
            .collect())
    }

    fn marked(&self, message: &str) -> Result<()> {
        let n = message.to_owned();
        let (saved, reminders) = self.db.with(|c| marks(c, &[n]))?;
        self.sink.later_changed(Some(Mark {
            message: message.to_owned(),
            saved: saved.contains(message),
            remind_at_us: reminders.get(message).copied(),
        }));
        Ok(())
    }

    pub async fn set_saved(&self, message: &str, saved: bool) -> Result<()> {
        let m = message.to_owned();
        self.db
            .call(move |c| {
                if saved {
                    c.execute(
                        "INSERT INTO saved (message, saved_us) VALUES (?1, ?2) ON CONFLICT(message) DO NOTHING",
                        params![m, ts::now()],
                    )?;
                } else {
                    c.execute("DELETE FROM saved WHERE message = ?1", [&m])?;
                }
                Ok(())
            })
            .await?;
        self.marked(message)
    }

    /// `at: None` clears the reminder.
    pub async fn set_reminder(&self, message: &str, at: Option<Micros>) -> Result<()> {
        let m = message.to_owned();
        self.db
            .call(move |c| {
                match at {
                    Some(at) => c.execute(
                        "INSERT INTO reminder (message, at_us) VALUES (?1, ?2)
                         ON CONFLICT(message) DO UPDATE SET at_us = excluded.at_us",
                        params![m, at],
                    )?,
                    None => c.execute("DELETE FROM reminder WHERE message = ?1", [&m])?,
                };
                Ok(())
            })
            .await?;
        self.marked(message)
    }

    /// Saved messages, most recently saved first.
    pub fn saved(&self) -> Result<Vec<ListedMessage>> {
        let rows = self.db.with(|c| {
            let mut st = c.prepare("SELECT message, saved_us FROM saved ORDER BY saved_us DESC")?;
            let rows = st
                .query_map([], |r| Ok((r.get(0)?, "saved", r.get(1)?, false)))?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            Ok(rows)
        })?;
        self.listed(rows)
    }

    /// Reminders still to come, soonest first.
    pub fn reminders(&self) -> Result<Vec<ListedMessage>> {
        let rows = self.db.with(|c| {
            let mut st = c.prepare("SELECT message, at_us FROM reminder ORDER BY at_us")?;
            let rows = st
                .query_map([], |r| Ok((r.get(0)?, "reminder", r.get(1)?, false)))?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            Ok(rows)
        })?;
        self.listed(rows)
    }

    /// Mentions of the user, and replies by others in threads the user wrote in; newest first.
    pub fn activity(&self) -> Result<Vec<ListedMessage>> {
        let me = self.me();
        let rows = self.db.with(|c| {
            let mut st = c.prepare(
                "SELECT m.name, m.mentions_me, m.create_us,
                        CASE WHEN m.is_reply = 1 THEN tr.last_read_us ELSE sr.last_read_us END
                 FROM message m
                 LEFT JOIN space_read_state sr ON sr.space = m.space
                 LEFT JOIN thread_read_state tr ON tr.thread = m.thread
                 WHERE m.delete_us IS NULL AND m.local_state = 'synced' AND m.sender != ?1
                   AND (m.mentions_me = 1
                        OR (m.is_reply = 1 AND m.thread IN
                              (SELECT thread FROM message WHERE sender = ?1 AND thread IS NOT NULL)))
                 ORDER BY m.create_us DESC
                 LIMIT ?2",
            )?;
            let rows = st
                .query_map(params![me, ACTIVITY_LIMIT], |r| {
                    let mention: bool = r.get(1)?;
                    let at: Micros = r.get(2)?;
                    let read: Option<Micros> = r.get(3)?;
                    Ok((
                        r.get::<_, String>(0)?,
                        if mention { "mention" } else { "reply" },
                        at,
                        // Never read at all counts as unread here: it's about you.
                        read.is_none_or(|r| at > r),
                    ))
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            Ok(rows)
        })?;
        self.listed(rows)
    }

    /// Keeps a message to send at `at`. Its files stay staged in the outbox until then.
    pub async fn schedule(
        &self,
        space: &str,
        thread: Option<&str>,
        text: &str,
        files: Vec<Outgoing>,
        at: Micros,
    ) -> Result<()> {
        let (space, thread, text) = (space.to_owned(), thread.map(str::to_owned), text.to_owned());
        let files = serde_json::to_string(&files)?;
        self.db
            .call(move |c| {
                c.execute(
                    "INSERT INTO scheduled (space, thread, text, files, at_us) VALUES (?1, ?2, ?3, ?4, ?5)",
                    params![space, thread, text, files, at],
                )?;
                Ok(())
            })
            .await?;
        self.sink.later_changed(None);
        Ok(())
    }

    /// Messages waiting to be sent, soonest first.
    pub fn scheduled(&self) -> Result<Vec<ScheduledView>> {
        let spaces = self.space_names();
        self.db.with(|c| {
            let mut st = c.prepare("SELECT id, space, thread, text, files, at_us FROM scheduled ORDER BY at_us")?;
            let rows = st
                .query_map([], |r| {
                    let space: String = r.get(1)?;
                    let files: String = r.get(4)?;
                    Ok(ScheduledView {
                        id: r.get(0)?,
                        space_name: spaces.get(&space).cloned().unwrap_or_default(),
                        space,
                        thread: r.get(2)?,
                        text: r.get(3)?,
                        files: serde_json::from_str::<Vec<Outgoing>>(&files)
                            .unwrap_or_default()
                            .into_iter()
                            .map(|f| f.name)
                            .collect(),
                        at_us: r.get(5)?,
                    })
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            Ok(rows)
        })
    }

    /// Drops a scheduled message and its staged files.
    pub async fn cancel_scheduled(&self, id: i64) -> Result<()> {
        let _pass = PASS.lock().await;
        let files: Option<String> = self
            .db
            .call(move |c| {
                let f = c
                    .query_row("SELECT files FROM scheduled WHERE id = ?1", [id], |r| r.get(0))
                    .optional()?;
                c.execute("DELETE FROM scheduled WHERE id = ?1", [id])?;
                Ok(f)
            })
            .await?;
        if let Some(f) = files {
            remove_staged(&serde_json::from_str::<Vec<Outgoing>>(&f).unwrap_or_default());
        }
        self.sink.later_changed(None);
        Ok(())
    }

    /// Sends a scheduled message right away.
    pub async fn send_scheduled_now(self: &Arc<Self>, id: i64) -> Result<()> {
        let changed = self
            .db
            .call(move |c| Ok(c.execute("UPDATE scheduled SET at_us = 0 WHERE id = ?1", [id])?))
            .await?;
        if changed == 0 {
            return Err(Error::Other("That message was already sent or cancelled.".into()));
        }
        self.later_pass().await
    }

    /// Runs reminders and scheduled sends for as long as kwek runs.
    pub async fn later(self: Arc<Self>) {
        loop {
            if let Err(e) = self.later_pass().await {
                tracing::warn!("reminders and scheduled messages: {e}");
            }
            tokio::time::sleep(EVERY).await;
        }
    }

    /// Fires what is due: reminders become notifications, scheduled messages are sent.
    pub async fn later_pass(self: &Arc<Self>) -> Result<()> {
        if self.me().is_empty() {
            return Ok(());
        }
        let _pass = PASS.lock().await;
        let now = ts::now();
        let due: Vec<String> = self
            .db
            .call(move |c| {
                let tx = c.transaction()?;
                let due = tx
                    .prepare("SELECT message FROM reminder WHERE at_us <= ?1 ORDER BY at_us")?
                    .query_map([now], |r| r.get(0))?
                    .collect::<rusqlite::Result<Vec<String>>>()?;
                tx.execute("DELETE FROM reminder WHERE at_us <= ?1", [now])?;
                tx.commit()?;
                Ok(due)
            })
            .await?;
        // Already taken off the list: one failing must not cost the others their notification.
        for name in due {
            match self.message_view(&name) {
                Ok(Some(v)) if !v.deleted => self.sink.notify(Notification {
                    title: format!("Reminder: {}", v.sender.display_name),
                    body: preview(&v),
                    space: v.space.clone(),
                    message: v.name.clone(),
                    thread: v.thread.clone(),
                    url: None,
                }),
                Ok(_) => tracing::info!("reminder for a message no longer here: {name}"),
                Err(e) => tracing::warn!("reminder for {name} not shown: {e}"),
            }
            if let Err(e) = self.marked(&name) {
                tracing::warn!("reminder state not sent to the window: {e}");
            }
        }

        type Row = (i64, String, Option<String>, String, String);
        let sends: Vec<Row> = self
            .db
            .call(move |c| {
                let mut st = c.prepare("SELECT id, space, thread, text, files FROM scheduled WHERE at_us <= ?1 ORDER BY at_us")?;
                let rows = st
                    .query_map([now], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)))?
                    .collect::<rusqlite::Result<Vec<Row>>>()?;
                Ok(rows)
            })
            .await?;
        let any = !sends.is_empty();
        for (id, space, thread, text, files) in sends {
            let files: Vec<Outgoing> = serde_json::from_str(&files).unwrap_or_default();
            // The send is queued (and retried like any other) before the row goes; a crash in
            // between sends it twice rather than never.
            if let Err(e) = self.send_message(&space, thread.as_deref(), &text, files).await {
                tracing::warn!("scheduled message not sent yet: {e}");
                continue;
            }
            self.db
                .call(move |c| Ok(c.execute("DELETE FROM scheduled WHERE id = ?1", [id])?))
                .await?;
        }
        if any {
            self.sink.later_changed(None);
        }
        Ok(())
    }
}
