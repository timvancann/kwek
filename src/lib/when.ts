// Times to come, for reminders and send later. Microseconds since the epoch, like Rust.

const LOCALE = "en-GB";
const time = new Intl.DateTimeFormat(LOCALE, { hour: "2-digit", minute: "2-digit" });
const weekdayTime = new Intl.DateTimeFormat(LOCALE, { weekday: "short", hour: "2-digit", minute: "2-digit" });
const dateTime = new Intl.DateTimeFormat(LOCALE, { weekday: "short", day: "numeric", month: "short", hour: "2-digit", minute: "2-digit" });

const at9 = (d: Date) => new Date(d.getFullYear(), d.getMonth(), d.getDate(), 9, 0).getTime() * 1000;

/** The quick choices, as Slack offers them. */
export function presets(now = new Date()): Array<{ label: string; atUs: number }> {
  const ms = now.getTime();
  const tomorrow = new Date(now.getFullYear(), now.getMonth(), now.getDate() + 1);
  // The coming Monday; a week on when today is Monday.
  const monday = new Date(now.getFullYear(), now.getMonth(), now.getDate() + (((8 - now.getDay()) % 7) || 7));
  return [
    { label: "In 20 minutes", atUs: (ms + 20 * 60_000) * 1000 },
    { label: "In 1 hour", atUs: (ms + 3_600_000) * 1000 },
    { label: "In 3 hours", atUs: (ms + 3 * 3_600_000) * 1000 },
    { label: "Tomorrow at 09:00", atUs: at9(tomorrow) },
    { label: "Monday at 09:00", atUs: at9(monday) },
  ];
}

/** "today 15:30", "tomorrow 09:00", "Thu 10:00" within a week, else with the date. */
export function upcoming(us: number, now = new Date()): string {
  const d = new Date(us / 1000);
  const days = Math.round(
    (new Date(d.getFullYear(), d.getMonth(), d.getDate()).getTime() - new Date(now.getFullYear(), now.getMonth(), now.getDate()).getTime()) /
      86_400_000,
  );
  if (days === 0) return `today ${time.format(d)}`;
  if (days === 1) return `tomorrow ${time.format(d)}`;
  if (days > 1 && days < 7) return weekdayTime.format(d);
  return dateTime.format(d);
}

/** A `datetime-local` input's value for `us`, in local time. */
export function localInput(us: number): string {
  const d = new Date(us / 1000);
  const p = (n: number) => String(n).padStart(2, "0");
  return `${d.getFullYear()}-${p(d.getMonth() + 1)}-${p(d.getDate())}T${p(d.getHours())}:${p(d.getMinutes())}`;
}
