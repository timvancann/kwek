// Time and date formatting. Times are microseconds since the epoch (from Rust).

// The interface copy is English, so dates are too (a Dutch system locale would otherwise
// mix "donderdag" into English sentences). en-GB keeps a 24-hour clock and day-month order.
const LOCALE = "en-GB";
const time = new Intl.DateTimeFormat(LOCALE, { hour: "2-digit", minute: "2-digit" });
const weekday = new Intl.DateTimeFormat(LOCALE, { weekday: "long", day: "numeric", month: "long" });
const fullDate = new Intl.DateTimeFormat(LOCALE, { weekday: "long", day: "numeric", month: "long", year: "numeric" });
const longStamp = new Intl.DateTimeFormat(LOCALE, { dateStyle: "full", timeStyle: "short" });

export const toDate = (us: number) => new Date(us / 1000);

export function clock(us: number): string {
  return time.format(toDate(us));
}

export function stamp(us: number): string {
  return longStamp.format(toDate(us));
}

function startOfDay(d: Date): number {
  return new Date(d.getFullYear(), d.getMonth(), d.getDate()).getTime();
}

export function dayKey(us: number): number {
  return startOfDay(toDate(us));
}

export function dayLabel(us: number): string {
  const d = toDate(us);
  const today = startOfDay(new Date());
  const diff = Math.round((today - startOfDay(d)) / 86_400_000);
  if (diff === 0) return "Today";
  if (diff === 1) return "Yesterday";
  return d.getFullYear() === new Date().getFullYear() ? weekday.format(d) : fullDate.format(d);
}

/** "now", "5 min", "3 h", "yesterday", or a date: for thread summaries and search hits. */
export function ago(us: number): string {
  const s = (Date.now() - us / 1000) / 1000;
  if (s < 60) return "just now";
  if (s < 3600) return `${Math.floor(s / 60)} min ago`;
  if (s < 86_400) {
    const h = Math.floor(s / 3600);
    return `${h} ${h === 1 ? "hour" : "hours"} ago`;
  }
  return dayLabel(us).toLowerCase() === "yesterday" ? "yesterday" : dayLabel(us);
}

const shortDay = new Intl.DateTimeFormat(LOCALE, { weekday: "short" });

/** Days from `now` to `us` (0 the same day, 1 the next), by calendar day. */
function daysAhead(us: number, now: number): number {
  return Math.round((startOfDay(toDate(us)) - startOfDay(toDate(now))) / 86_400_000);
}

/** When a meeting is, seen from `now`: "now, 25 min left", "now, until 15:30", "in 4 min",
 * "at 14:30", "tomorrow 09:30", "Thu 10:00". */
export function meetingWhen(startUs: number, endUs: number, now: number): string {
  if (startUs <= now && now < endUs) {
    const left = Math.ceil((endUs - now) / 60e6);
    return left < 60 ? `now, ${left} min left` : `now, until ${clock(endUs)}`;
  }
  const min = Math.round((startUs - now) / 60e6);
  if (min < 60) return `in ${Math.max(1, min)} min`;
  const d = daysAhead(startUs, now);
  if (d === 0) return `at ${clock(startUs)}`;
  if (d === 1) return `tomorrow ${clock(startUs)}`;
  return `${shortDay.format(toDate(startUs))} ${clock(startUs)}`;
}

/** A heading for a day of meetings: "Today", "Tomorrow", "Thursday 12 November". */
export function agendaDay(us: number, now: number): string {
  const d = daysAhead(us, now);
  if (d === 0) return "Today";
  if (d === 1) return "Tomorrow";
  return weekday.format(toDate(us));
}

export function initials(name: string): string {
  const parts = name.trim().split(/\s+/).filter(Boolean);
  if (parts.length === 0) return "?";
  return ((parts[0][0] ?? "") + (parts.length > 1 ? (parts[parts.length - 1][0] ?? "") : "")).toUpperCase();
}
