//! Meetings from the user's Google Calendar: what is next, and a reminder with the Meet link
//! a minute before it starts. Read-only (`calendar.events.readonly`), polled; a calendar
//! that can't be read never gets in the way of chat.

use std::collections::HashSet;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use serde_json::Value;
use tokio::sync::Notify;

use crate::error::{Error, Result};
use crate::google::{Google, q};
use crate::sync::{Notification, Sink};
use crate::ts::{self, DAY, MINUTE, Micros};
use crate::views::{CalendarView, MeetingView};

/// How often the meeting list is fetched again.
const REFRESH: Duration = Duration::from_secs(5 * 60);
/// How often reminders are checked.
const TICK: Duration = Duration::from_secs(15);
/// A reminder this long before the start.
const LEAD: Micros = MINUTE;
/// ...and still, this long after it, for a kwek that just started or woke up.
const GRACE: Micros = 2 * MINUTE;

#[async_trait]
pub trait CalendarApi: Send + Sync {
    /// Timed meetings overlapping `[from, to)`, soonest first.
    async fn upcoming(&self, from: Micros, to: Micros) -> Result<Vec<MeetingView>>;
}

pub struct HttpCalendar {
    g: Google,
}

impl HttpCalendar {
    pub fn new(g: Google) -> Self {
        Self { g }
    }
}

#[async_trait]
impl CalendarApi for HttpCalendar {
    async fn upcoming(&self, from: Micros, to: Micros) -> Result<Vec<MeetingView>> {
        let url = format!(
            // 250 is the API's page maximum, more than a day's meetings.
            "https://www.googleapis.com/calendar/v3/calendars/primary/events?singleEvents=true&orderBy=startTime&maxResults=250&timeMin={}&timeMax={}",
            q(&ts::format(from)),
            q(&ts::format(to))
        );
        let v: Value = self.g.get(&url).await?;
        Ok(parse_events(&v))
    }
}

/// The meetings worth showing: timed (not all-day), not cancelled, not declined, and not a
/// working-location, focus-time or out-of-office block.
pub fn parse_events(v: &Value) -> Vec<MeetingView> {
    let Some(items) = v.get("items").and_then(|i| i.as_array()) else {
        return vec![];
    };
    items
        .iter()
        .filter(|e| e.get("status").and_then(|s| s.as_str()) != Some("cancelled"))
        .filter(|e| {
            !matches!(
                e.get("eventType").and_then(|t| t.as_str()),
                Some("workingLocation" | "focusTime" | "outOfOffice")
            )
        })
        .filter(|e| {
            let me_declined = e.get("attendees").and_then(|a| a.as_array()).is_some_and(|a| {
                a.iter().any(|p| {
                    p.get("self").and_then(|s| s.as_bool()) == Some(true)
                        && p.get("responseStatus").and_then(|r| r.as_str()) == Some("declined")
                })
            });
            !me_declined
        })
        .filter_map(|e| {
            let time = |k: &str| {
                e.pointer(&format!("/{k}/dateTime"))
                    .and_then(|t| t.as_str())
                    .and_then(ts::parse)
            };
            let join_url = e
                .get("hangoutLink")
                .and_then(|l| l.as_str())
                .or_else(|| {
                    e.pointer("/conferenceData/entryPoints")?
                        .as_array()?
                        .iter()
                        .find(|p| p.get("entryPointType").and_then(|t| t.as_str()) == Some("video"))?
                        .get("uri")?
                        .as_str()
                })
                .map(str::to_owned);
            Some(MeetingView {
                id: e.get("id")?.as_str()?.to_owned(),
                title: e
                    .get("summary")
                    .and_then(|s| s.as_str())
                    .filter(|s| !s.trim().is_empty())
                    .unwrap_or("Meeting")
                    .to_owned(),
                start_us: time("start")?,
                end_us: time("end")?,
                join_url,
                open_url: e.get("htmlLink").and_then(|l| l.as_str()).map(str::to_owned),
            })
        })
        .collect()
}

/// Meetings whose reminder is due at `now` and hasn't gone out: from a minute before the
/// start until two minutes after. Keyed by id and start, so a moved meeting reminds again.
pub fn due_reminders<'a>(meetings: &'a [MeetingView], now: Micros, sent: &HashSet<String>) -> Vec<&'a MeetingView> {
    meetings
        .iter()
        .filter(|m| m.start_us - LEAD <= now && now < m.start_us + GRACE)
        .filter(|m| !sent.contains(&reminder_key(m)))
        .collect()
}

