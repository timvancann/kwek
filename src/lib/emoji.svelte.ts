// Every Unicode emoji (emojibase), with Slack's shortcodes (iamcal), for `:` autocomplete and
// the emoji pickers. Unicode only: custom workspace emoji are not listable through the API.
// The data (about 600 KB) loads on its own at start, off the first render.

export type Emoji = {
  /** The character(s) to insert. */
  e: string;
  /** Shortcodes without colons, Slack's first; the CLDR label when there are none. */
  codes: string[];
  label: string;
  tags: string[];
  group: number;
};

/** Picker sections in emojibase's group order (2, skin tone components, is left out). */
export const GROUPS: Array<{ id: number; label: string }> = [
  { id: 0, label: "Smileys and emotion" },
  { id: 1, label: "People" },
  { id: 3, label: "Animals and nature" },
  { id: 4, label: "Food and drink" },
  { id: 5, label: "Travel and places" },
  { id: 6, label: "Activities" },
  { id: 7, label: "Objects" },
  { id: 8, label: "Symbols" },
  { id: 9, label: "Flags" },
];

/** The quick row until enough have been used to fill it. */
const QUICK = ["👍", "❤️", "😂", "🎉", "👀", "🙏", "✅", "🔥"];
const RECENT_KEY = "kwek.recentEmoji";

export const emoji = $state<{ list: Emoji[] }>({ list: [] });

void Promise.all([import("emojibase-data/en/compact.json"), import("emojibase-data/en/shortcodes/iamcal.json")]).then(
  ([data, slack]) => {
    const codes = slack.default as Record<string, string | string[]>;
    emoji.list = data.default
      .filter((d) => d.group !== undefined && d.group !== 2)
      .sort((a, b) => (a.order ?? 0) - (b.order ?? 0))
      .map((d) => {
        const c = codes[d.hexcode];
        return {
          e: d.unicode,
          codes: c === undefined ? [d.label.toLowerCase().replace(/[^a-z0-9]+/g, "_")] : Array.isArray(c) ? c : [c],
          label: d.label,
          tags: d.tags ?? [],
          group: d.group!,
        };
      });
  },
);

/** Best matches for `q` (no colons): shortcode prefix first, then anywhere in a shortcode,
 * then the label or a tag. */
export function searchEmoji(q: string, limit = 8): Emoji[] {
  const s = q.trim().toLowerCase().replace(/^:|:$/g, "");
  if (!s) return [];
  const rank = (x: Emoji) =>
    x.codes.some((c) => c.startsWith(s))
      ? 0
      : x.codes.some((c) => c.includes(s))
        ? 1
        : x.label.toLowerCase().split(/\W+/).some((w) => w.startsWith(s)) || x.tags.some((t) => t.startsWith(s))
          ? 2
          : 3;
  const out: Array<[number, Emoji]> = [];
  for (const x of emoji.list) {
    const r = rank(x);
    if (r < 3) out.push([r, x]);
  }
  // Stable: within a rank, emojibase's order (the common ones first).
  return out
    .sort((a, b) => a[0] - b[0])
    .slice(0, limit)
    .map(([, x]) => x);
}

function loadRecent(): string[] {
  try {
    const v = JSON.parse(localStorage.getItem(RECENT_KEY) ?? "[]");
    return Array.isArray(v) ? v.filter((x) => typeof x === "string") : [];
  } catch {
    return [];
  }
}

const recent = $state<{ list: string[] }>({ list: loadRecent() });

/** The quick row: the most recently used, topped up with the defaults. */
export function quickRow(n = 8): string[] {
  return [...new Set([...recent.list, ...QUICK])].slice(0, n);
}

/** Remembers `e` for the quick row (on this computer only). */
export function usedEmoji(e: string) {
  recent.list = [e, ...recent.list.filter((x) => x !== e)].slice(0, 24);
  try {
    localStorage.setItem(RECENT_KEY, JSON.stringify(recent.list));
  } catch {
    // Private storage unavailable: remembered until restart.
  }
}
