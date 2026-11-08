// UI state. A view over Rust: reads come from commands, changes arrive as events. Nothing
// here talks to Google or computes unread counts; Rust does.
import { toast } from "svelte-sonner";
import {
  api,
  errorText,
  on,
  type AuthState,
  type CalendarView,
  type DeletePayload,
  type InAppNotice,
  type MessageView,
  type NavPayload,
  type SpaceSummary,
  type SyncStatus,
  type UpsertPayload,
} from "./api";
import type { Block } from "./bindings/Block";
import type { Inline } from "./bindings/Inline";

export type Zone = "sidebar" | "messages" | "composer" | "thread" | "threadComposer";
export type SpaceSort = "recent" | "az" | "custom";

function loadSpaceSort(): SpaceSort {
  try {
    const v = localStorage.getItem("kwek.spaceSort");
    if (v === "recent" || v === "az" || v === "custom") return v;
  } catch {
    // Private storage unavailable: start with the default.
  }
  return "recent";
}

/** `xs` arrive by recent activity. Custom: placed spaces by position, the rest after them
 * by activity. */
function sortSpaces(xs: SpaceSummary[], sort: SpaceSort): SpaceSummary[] {
  if (sort === "az") return [...xs].sort((a, b) => a.displayName.localeCompare(b.displayName));
  if (sort === "custom") {
    const placed = xs.filter((s) => s.position !== null).sort((a, b) => a.position! - b.position!);
    return [...placed, ...xs.filter((s) => s.position === null)];
  }
  return xs;
}
export type Overlay = null | "switcher" | "search" | "keys";

const PAGE = 100;

function byTime(a: MessageView, b: MessageView) {
  return a.createUs - b.createUs || a.name.localeCompare(b.name);
}

function upsertInto(list: MessageView[], m: MessageView): MessageView[] {
  const i = list.findIndex((x) => x.name === m.name);
  if (i >= 0) {
    list[i] = m;
    return list;
  }
  const out = [...list, m];
  // New messages almost always land at the end; sort only when they do not.
  if (out.length > 1 && byTime(out[out.length - 2], m) > 0) out.sort(byTime);
  return out;
}

class AppState {
  auth = $state<AuthState>({ state: "signedOut" });
  ready = $state(false);
  status = $state<SyncStatus>({ state: "starting", detail: null });
  spaces = $state<SpaceSummary[]>([]);
  current = $state<string | null>(null);
  messages = $state<Record<string, MessageView[]>>({});
  loadingOlder = $state(false);
  thread = $state<{ name: string; space: string; messages: MessageView[] } | null>(null);
  /** Keyboard selection in the channel and the thread panel. */
  selected = $state<string | null>(null);
  threadSelected = $state<string | null>(null);
  zone = $state<Zone>("composer");
  /** The conversation highlighted by j/k in the sidebar (a name, so reordering keeps it). */
  sidebarFocus = $state<string | null>(null);
  overlay = $state<Overlay>(null);
  /** Where the read marker sat when the space was opened ("New" divider). */
  newMarker = $state<Record<string, number | null>>({});
  /** A message to scroll to and select once its space is open (search, notifications). */
  jumpTo = $state<string | null>(null);
  /** Composer edit mode: the message being edited, per composer. */
  editing = $state<{ name: string; text: string; where: "channel" | "thread" } | null>(null);
  confirmDelete = $state<MessageView | null>(null);
  confirmSignOut = $state(false);
  reactTo = $state<MessageView | null>(null);
  /** Meetings from the user's calendar, and whether it can be read. */
  calendar = $state<CalendarView>({ state: "off", detail: null, meetings: [] });
  /** The image viewer: a message's images, opened at `index` (among its images). */
  viewing = $state<{ message: MessageView; index: number; from?: HTMLElement } | null>(null);
  announce = $state("");
  windowFocused = $state(true);
  /** Bumped to ask the active zone to take focus again (after a dialog closes). */
  focusNonce = $state(0);
  /** Collapsed sidebar sections; Hidden starts folded away. */
  collapsed = $state<Record<string, boolean>>({ hidden: true });
  /** Window under 1000px: the thread panel overlays the channel. */
  narrow = $state(false);
  /** How the Spaces section is sorted; remembered on this Mac. */
  spaceSort = $state<SpaceSort>(loadSpaceSort());

