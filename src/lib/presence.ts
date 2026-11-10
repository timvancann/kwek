// Presence in words: the dot is never the only way a state is shown.
import type { PresenceView } from "./api";
import { ago, clock, dayKey, toDate } from "./format";

const STATE_WORDS: Record<string, string> = {
  active: "Active",
  idle: "Idle",
  away: "Away",
  dnd: "Do not disturb",
};

const shortDay = new Intl.DateTimeFormat("en-GB", { weekday: "short" });

/** "12:00" today, "Thu 09:00" on another day. */
export function untilText(us: number): string {
  return dayKey(us) === dayKey(Date.now() * 1000) ? clock(us) : `${shortDay.format(toDate(us))} ${clock(us)}`;
}

export function stateWords(state: string, dndUntilUs: number | null = null): string {
  const words = STATE_WORDS[state] ?? state;
  return state === "dnd" && dndUntilUs ? `${words} until ${untilText(dndUntilUs)}` : words;
}

/** "Active", "Do not disturb until 12:00", "Last seen 12 min ago"; null when kwek has
 * never heard from them (they don't run kwek). */
export function presenceWords(p: PresenceView | undefined): string | null {
  if (!p) return null;
  if (!p.state) return `Last seen ${ago(p.lastSeenUs)}`;
  return stateWords(p.state, p.dndUntilUs);
}

/** Local midnight tonight, for "busy for the rest of the day". */
export function endOfToday(): number {
  const d = new Date();
  return new Date(d.getFullYear(), d.getMonth(), d.getDate() + 1).getTime() * 1000;
}
