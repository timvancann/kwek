//! Presence: your own Google Chat status, and colleagues' as their kwek broadcasts it.
//!
//! Google's availability API only covers the signed-in user, so every kwek reads its own
//! user's state, keeps it ACTIVE while they're at their computer (kwek's API calls don't
//! count as activity), and publishes it on a Pub/Sub topic the other kwek users read.
//! See docs/superpowers/specs/2026-11-10-presence-design.md.

use serde::{Deserialize, Serialize};
use unicode_segmentation::UnicodeSegmentation;

use crate::ts::{self, MINUTE, Micros};
use crate::views::{CustomStatusView, PresenceView};

/// How long one "active" mark lasts at Google.
pub const MARK_TTL: Micros = 5 * MINUTE;
/// Marks are renewed this often while the person is here.
pub const MARK_EVERY: Micros = 2 * MINUTE;
/// No keyboard or mouse for this long and the person is no longer here.
pub const IDLE_AFTER_SECS: u64 = 5 * 60;
/// A colleague's kwek quiet for this long: no dot, "last seen" instead.
pub const STALE_AFTER: Micros = 3 * MINUTE;
/// Messages older than this (by the sender's clock) are ignored.
const MAX_AGE: Micros = 10 * MINUTE;
/// ...and so are ones from more than this far in the future: a clock that's off.
const MAX_AHEAD: Micros = 2 * MINUTE;
const MAX_TEXT: usize = 100;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    Active,
    Idle,
    Away,
    Dnd,
}

impl State {
    /// From the API's (and the topic's) names.
    pub fn from_api(s: &str) -> Option<Self> {
        Some(match s {
            "ACTIVE" => State::Active,
            "IDLE" => State::Idle,
            "AWAY" => State::Away,
            "DO_NOT_DISTURB" => State::Dnd,
            _ => return None,
        })
    }

    pub fn api(self) -> &'static str {
        match self {
            State::Active => "ACTIVE",
            State::Idle => "IDLE",
            State::Away => "AWAY",
            State::Dnd => "DO_NOT_DISTURB",
        }
    }

    /// For the webview.
    pub fn short(self) -> &'static str {
        match self {
            State::Active => "active",
            State::Idle => "idle",
            State::Away => "away",
            State::Dnd => "dnd",
        }
    }
}

// --------------------------------------------------------------- own presence

pub struct DecideIn {
    /// Keyboard or mouse used in the last few minutes, anywhere on the machine.
    pub here: bool,
    pub google: State,
    /// The person set away themselves; kwek leaves it alone.
    pub manual_away: bool,
    /// When kwek last marked them active.
    pub last_mark: Option<Micros>,
    pub now: Micros,
}

#[derive(Debug, PartialEq, Eq)]
pub struct DecideOut {
    pub mark_active: bool,
    pub manual_away: bool,
}

/// What to do about the person's own state, once per tick. kwek never touches do not
/// disturb or an away the person chose. It can't ask Google whether an away was manual, so
/// it infers it: an away that shows up while kwek's own active mark should still be running
/// was set by hand. Any other away (the mark ran out overnight, or after a break) is
/// automatic, and kwek marks active again once the person is back.
pub fn decide(i: &DecideIn) -> DecideOut {
    let mark_running = i.last_mark.is_some_and(|t| i.now < t + MARK_TTL);
    let due = i.last_mark.is_none_or(|t| i.now - t >= MARK_EVERY);
    match i.google {
        // Do not disturb replaces any away; when it ends Google moves to away by itself,
        // and that away is automatic.
        State::Dnd => DecideOut {
            mark_active: false,
            manual_away: false,
        },
        State::Away if i.manual_away || mark_running => DecideOut {
            mark_active: false,
            manual_away: true,
        },
        State::Away => DecideOut {
            mark_active: i.here,
            manual_away: false,
        },
        State::Active | State::Idle => DecideOut {
            mark_active: i.here && due,
            manual_away: false,
        },
    }
}

// --------------------------------------------------------------- the topic

/// One message on the presence topic (spec §3.3).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PresenceMsg {
    pub v: u32,
    pub user: String,
    pub state: String,
    #[serde(default)]
    pub dnd_until: Option<String>,
    #[serde(default)]
    pub custom: Option<CustomMsg>,
    pub sent_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CustomMsg {
    pub emoji: String,
    pub text: String,
    #[serde(default)]
    pub expire_time: Option<String>,
}

/// The person's own availability as Google reports it.
#[derive(Debug, Clone, PartialEq)]
pub struct Own {
    pub state: State,
    pub dnd_until: Option<Micros>,
    pub custom: Option<CustomStatusView>,
}