  currentSpace = $derived(this.spaces.find((s) => s.name === this.current) ?? null);
  currentMessages = $derived(this.current ? (this.messages[this.current] ?? []) : []);

  /** Sidebar order: Starred, Direct messages (DMs and group chats), Spaces, then Hidden.
   * DMs by activity; Spaces by the chosen sort. */
  sections = $derived.by(() => {
    const shown = this.spaces.filter((s) => !s.hidden);
    const starred = shown.filter((s) => s.starred);
    const dms = shown.filter((s) => !s.starred && s.kind !== "space");
    const rooms = shown.filter((s) => !s.starred && s.kind === "space");
    const hidden = [...this.spaces.filter((s) => s.hidden)].sort((a, b) => a.displayName.localeCompare(b.displayName));
    return [
      { id: "starred", title: "Starred", items: starred },
      { id: "dms", title: "Direct messages", items: dms },
      { id: "spaces", title: "Spaces", items: sortSpaces(rooms, this.spaceSort) },
      { id: "hidden", title: "Hidden", items: hidden },
    ].filter((s) => s.items.length > 0);
  });
  /** What the sidebar shows, in order: j/k walk exactly this. */
  sidebarOrder = $derived(this.sections.flatMap((s) => (this.collapsed[s.id] ? [] : s.items)));

  /** Screen reader announcement; identical consecutive texts are announced again. */
  say(text: string) {
    this.announce = "";
    setTimeout(() => (this.announce = text), 50);
  }

  async init() {
    await Promise.all([
      on<SpaceSummary[]>("spaces:changed", (s) => (this.spaces = s)),
      on<UpsertPayload>("messages:upserted", (p) => this.onUpsert(p)),
      on<DeletePayload>("messages:deleted", (p) => this.onDelete(p)),
      on<SyncStatus>("sync:status", (s) => {
        const was = this.status.state;
        this.status = s;
        if (s.state === was) return;
        if (s.state === "offline") this.say("Offline. Showing cached messages.");
        else if (s.state === "authRequired") this.say("Signed out of Google.");
        else if (s.state === "error") this.say("Sync stopped. kwek keeps retrying.");
        else if (s.state === "live" && (was === "offline" || was === "error")) this.say("Back online.");
      }),
      on<AuthState>("auth:changed", (a) => {
        const wasIn = this.auth.state === "signedIn";
        this.auth = a;
        if (a.state === "signedIn" && !wasIn) void this.loadSpaces();
        if (a.state === "signedOut") this.reset();
      }),
      on<NavPayload>("nav:open", (n) => void this.open(n.space, n.message ?? null, n.thread ?? null)),
      // While kwek is in front, DMs and mentions elsewhere are a quiet pop-up, not a system
      // notification.
      on<InAppNotice>("notify:inapp", (n) =>
        toast(n.title, {
          description: n.body,
          // A call or meeting waits longer for its Join than a message for its Open.
          duration: n.url ? 60_000 : 8000,
          action: n.url
            ? { label: "Join", onClick: () => void api.openUrl(n.url!) }
            : { label: "Open", onClick: () => void this.open(n.space, n.message ?? null, n.thread ?? null) },
        }),
      ),
      on<CalendarView>("calendar:changed", (c) => (this.calendar = c)),
    ]);
    window.addEventListener("focus", () => {
      this.windowFocused = true;
      this.maybeMarkRead();
    });
    window.addEventListener("blur", () => (this.windowFocused = false));
    this.auth = await api.authStatus();
    this.status = await api.syncStatus();
    this.calendar = await api.calendar().catch(() => this.calendar);
    if (this.auth.state === "signedIn") await this.loadSpaces();
    this.ready = true;
  }

  private reset() {
    this.spaces = [];
    this.messages = {};
    this.current = null;
    this.thread = null;
    this.selected = null;
  }

  async loadSpaces() {
    this.spaces = await api.listSpaces();
    if (!this.current && this.spaces.length > 0) {
      await this.open(this.spaces[0].name);
    }
  }

  async open(space: string, message: string | null = null, thread: string | null = null) {
    const s = this.spaces.find((x) => x.name === space);
    if (this.current !== space) {
      this.newMarker[space] = s?.unread ? (s.lastReadUs ?? null) : null;
      this.current = space;
      this.editing = null;
      this.selected = null;
      // Unknown until the list has restored its scroll position; it reports back.
      this.atBottom = false;
      this.sidebarFocus = space;
      if (this.thread && this.thread.space !== space) this.closeThread();
      try {
        this.messages[space] = await api.getMessages(space, null, PAGE);
      } catch (e) {
        toast.error(errorText(e));
      }
      void api.setOpen(space, this.thread?.name ?? null);
    }
    if (thread) await this.openThread(thread);
    if (message && (await this.reach(space, message))) {
      this.jumpTo = message;
      this.selected = message;
      this.zone = "messages";
    }
  }

