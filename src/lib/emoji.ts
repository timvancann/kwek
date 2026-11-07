// A compact emoji set for `:` autocomplete and the reaction picker. Unicode only: custom
// workspace emoji are not listable through the API.
export const EMOJI: Array<[string, string]> = [
  ["thumbsup", "👍"], ["thumbsdown", "👎"], ["heart", "❤️"], ["joy", "😂"], ["smile", "😄"],
  ["slightly_smiling_face", "🙂"], ["wink", "😉"], ["grin", "😁"], ["laughing", "😆"], ["sweat_smile", "😅"],
  ["rofl", "🤣"], ["thinking", "🤔"], ["eyes", "👀"], ["tada", "🎉"], ["fire", "🔥"],
  ["rocket", "🚀"], ["100", "💯"], ["white_check_mark", "✅"], ["heavy_check_mark", "✔️"], ["x", "❌"],
  ["warning", "⚠️"], ["pray", "🙏"], ["clap", "👏"], ["raised_hands", "🙌"], ["muscle", "💪"],
  ["ok_hand", "👌"], ["wave", "👋"], ["point_up", "☝️"], ["point_right", "👉"], ["handshake", "🤝"],
  ["sob", "😭"], ["cry", "😢"], ["disappointed", "😞"], ["confused", "😕"], ["neutral_face", "😐"],
  ["expressionless", "😑"], ["unamused", "😒"], ["roll_eyes", "🙄"], ["grimacing", "😬"], ["scream", "😱"],
  ["open_mouth", "😮"], ["astonished", "😲"], ["flushed", "😳"], ["sunglasses", "😎"], ["nerd", "🤓"],
  ["heart_eyes", "😍"], ["kissing_heart", "😘"], ["hugs", "🤗"], ["shrug", "🤷"], ["facepalm", "🤦"],
  ["sleeping", "😴"], ["zzz", "💤"], ["coffee", "☕"], ["beer", "🍺"], ["pizza", "🍕"],
  ["star", "⭐"], ["sparkles", "✨"], ["bulb", "💡"], ["memo", "📝"], ["calendar", "📅"],
  ["bug", "🐛"], ["wrench", "🔧"], ["hammer", "🔨"], ["lock", "🔒"], ["key", "🔑"],
  ["chart", "📈"], ["moneybag", "💰"], ["hourglass", "⏳"], ["alarm", "⏰"], ["phone", "📞"],
  ["email", "📧"], ["link", "🔗"], ["pushpin", "📌"], ["question", "❓"], ["exclamation", "❗"],
  ["plus1", "👍"], ["duck", "🦆"], ["party", "🥳"], ["melting", "🫠"], ["salute", "🫡"],
];

/** The quick row in the reaction picker. */
export const QUICK = ["👍", "❤️", "😂", "🎉", "👀", "🙏", "✅", "🔥"];

export function searchEmoji(q: string, limit = 8): Array<[string, string]> {
  const s = q.toLowerCase();
  const starts = EMOJI.filter(([n]) => n.startsWith(s));
  const contains = EMOJI.filter(([n]) => !n.startsWith(s) && n.includes(s));
  const seen = new Set<string>();
  return [...starts, ...contains].filter(([, e]) => (seen.has(e) ? false : (seen.add(e), true))).slice(0, limit);
}