impl PresenceMsg {
    pub fn own(me: &str, own: &Own, now: Micros) -> Self {
        PresenceMsg {
            v: 1,
            user: me.to_owned(),
            state: own.state.api().to_owned(),
            dnd_until: own.dnd_until.map(ts::format),
            custom: own.custom.as_ref().map(|c| CustomMsg {
                emoji: c.emoji.clone(),
                text: c.text.clone(),
                expire_time: c.until_us.map(ts::format),
            }),
            sent_at: ts::format(now),
        }
    }
}

/// What kwek holds for a colleague.
#[derive(Debug, Clone, PartialEq)]
pub struct Held {
    pub user: String,
    pub state: State,
    pub dnd_until: Option<Micros>,
    pub custom: Option<CustomStatusView>,
    /// By the sender's clock: orders their messages.
    pub sent: Micros,
    /// By this machine's clock: says whether their kwek is still running.
    pub received: Micros,
}

/// A message from the topic, if it is one to keep. Everything in it is untrusted: anyone
/// with publish rights on the topic could have sent it (spec D5).
pub fn accept(data: &[u8], knows: impl Fn(&str) -> bool, me: &str, now: Micros, held: Option<&Held>) -> Option<Held> {
    let m: PresenceMsg = serde_json::from_slice(data).ok()?;
    if m.v != 1 || m.user == me || !knows(&m.user) {
        return None;
    }
    let state = State::from_api(&m.state)?;
    let sent = ts::parse(&m.sent_at)?;
    if sent < now - MAX_AGE || sent > now + MAX_AHEAD || held.is_some_and(|h| sent <= h.sent) {
        return None;
    }
    let custom = m.custom.and_then(|c| {
        let emoji = c.emoji.graphemes(true).next().unwrap_or("").to_owned();
        let text: String = c.text.chars().take(MAX_TEXT).collect();
        let text = text.trim().to_owned();
        (!text.is_empty() || !emoji.is_empty()).then(|| CustomStatusView {
            emoji,
            text,
            until_us: c.expire_time.as_deref().and_then(ts::parse),
        })
    });
    Some(Held {
        user: m.user,
        state,
        dnd_until: m.dnd_until.as_deref().and_then(ts::parse),
        custom,
        sent,
        received: now,
    })
}

/// How a colleague shows right now: no dot once their kwek has gone quiet, no custom status
/// once it has expired.
pub fn view_of(h: &Held, now: Micros) -> PresenceView {
    let fresh = now - h.received < STALE_AFTER;
    PresenceView {
        user: h.user.clone(),
        state: fresh.then(|| h.state.short().to_owned()),
        last_seen_us: h.received,
        dnd_until_us: h.dnd_until.filter(|_| fresh && h.state == State::Dnd),
        custom: h.custom.clone().filter(|c| c.until_us.is_none_or(|u| now < u)),
    }
}