  /** Opens a thread with one of its messages selected and focused (a search hit). */
  async openInThread(space: string, thread: string, message: string) {
    await this.open(space, null, thread);
    if (this.thread?.name !== thread) return;
    this.threadSelected = message;
    this.zone = "thread";
    this.focusNonce++;
  }

  /** Extends the loaded list back from the cache until it holds `message` (a search hit or
   * notification can point far back), so the list can scroll to it. False when it is not
   * there (deleted, or further back than the list may grow); says so. */
  private async reach(space: string, message: string): Promise<boolean> {
    // The list is not virtualised; past this it gets slow to render.
    const max = 3000;
    const has = () => (this.messages[space] ?? []).some((m) => m.name === message);
    if (has()) return true;
    this.loadingOlder = true;
    this.say("Loading older messages");
    try {
      while (this.current === space && !has()) {
        const list = this.messages[space] ?? [];
        if (list.length === 0 || list.length >= max) break;
        const older = await api.getMessages(space, list[0].createUs, 500);
        if (older.length === 0 || this.current !== space) break;
        // Merge into the live list: messages may have arrived meanwhile.
        const cur = this.messages[space] ?? [];
        const names = new Set(cur.map((m) => m.name));
        this.messages[space] = [...older.filter((m) => !names.has(m.name)), ...cur];
      }
    } catch (e) {
      toast.error(errorText(e));
      return false;
    } finally {
      this.loadingOlder = false;
    }
    if (has()) return true;
    if (this.current === space) toast.error("That message is too far back to open here.");
    return false;
  }

  async loadOlder() {
    const space = this.current;
    if (!space || this.loadingOlder) return;
    const list = this.messages[space] ?? [];
    this.loadingOlder = true;
    try {
      const oldest = list[0]?.createUs ?? null;
      let older = oldest ? await api.getMessages(space, oldest, PAGE) : [];
      // The cache is contiguous only down to the low-water mark. A short page, or one that
      // reaches below the mark, may hide a gap: page the API back first, then read again.
      const lw = this.currentSpace?.lowWaterUs ?? null;
      const crossesGap = older.length < PAGE || (lw !== null && older[0] !== undefined && older[0].createUs < lw);
      if (crossesGap && !this.currentSpace?.historyComplete) {
        const n = await api.loadOlder(space);
        if (n > 0 && oldest) older = await api.getMessages(space, oldest, PAGE);
      }
      if (older.length > 0) {
        const names = new Set((this.messages[space] ?? []).map((m) => m.name));
        this.messages[space] = [...older.filter((m) => !names.has(m.name)), ...(this.messages[space] ?? [])];
      }
    } catch (e) {
      toast.error(errorText(e));
    } finally {
      this.loadingOlder = false;
    }
  }

  async openThread(thread: string) {
    const space = thread.split("/").slice(0, 2).join("/");
    try {
      const messages = await api.getThread(thread);
      this.thread = { name: thread, space, messages };
      this.threadSelected = null;
      this.zone = "threadComposer";
      void api.setOpen(this.current, thread);
      const last = messages[messages.length - 1];
      if (last) void api.markThreadRead(thread, last.createUs);
    } catch (e) {
      toast.error(errorText(e));
    }
  }

  closeThread() {
    this.thread = null;
    this.threadSelected = null;
    // Focus goes back to the message the thread belongs to, or the channel list.
    if (this.zone === "thread" || this.zone === "threadComposer") this.zone = this.selected ? "messages" : "composer";
    this.focusNonce++;
    void api.setOpen(this.current, null);
  }