fn reminder_key(m: &MeetingView) -> String {
    format!("{}@{}", m.id, m.start_us)
}

/// Keeps the meeting list fresh and sends reminders. Runs forever; quiet while signed out.
pub struct Agenda {
    api: Arc<dyn CalendarApi>,
    sink: Arc<dyn Sink>,
    view: Mutex<CalendarView>,
    sent: Mutex<HashSet<String>>,
    wake: Notify,
}

impl Agenda {
    pub fn new(api: Arc<dyn CalendarApi>, sink: Arc<dyn Sink>) -> Arc<Self> {
        Arc::new(Self {
            api,
            sink,
            view: Mutex::new(CalendarView {
                state: "off".into(),
                detail: None,
                meetings: vec![],
            }),
            sent: Mutex::new(HashSet::new()),
            wake: Notify::new(),
        })
    }

    pub fn view(&self) -> CalendarView {
        self.view.lock().unwrap().clone()
    }

    /// Fetch again now (after signing in, or getting calendar access).
    pub fn poke(&self) {
        self.wake.notify_one();
    }

    pub async fn run(self: Arc<Self>) {
        loop {
            self.refresh().await;
            let mut next = tokio::time::Instant::now() + REFRESH;
            loop {
                self.remind(ts::now());
                tokio::select! {
                    _ = self.wake.notified() => break,
                    _ = tokio::time::sleep(TICK) => {}
                }
                if tokio::time::Instant::now() >= next {
                    next = tokio::time::Instant::now() + REFRESH;
                    self.refresh().await;
                }
            }
        }
    }

    pub async fn refresh(&self) {
        let now = ts::now();
        let next = match self.api.upcoming(now, now + DAY).await {
            Ok(meetings) => CalendarView {
                state: "ok".into(),
                detail: None,
                meetings,
            },
            // Offline: keep what we have.
            Err(e) if e.is_offline() => return,
            Err(Error::AuthRequired | Error::NoOAuthClient) => CalendarView {
                state: "off".into(),
                detail: None,
                meetings: vec![],
            },
            Err(e) => {
                tracing::info!("calendar unavailable: {e}");
                match classify(&e) {
                    // A hiccup (429, 5xx, an odd answer): keep the meetings and reminders we
                    // have, and try again at the next refresh.
                    None => return,
                    Some((state, detail)) => CalendarView {
                        state: state.into(),
                        detail,
                        meetings: vec![],
                    },
                }
            }
        };
        let changed = {
            let mut v = self.view.lock().unwrap();
            let changed = *v != next;
            *v = next.clone();
            changed
        };
        if changed {
            self.sink.calendar(next);
        }
    }

    pub fn remind(&self, now: Micros) {
        let meetings = self.view.lock().unwrap().meetings.clone();
        let due: Vec<MeetingView> = {
            let sent = self.sent.lock().unwrap();
            due_reminders(&meetings, now, &sent).into_iter().cloned().collect()
        };
        for m in due {
            self.sent.lock().unwrap().insert(reminder_key(&m));
            let title = if m.start_us > now {
                format!("{} starts in a minute", m.title)
            } else {
                format!("{} is starting", m.title)
            };
            let body = if m.join_url.is_some() {
                "Join in your browser."
            } else {
                "No video link in the invite."
            };
            // Only a Meet link is something to join; without one, the reminder just reminds.
            self.sink.notify(Notification {
                title,
                body: body.into(),
                url: m.join_url.clone(),
                ..Default::default()
            });
        }
    }
}

