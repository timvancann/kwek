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

/// How far ahead meetings are fetched.
const AHEAD: Micros = 7 * DAY;
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
        let base = format!(
            // 250 is the API's page maximum; all-day and focus-time blocks count against it.
            "https://www.googleapis.com/calendar/v3/calendars/primary/events?singleEvents=true&orderBy=startTime&maxResults=250&timeMin={}&timeMax={}",
            q(&ts::format(from)),
            q(&ts::format(to))
        );
        let mut out = vec![];
        let mut token: Option<String> = None;
        // A few pages at most: a week never needs more, and a loop must end.
        for _ in 0..4 {
            let url = match &token {
                Some(t) => format!("{base}&pageToken={}", q(t)),
                None => base.clone(),
            };
            let v: Value = self.g.get(&url).await?;
            out.extend(parse_events(&v));
            token = v.get("nextPageToken").and_then(|t| t.as_str()).map(str::to_owned);
            if token.is_none() {
                break;
            }
        }
        Ok(out)
    }
}

/// The meetings worth showing: timed (not all-day), with other people, not cancelled, not
/// declined, and not a working-location, focus-time or out-of-office block.
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
        .filter(|e| my_response(e) != Some("declined"))
        .filter(|e| with_others(e))
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
                .filter(|u| is_meeting_link(u))
                .filter(|_| !unanswered_outside_invite(e))
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
                location: e
                    .get("location")
                    .and_then(|l| l.as_str())
                    // One line, for the sidebar and a notification.
                    .map(|l| l.split_whitespace().collect::<Vec<_>>().join(" "))
                    .filter(|l| !l.is_empty()),
            })
        })
        .collect()
}

/// A meeting with someone else: another guest who isn't a room, or an invite organised by
/// someone else (whose guest list may be hidden from you). Your own blocks don't count.
fn with_others(e: &Value) -> bool {
    let is = |p: &Value, k: &str| p.get(k).and_then(|s| s.as_bool()) == Some(true);
    let guest = e
        .get("attendees")
        .and_then(|a| a.as_array())
        .is_some_and(|a| a.iter().any(|p| !is(p, "self") && !is(p, "resource")));
    let invited = e.get("organizer").is_some_and(|o| !is(o, "self"));
    guest || invited
}

fn my_response(e: &Value) -> Option<&str> {
    e.get("attendees")?
        .as_array()?
        .iter()
        .find(|p| p.get("self").and_then(|s| s.as_bool()) == Some(true))?
        .get("responseStatus")?
        .as_str()
}

/// An invite from another domain that you haven't answered. Anyone can send one; it still
/// shows and reminds (it may well be a client), but offers no Join until you accept it.
/// Without an organizer or emails to compare, it counts as yours.
fn unanswered_outside_invite(e: &Value) -> bool {
    let domain = |email: Option<&str>| {
        email
            .and_then(|m| m.rsplit_once('@'))
            .map(|(_, d)| d.to_ascii_lowercase())
    };
    let me = e
        .get("attendees")
        .and_then(|a| a.as_array())
        .and_then(|a| a.iter().find(|p| p.get("self").and_then(|s| s.as_bool()) == Some(true)));
    let mine = domain(me.and_then(|p| p.get("email")).and_then(|m| m.as_str()));
    let theirs = domain(e.pointer("/organizer/email").and_then(|m| m.as_str()));
    my_response(e) == Some("needsAction") && mine.is_some() && theirs.is_some() && mine != theirs
}