  private onUpsert(p: UpsertPayload) {
    const loaded = this.messages[p.space];
    const arrived: MessageView[] = [];
    if (loaded) {
      let list = loaded;
      for (const m of p.messages) {
        if (m.isReply) continue;
        const before = list.find((x) => x.name === m.name);
        list = upsertInto(list, m);
        if (!before && !m.isOwn) arrived.push(m);
        if (m.localState === "failed" && before?.localState !== "failed") this.say("Message not sent. Retry is next to it.");
      }
      this.messages[p.space] = list;
    }
    if (this.thread && this.thread.space === p.space) {
      let list = this.thread.messages;
      for (const m of p.messages) {
        if (m.thread !== this.thread.name) continue;
        const isNew = !list.some((x) => x.name === m.name);
        list = upsertInto(list, m);
        if (isNew && !m.isOwn && m.isReply) arrived.push(m);
      }
      this.thread.messages = list;
    }
    if (arrived.length && p.space === this.current) {
      const who = arrived[arrived.length - 1].sender.displayName;
      this.say(arrived.length === 1 ? `New message from ${who}` : `${arrived.length} new messages, latest from ${who}`);
      this.maybeMarkRead();
    }
  }

  private onDelete(p: DeletePayload) {
    const drop = new Set(p.names);
    const list = this.messages[p.space];
    if (list) {
      // Keep the keyboard selection on a neighbour instead of losing focus.
      if (this.selected && drop.has(this.selected)) {
        const i = list.findIndex((m) => m.name === this.selected);
        const rest = list.filter((m) => !drop.has(m.name));
        this.selected = rest[Math.min(i, rest.length - 1)]?.name ?? null;
      }
      this.messages[p.space] = list.filter((m) => !drop.has(m.name));
    }
    if (this.thread) {
      const tl = this.thread.messages;
      if (this.threadSelected && drop.has(this.threadSelected)) {
        const i = tl.findIndex((m) => m.name === this.threadSelected);
        const rest = tl.filter((m) => !drop.has(m.name));
        this.threadSelected = rest[Math.min(i, rest.length - 1)]?.name ?? null;
      }
      this.thread.messages = tl.filter((m) => !drop.has(m.name));
    }
  }

  /** At the bottom of the open space with the window focused: mark it read. */
  atBottom = $state(true);
  maybeMarkRead() {
    const space = this.currentSpace;
    if (!space || !this.windowFocused || !this.atBottom) return;
    const list = this.messages[space.name] ?? [];
    const newest = list[list.length - 1];
    if (!newest || newest.localState !== "synced") return;
    if (space.lastReadUs !== null && space.lastReadUs >= newest.createUs) return;
    void api.markRead(space.name, newest.createUs).catch(() => {});
  }

  // ------------------------------------------------------------- actions

  async send(text: string, thread: string | null, attachments: string[] = []) {
    const space = thread ? this.thread?.space : this.current;
    if (!space) return;
    try {
      const pending = await api.sendMessage(space, thread, text, attachments);
      this.onUpsert({ space, messages: [pending] });
    } catch (e) {
      toast.error(errorText(e));
      throw e;
    }
  }

  async saveEdit(text: string) {
    const ed = this.editing;
    if (!ed) return;
    this.editing = null;
    try {
      await api.editMessage(ed.name, text);
    } catch (e) {
      toast.error(errorText(e), { description: "Your edit was not saved; the original text is back." });
    }
  }

  startEdit(m: MessageView, where: "channel" | "thread") {
    if (!m.isOwn || m.deleted || m.localState !== "synced") return;
    this.editing = { name: m.name, text: plainText(m), where };
    this.zone = where === "thread" ? "threadComposer" : "composer";
  }

  async editLast(where: "channel" | "thread") {
    const space = where === "thread" ? this.thread?.space : this.current;
    if (!space) return;
    const m = await api.lastOwnMessage(space, where === "thread" ? (this.thread?.name ?? null) : null);
    if (m) this.startEdit(m, where);
  }

  async doDelete(m: MessageView) {
    this.confirmDelete = null;
    try {
      await api.deleteMessage(m.name);
      toast.success("Message deleted");
    } catch (e) {
      toast.error(errorText(e));
    }
  }

  async react(m: MessageView, emoji: string) {
    try {
      await api.toggleReaction(m.name, emoji);
    } catch (e) {
      toast.error(errorText(e));
    }
  }

  async copyLink(m: MessageView) {
    if (!m.webUrl) return;
    try {
      await navigator.clipboard.writeText(m.webUrl);
      toast.success("Link copied");
    } catch {
      toast.error("Could not copy the link.");
    }
  }

  async toggleStar(space: SpaceSummary) {
    await api.setSpacePrefs(space.name, { starred: !space.starred });
  }