/// Why the calendar can't be read, for the sidebar: a lasting reason (state and detail), or
/// None for a hiccup worth riding out.
fn classify(e: &Error) -> Option<(&'static str, Option<String>)> {
    match e {
        Error::Api {
            status: 403, message, ..
        } if message.contains("insufficient") || message.contains("scope") => Some(("needsPermission", None)),
        Error::Api {
            status: 403, message, ..
        } if message.contains("has not been used") || message.contains("disabled") => Some((
            "unavailable",
            Some("The Google Calendar API is off in your Google Cloud project (setup guide, step 1).".into()),
        )),
        Error::Api {
            status: 401 | 403 | 404,
            ..
        } => Some((
            "unavailable",
            Some("Google Calendar refuses kwek access to your calendar.".into()),
        )),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn event(id: &str, start: &str, extra: Value) -> Value {
        let mut e = json!({
            "id": id, "summary": format!("Meeting {id}"), "status": "confirmed",
            "start": { "dateTime": start }, "end": { "dateTime": "2026-11-09T11:00:00+01:00" },
            "htmlLink": format!("https://calendar.google.com/event?eid={id}"),
        });
        for (k, v) in extra.as_object().unwrap() {
            e[k] = v.clone();
        }
        e
    }

    #[test]
    fn keeps_meetings_and_finds_their_meet_link() {
        let v = json!({ "items": [
            event("a", "2026-11-09T10:00:00+01:00", json!({ "hangoutLink": "https://meet.google.com/aaa-bbbb-ccc" })),
            event("b", "2026-11-09T10:30:00+01:00", json!({ "conferenceData": { "entryPoints": [
                { "entryPointType": "phone", "uri": "tel:+31" },
                { "entryPointType": "video", "uri": "https://meet.google.com/ddd-eeee-fff" } ] } })),
            event("c", "2026-11-09T10:45:00+01:00", json!({})),
            event("cancelled", "2026-11-09T10:00:00+01:00", json!({ "status": "cancelled" })),
            event("declined", "2026-11-09T10:00:00+01:00", json!({ "attendees": [
                { "email": "me@example.com", "self": true, "responseStatus": "declined" } ] })),
            event("focus", "2026-11-09T10:00:00+01:00", json!({ "eventType": "focusTime" })),
            { "id": "allday", "summary": "Holiday", "start": { "date": "2026-11-09" }, "end": { "date": "2026-11-10" } },
        ]});
        let m = parse_events(&v);
        let ids: Vec<&str> = m.iter().map(|x| x.id.as_str()).collect();
        assert_eq!(ids, vec!["a", "b", "c"]);
        assert_eq!(m[0].join_url.as_deref(), Some("https://meet.google.com/aaa-bbbb-ccc"));
        assert_eq!(m[1].join_url.as_deref(), Some("https://meet.google.com/ddd-eeee-fff"));
        assert_eq!(m[2].join_url, None);
        assert_eq!(m[0].start_us, ts::parse("2026-11-09T09:00:00Z").unwrap());
    }

    #[test]
    fn reminds_once_around_the_start() {
        let start = ts::parse("2026-11-09T09:00:00Z").unwrap();
        let m = vec![MeetingView {
            id: "a".into(),
            title: "Standup".into(),
            start_us: start,
            end_us: start + 15 * MINUTE,
            join_url: None,
            open_url: None,
        }];
        let mut sent = HashSet::new();
        assert!(due_reminders(&m, start - 2 * MINUTE, &sent).is_empty(), "too early");
        assert_eq!(due_reminders(&m, start - 30 * ts::SECOND, &sent).len(), 1);
        assert_eq!(due_reminders(&m, start + MINUTE, &sent).len(), 1, "just started kwek");
        assert!(
            due_reminders(&m, start + 10 * MINUTE, &sent).is_empty(),
            "long running: no reminder"
        );
        sent.insert(reminder_key(&m[0]));
        assert!(
            due_reminders(&m, start - 30 * ts::SECOND, &sent).is_empty(),
            "only once"
        );
        let mut moved = m.clone();
        moved[0].start_us += 30 * MINUTE;
        assert_eq!(
            due_reminders(&moved, moved[0].start_us - 30 * ts::SECOND, &sent).len(),
            1,
            "moved: again"
        );
    }

    #[test]
    fn explains_why_the_calendar_cannot_be_read() {
        let api = |m: &str| Error::Api {
            status: 403,
            status_code: Some("PERMISSION_DENIED".into()),
            message: m.into(),
        };
        assert_eq!(
            classify(&api("Request had insufficient authentication scopes."))
                .unwrap()
                .0,
            "needsPermission"
        );
        assert_eq!(
            classify(&api(
                "Google Calendar API has not been used in project 1 before or it is disabled."
            ))
            .unwrap()
            .0,
            "unavailable"
        );
        let hiccup = Error::Api {
            status: 503,
            status_code: None,
            message: "backend error".into(),
        };
        assert!(classify(&hiccup).is_none(), "a passing error keeps the meetings");
    }
}