/// A link kwek will offer to join: https, on a known video meeting service. Anyone can put
/// any "video link" in an invite they send you; anything else counts as no video link. This
/// keeps lookalike and arbitrary sites out; it can't stop a stranger inviting you to a real
/// Meet or Zoom room of their own, which is no different from such a link in an email.
pub fn is_meeting_link(link: &str) -> bool {
    let Ok(u) = url::Url::parse(link) else {
        return false;
    };
    let Some(host) = u.host_str().map(|h| h.to_ascii_lowercase()) else {
        return false;
    };
    let under = |domain: &str| host == domain || host.ends_with(&format!(".{domain}"));
    u.scheme() == "https"
        && (host == "meet.google.com"
            || under("zoom.us")
            || host == "teams.microsoft.com"
            || host == "teams.live.com"
            || under("webex.com"))
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
        // A week ahead: the sidebar shows what's next even on a Friday afternoon, and the
        // agenda lists the coming days.
        let next = match self.api.upcoming(now, now + AHEAD).await {
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
            let body = match (&m.join_url, &m.location) {
                (Some(_), _) => "Join in your browser.".to_owned(),
                (None, Some(place)) => place.clone(),
                (None, None) => "No video link in the invite.".to_owned(),
            };
            // Only a Meet link is something to join; without one, the reminder just reminds.
            self.sink.notify(Notification {
                title,
                body,
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
            "attendees": [
                { "email": "tim@acme.nl", "self": true, "responseStatus": "accepted" },
                { "email": "noor@acme.nl", "responseStatus": "accepted" } ],
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
    fn only_real_meeting_links_are_joinable() {
        assert!(is_meeting_link("https://meet.google.com/aaa-bbbb-ccc"));
        assert!(is_meeting_link("https://acme.zoom.us/j/123"));
        assert!(is_meeting_link("https://teams.microsoft.com/l/meetup-join/x"));
        assert!(!is_meeting_link("http://meet.google.com/aaa-bbbb-ccc"), "https only");
        assert!(!is_meeting_link("https://meet.google.com.evil.example/x"));
        assert!(!is_meeting_link("https://evilzoom.us/j/1"));
        assert!(!is_meeting_link("javascript:alert(1)"));
        assert!(
            !is_meeting_link("https://meet.google.com@evil.example/x"),
            "userinfo trick"
        );
        assert!(
            !is_meeting_link("https://meet.google.com./x"),
            "trailing dot fails closed"
        );
        assert!(
            is_meeting_link("https://MEET.GOOGLE.COM/aaa-bbbb-ccc"),
            "host case doesn't matter"
        );
        assert!(is_meeting_link("https://meet.google.com:443/aaa-bbbb-ccc"));

        // An invite with an arbitrary "video link": shown, but nothing to join.
        let v = json!({ "items": [event("x", "2026-11-09T10:00:00+01:00", json!({
            "conferenceData": { "entryPoints": [{ "entryPointType": "video", "uri": "https://login.example/meet" }] } }))] });
        assert_eq!(parse_events(&v)[0].join_url, None);
    }

    #[test]
    fn unanswered_invites_from_outside_offer_no_join() {
        let me = |status: &str| json!({ "email": "tim@acme.nl", "self": true, "responseStatus": status });
        let meet = "https://meet.google.com/aaa-bbbb-ccc";
        let v = json!({ "items": [
            event("outside", "2026-11-09T10:00:00+01:00", json!({ "hangoutLink": meet,
                "organizer": { "email": "someone@stranger.example" }, "attendees": [me("needsAction")] })),
            event("outside-accepted", "2026-11-09T10:00:00+01:00", json!({ "hangoutLink": meet,
                "organizer": { "email": "someone@stranger.example" }, "attendees": [me("accepted")] })),
            event("colleague", "2026-11-09T10:00:00+01:00", json!({ "hangoutLink": meet,
                "organizer": { "email": "noor@acme.nl" }, "attendees": [me("needsAction")] })),
            event("no-organizer", "2026-11-09T10:00:00+01:00", json!({ "hangoutLink": meet,
                "attendees": [me("needsAction"), { "email": "noor@acme.nl" }] })),
        ]});
        let joins: Vec<(String, bool)> = parse_events(&v)
            .into_iter()
            .map(|m| (m.id, m.join_url.is_some()))
            .collect();
        assert_eq!(
            joins,
            vec![
                ("outside".into(), false),
                ("outside-accepted".into(), true),
                ("colleague".into(), true),
                ("no-organizer".into(), true),
            ],
            "all shown and reminded; only the unanswered outside one has nothing to join"
        );
    }

    #[test]
    fn only_events_with_other_people_count() {
        let me = json!({ "email": "tim@acme.nl", "self": true, "responseStatus": "accepted" });
        let room = json!({ "email": "room-3@resource.calendar.google.com", "resource": true });
        let v = json!({ "items": [
            event("with-noor", "2026-11-09T10:00:00+01:00", json!({ "location": "  Room 3.14\n Building B " })),
            event("no-guests", "2026-11-09T10:00:00+01:00", json!({ "attendees": null,
                "organizer": { "email": "tim@acme.nl", "self": true } })),
            event("just-me", "2026-11-09T10:00:00+01:00", json!({ "attendees": [me],
                "organizer": { "email": "tim@acme.nl", "self": true } })),
            event("me-and-a-room", "2026-11-09T10:00:00+01:00", json!({ "attendees": [me, room],
                "organizer": { "email": "tim@acme.nl", "self": true } })),
            event("hidden-guests", "2026-11-09T10:00:00+01:00", json!({ "attendees": [me],
                "organizer": { "email": "lieke@acme.nl" }, "location": "" })),
        ]});
        let m = parse_events(&v);
        let ids: Vec<&str> = m.iter().map(|x| x.id.as_str()).collect();
        assert_eq!(ids, vec!["with-noor", "hidden-guests"]);
        assert_eq!(m[0].location.as_deref(), Some("Room 3.14 Building B"));
        assert_eq!(m[1].location, None, "an empty location is no location");
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
            location: None,
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
