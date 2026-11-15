// The composer's formatting: each function turns the text and its selection into one edit in
// Chat syntax (what Google Chat itself accepts), so the toolbar and the shortcuts stay a
// plain textarea underneath. A second press on formatted text takes the formatting off.

/** Replace [start, end) with `insert`, then select [selStart, selEnd). */
export type Edit = { start: number; end: number; insert: string; selStart: number; selEnd: number };

/** `*bold*`, `_italic_`, `~strike~`, `` `code` ``. */
export function wrap(text: string, s: number, e: number, mark: string, placeholder: string): Edit {
  const sel = text.slice(s, e);
  const n = mark.length;
  if (s >= n && text.slice(s - n, s) === mark && text.slice(e, e + n) === mark) {
    return { start: s - n, end: e + n, insert: sel, selStart: s - n, selEnd: e - n };
  }
  if (sel.length > 2 * n && sel.startsWith(mark) && sel.endsWith(mark)) {
    const inner = sel.slice(n, -n);
    return { start: s, end: e, insert: inner, selStart: s, selEnd: s + inner.length };
  }
  const body = sel || placeholder;
  return { start: s, end: e, insert: mark + body + mark, selStart: s + n, selEnd: s + n + body.length };
}

/** Every selected line gets `prefix` ("- ", "> "), or loses it when all have it. */
export function prefixLines(text: string, s: number, e: number, prefix: string): Edit {
  const start = s === 0 ? 0 : text.lastIndexOf("\n", s - 1) + 1;
  const nl = text.indexOf("\n", e > s && text[e - 1] === "\n" ? e - 1 : e);
  const end = nl === -1 ? text.length : nl;
  const lines = text.slice(start, end).split("\n");
  const all = lines.every((l) => l.startsWith(prefix));
  const insert = lines.map((l) => (all ? l.slice(prefix.length) : prefix + l)).join("\n");
  return { start, end, insert, selStart: start, selEnd: start + insert.length };
}

/** A ``` block on lines of its own around the selection. */
export function codeBlock(text: string, s: number, e: number): Edit {
  const sel = text.slice(s, e);
  const m = /^```\n?([\s\S]*?)\n?```$/.exec(sel);
  if (m) return { start: s, end: e, insert: m[1], selStart: s, selEnd: s + m[1].length };
  const before = s > 0 && text[s - 1] !== "\n" ? "\n" : "";
  const after = e < text.length && text[e] !== "\n" ? "\n" : "";
  const body = sel || "code";
  const open = `${before}\`\`\`\n`;
  return { start: s, end: e, insert: `${open}${body}\n\`\`\`${after}`, selStart: s + open.length, selEnd: s + open.length + body.length };
}

const URL_RE = /^(https?:\/\/|mailto:)\S+$/;
export const isUrl = (s: string) => URL_RE.test(s.trim());

/** `<url|label>`. With a URL selected it becomes the target and the label is selected next;
 * otherwise the selection is the label and the URL part is selected to type over. */
export function link(text: string, s: number, e: number, url?: string): Edit {
  const sel = text.slice(s, e);
  if (url) {
    const label = sel || url;
    const insert = `<${url}|${label}>`;
    return { start: s, end: e, insert, selStart: s + insert.length, selEnd: s + insert.length };
  }
  if (isUrl(sel)) {
    const insert = `<${sel.trim()}|link text>`;
    const at = s + sel.trim().length + 2;
    return { start: s, end: e, insert, selStart: at, selEnd: at + "link text".length };
  }
  const label = sel || "link text";
  const insert = `<https://|${label}>`;
  return { start: s, end: e, insert, selStart: s + 1, selEnd: s + "<https://".length };
}