  async toggleMute(space: SpaceSummary) {
    await api.setSpacePrefs(space.name, { muted: !space.muted });
  }

  /** ⌘J: joins the meeting that is running or starts within ten minutes. */
  joinMeeting() {
    const now = Date.now() * 1000;
    const m = this.calendar.meetings.find((x) => x.joinUrl && x.startUs - 10 * 60e6 <= now && now < x.endUs);
    if (m?.joinUrl) {
      void api.openUrl(m.joinUrl);
      this.say(`Joining ${m.title}.`);
    } else {
      toast("No meeting with a video link in the next ten minutes.");
    }
  }

  async toggleHidden(space: SpaceSummary) {
    try {
      await api.setSpacePrefs(space.name, { hidden: !space.hidden });
      this.say(space.hidden ? `${space.displayName} is back in the sidebar.` : `${space.displayName} hidden. It's under Hidden at the bottom of the sidebar.`);
    } catch (e) {
      toast.error(errorText(e));
    }
  }

  setSpaceSort(sort: SpaceSort, announce = false) {
    this.spaceSort = sort;
    if (announce) this.say(`Spaces sorted ${sort === "recent" ? "by recent activity" : sort === "az" ? "A to Z" : "in your own order"}.`);
    try {
      localStorage.setItem("kwek.spaceSort", sort);
    } catch {
      // Private storage unavailable: the choice lasts until restart.
    }
  }

  /** Moves a space within the Spaces section to `to` (an index in its current order) and
   * switches that section to the custom order. Returns where it ended up, or null when it
   * didn't move (already there, or at the edge). */
  async moveSpace(name: string, to: number): Promise<number | null> {
    const rooms = this.sections.find((s) => s.id === "spaces")?.items ?? [];
    const from = rooms.findIndex((s) => s.name === name);
    if (from < 0) return null;
    const target = Math.max(0, Math.min(rooms.length - 1, to));
    if (target === from) return null;
    const order = rooms.map((s) => s.name);
    order.splice(target, 0, ...order.splice(from, 1));
    // Show it at once; Rust confirms with the next spaces update. Everything outside this
    // list loses its place, as Rust does.
    const pos = new Map(order.map((n, i) => [n, i]));
    this.spaces = this.spaces.map((s) => ({ ...s, position: pos.get(s.name) ?? null }));
    this.setSpaceSort("custom");
    try {
      await api.setSpaceOrder(order);
    } catch (e) {
      toast.error(errorText(e));
      // Back to what is saved.
      this.spaces = await api.listSpaces().catch(() => this.spaces);
    }
    return target;
  }

  nextUnread(dir: 1 | -1) {
    const order = this.sidebarOrder;
    if (order.length === 0) return;
    const start = Math.max(0, order.findIndex((s) => s.name === this.current));
    for (let i = 1; i <= order.length; i++) {
      const s = order[(start + dir * i + order.length * 2) % order.length];
      if (s.unread > 0 || s.mentions > 0) {
        void this.open(s.name);
        return;
      }
    }
  }
}

/** The text of a message as typed, for editing. Rebuilt from the rendered blocks. */
export function plainText(m: MessageView): string {
  const out: string[] = [];
  const inl = (xs: Inline[]) =>
    xs
      .map((i) => {
        switch (i.kind) {
          case "text":
            return i.bold ? `*${i.text}*` : i.italic ? `_${i.text}_` : i.strike ? `~${i.text}~` : i.text;
          case "code":
            return "`" + i.text + "`";
          case "mention":
            return `<${i.user}>`;
          case "link":
            return i.label === i.url ? i.url : `<${i.url}|${i.label}>`;
          case "break":
            return "\n";
        }
      })
      .join("");
  const walk = (blocks: Block[], prefix = "") => {
    for (const b of blocks) {
      switch (b.kind) {
        case "paragraph":
        case "heading":
          out.push(prefix + inl(b.inlines));
          break;
        case "code":
          out.push("```\n" + b.text + "\n```");
          break;
        case "quote":
          walk(b.blocks, "> ");
          break;
        case "list":
          b.items.forEach((item, i) => {
            const marker = b.ordered ? `${b.start + i}. ` : "- ";
            const before = out.length;
            walk(item);
            if (out.length > before) out[before] = marker + out[before];
          });
          break;
        case "rule":
          break;
      }
    }
  };
  walk(m.blocks);
  return out.join("\n");
}

export const app = new AppState();