/// Idle seconds to here or not.
pub fn is_here(idle_secs: u64) -> bool {
    idle_secs < IDLE_AFTER_SECS
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ts::SECOND;

    const NOW: Micros = 1_800_000_000 * SECOND;

    fn d(here: bool, google: State, manual_away: bool, last_mark: Option<Micros>) -> DecideOut {
        decide(&DecideIn {
            here,
            google,
            manual_away,
            last_mark,
            now: NOW,
        })
    }

    fn out(mark_active: bool, manual_away: bool) -> DecideOut {
        DecideOut {
            mark_active,
            manual_away,
        }
    }

    #[test]
    fn here_and_active_keeps_marking_but_not_every_tick() {
        assert_eq!(d(true, State::Active, false, None), out(true, false));
        assert_eq!(d(true, State::Idle, false, Some(NOW - 3 * MINUTE)), out(true, false));
        assert_eq!(
            d(true, State::Active, false, Some(NOW - MINUTE)),
            out(false, false),
            "marked a minute ago"
        );
    }

    #[test]
    fn gone_does_nothing() {
        assert_eq!(
            d(false, State::Active, false, Some(NOW - 3 * MINUTE)),
            out(false, false)
        );
        assert_eq!(d(false, State::Away, false, None), out(false, false));
    }

    #[test]
    fn away_while_the_mark_runs_was_set_by_hand_and_holds() {
        // kwek marked a minute ago; Google says away: only a person does that.
        assert_eq!(d(true, State::Away, false, Some(NOW - MINUTE)), out(false, true));
        // Hours later, still away and remembered as manual: left alone.
        assert_eq!(
            d(true, State::Away, true, Some(NOW - 3 * 60 * MINUTE)),
            out(false, true)
        );
    }

    #[test]
    fn away_after_the_night_is_automatic() {
        // kwek's last mark ran out long ago (the laptop slept); back at the keyboard.
        assert_eq!(
            d(true, State::Away, false, Some(NOW - 10 * 60 * MINUTE)),
            out(true, false)
        );
        assert_eq!(d(true, State::Away, false, None), out(true, false), "first start");
    }

    #[test]
    fn do_not_disturb_is_never_touched_and_ends_manual_away() {
        assert_eq!(d(true, State::Dnd, false, None), out(false, false));
        assert_eq!(d(true, State::Dnd, true, Some(NOW - 3 * MINUTE)), out(false, false));
    }

    #[test]
    fn active_in_a_google_client_clears_manual_away() {
        assert_eq!(d(true, State::Active, true, Some(NOW - 3 * MINUTE)), out(true, false));
    }

    const ME: &str = "users/me";
    const SAM: &str = "users/sam";

    fn knows(u: &str) -> bool {
        u == SAM || u == ME
    }

    fn msg(user: &str, state: &str, sent: Micros) -> Vec<u8> {
        serde_json::to_vec(&serde_json::json!({
            "v": 1, "user": user, "state": state, "sentAt": ts::format(sent),
            "custom": { "emoji": "🎧", "text": "Deep work", "expireTime": ts::format(NOW + 60 * MINUTE) }
        }))
        .unwrap()
    }

    #[test]
    fn keeps_a_good_message() {
        let h = accept(&msg(SAM, "DO_NOT_DISTURB", NOW - SECOND), knows, ME, NOW, None).unwrap();
        assert_eq!(h.state, State::Dnd);
        assert_eq!(h.received, NOW);
        let c = h.custom.unwrap();
        assert_eq!((c.emoji.as_str(), c.text.as_str()), ("🎧", "Deep work"));
        assert_eq!(c.until_us, Some(NOW + 60 * MINUTE));
    }

    #[test]
    fn drops_what_it_should_not_trust() {
        let none = |data: &[u8], held: Option<&Held>| accept(data, knows, ME, NOW, held).is_none();
        assert!(none(&msg("users/stranger", "ACTIVE", NOW), None), "unknown user");
        assert!(none(&msg(ME, "ACTIVE", NOW), None), "our own message comes back too");
        assert!(none(&msg(SAM, "ONLINE", NOW), None), "no such state");
        assert!(none(&msg(SAM, "ACTIVE", NOW - 11 * MINUTE), None), "too old");
        assert!(none(&msg(SAM, "ACTIVE", NOW + 5 * MINUTE), None), "from the future");
        assert!(none(b"{not json", None));
        assert!(none(
            br#"{"v":2,"user":"users/sam","state":"ACTIVE","sentAt":"2027-01-15T08:00:00Z"}"#,
            None
        ));
        let newer = accept(&msg(SAM, "ACTIVE", NOW - SECOND), knows, ME, NOW, None).unwrap();
        assert!(none(&msg(SAM, "AWAY", NOW - 2 * SECOND), Some(&newer)), "out of order");
    }

    #[test]
    fn caps_what_it_shows() {
        let long = "x".repeat(500);
        let data = serde_json::to_vec(&serde_json::json!({
            "v": 1, "user": SAM, "state": "ACTIVE", "sentAt": ts::format(NOW),
            "custom": { "emoji": "👩‍👩‍👧‍👦🎉🎉", "text": long }
        }))
        .unwrap();
        let c = accept(&data, knows, ME, NOW, None).unwrap().custom.unwrap();
        assert_eq!(c.text.chars().count(), MAX_TEXT);
        assert_eq!(c.emoji, "👩‍👩‍👧‍👦", "one emoji, even a joined one");
    }

    #[test]
    fn goes_quiet_after_three_minutes_and_status_expires() {
        let h = accept(&msg(SAM, "ACTIVE", NOW), knows, ME, NOW, None).unwrap();
        assert_eq!(view_of(&h, NOW + STALE_AFTER - SECOND).state.as_deref(), Some("active"));
        let quiet = view_of(&h, NOW + STALE_AFTER);
        assert_eq!(quiet.state, None);
        assert_eq!(quiet.last_seen_us, NOW);
        assert!(view_of(&h, NOW + 59 * MINUTE).custom.is_some());
        assert!(view_of(&h, NOW + 60 * MINUTE).custom.is_none());
    }

    #[test]
    fn own_message_round_trips() {
        let own = Own {
            state: State::Away,
            dnd_until: None,
            custom: Some(CustomStatusView {
                emoji: "🌴".into(),
                text: "Back Monday".into(),
                until_us: Some(NOW + 60 * MINUTE),
            }),
        };
        let data = serde_json::to_vec(&PresenceMsg::own(SAM, &own, NOW)).unwrap();
        let h = accept(&data, knows, ME, NOW, None).unwrap();
        assert_eq!((h.state, h.custom), (State::Away, own.custom));
    }
}
