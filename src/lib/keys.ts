// The keymap: one registry for dispatch and for the "?" overlay, so it can become
// configurable later. Vim keys apply when no text field has focus.
import { app } from "./state.svelte";
import type { MessageView } from "./api";

export interface Binding {
  keys: string;
  label: string;
  group: "Navigate" | "Messages" | "General";
  run: () => void;
}

function list(): MessageView[] {
  return app.zone === "thread" ? (app.thread?.messages ?? []) : app.currentMessages;
}

function selectedMessage(): MessageView | null {
  const name = app.zone === "thread" ? app.threadSelected : app.selected;
  return list().find((m) => m.name === name) ?? null;
}

function setSelected(name: string | null) {
  if (app.zone === "thread") app.threadSelected = name;
  else app.selected = name;
}

/** In the sidebar, j/k only move the highlight: opening (and marking read) needs Enter or o. */
function move(dir: 1 | -1) {
  if (app.zone === "sidebar") {
    const order = app.sidebarOrder;
    if (order.length === 0) return;
    // The highlight may sit in a collapsed section (Hidden): then step from the open one.
    let at = order.findIndex((s) => s.name === app.sidebarFocus);
    if (at < 0) at = order.findIndex((s) => s.name === app.current);
    const next = at < 0 ? 0 : Math.min(order.length - 1, Math.max(0, at + dir));
    app.sidebarFocus = order[next].name;
    return;
  }
  if (app.zone !== "thread") app.zone = "messages";
  const items = list().filter((m) => m.localState !== "ghost");
  if (items.length === 0) return;
  const cur = items.findIndex((m) => m.name === (app.zone === "thread" ? app.threadSelected : app.selected));
  const next = cur < 0 ? (dir === 1 ? 0 : items.length - 1) : Math.min(items.length - 1, Math.max(0, cur + dir));
  setSelected(items[next].name);
}

function edge(last: boolean) {
  if (app.zone === "sidebar") {
    const order = app.sidebarOrder;
    if (order.length === 0) return;
    app.sidebarFocus = order[last ? order.length - 1 : 0].name;
    return;
  }
  if (app.zone !== "thread") app.zone = "messages";
  const items = list();
  if (items.length) setSelected(items[last ? items.length - 1 : 0].name);
}

function focusComposer() {
  app.zone = app.zone === "thread" || (app.thread && app.zone === "threadComposer") ? "threadComposer" : "composer";
}

function zoneLeft() {
  if (app.zone === "thread" || app.zone === "threadComposer") {
    // In a narrow window the channel sits behind the overlay: leaving means closing.
    if (app.narrow) app.closeThread();
    else app.zone = "messages";
  } else {
    // Entering the sidebar starts at the open conversation.
    app.sidebarFocus = app.current;
    app.zone = "sidebar";
  }
}

function zoneRight() {
  if (app.zone === "sidebar") app.zone = "messages";
  else if (app.thread) app.zone = "thread";
}

function withSelected(f: (m: MessageView) => void) {
  const m = selectedMessage();
  if (m) f(m);
}

export const bindings: Binding[] = [
  { keys: "⌘K", label: "Jump to a conversation", group: "General", run: () => (app.overlay = "switcher") },
  { keys: "⌘F", label: "Search messages", group: "General", run: () => (app.overlay = "search") },
  { keys: "/", label: "Search messages", group: "General", run: () => (app.overlay = "search") },
  { keys: "?", label: "Show keyboard shortcuts", group: "General", run: () => (app.overlay = "keys") },
  { keys: "Esc", label: "Leave the composer, close the thread or a dialog", group: "General", run: escape },
  { keys: "j", label: "Next message (or conversation in the sidebar)", group: "Navigate", run: () => move(1) },
  { keys: "k", label: "Previous message (or conversation)", group: "Navigate", run: () => move(-1) },
  { keys: "J", label: "Next conversation with unread messages", group: "Navigate", run: () => app.nextUnread(1) },
  { keys: "K", label: "Previous conversation with unread messages", group: "Navigate", run: () => app.nextUnread(-1) },
  { keys: "gg", label: "Oldest loaded message", group: "Navigate", run: () => edge(false) },
  { keys: "G", label: "Newest message", group: "Navigate", run: () => edge(true) },
  { keys: "⌃H", label: "Move to the pane on the left", group: "Navigate", run: zoneLeft },
  { keys: "⌃L", label: "Move to the pane on the right", group: "Navigate", run: zoneRight },
  { keys: "Enter", label: "Write a message (in the sidebar: open the conversation)", group: "Messages", run: focusComposer },
  { keys: "i", label: "Write a message", group: "Messages", run: focusComposer },
  {
    keys: "r",
    label: "Reply in thread",
    group: "Messages",
    run: () =>
      withSelected((m) => {
        if (m.thread) void app.openThread(m.thread);
      }),
  },
  {
    keys: "o",
    label: "Open the thread (or the conversation in the sidebar)",
    group: "Messages",
    run: () => {
      if (app.zone === "sidebar") {
        const s = app.sidebarFocus ?? app.current;
        if (s) void app.open(s).then(() => (app.zone = "composer"));
        return;
      }
      withSelected((m) => {
        if (m.thread)
          void app.openThread(m.thread).then(() => {
            app.zone = "thread";
          });
      });
    },
  },
  {
    keys: "e",
    label: "Edit your message",
    group: "Messages",
    run: () => withSelected((m) => app.startEdit(m, app.zone === "thread" ? "thread" : "channel")),
  },
  {
    keys: "dd",
    label: "Delete your message",
    group: "Messages",
    run: () =>
      withSelected((m) => {
        if (m.isOwn && !m.deleted) app.confirmDelete = m;
      }),
  },
  { keys: "+", label: "React", group: "Messages", run: () => withSelected((m) => (app.reactTo = m)) },
  { keys: "y", label: "Copy link to the message", group: "Messages", run: () => withSelected((m) => void app.copyLink(m)) },
];

function escape() {
  if (app.zone === "thread" || app.zone === "threadComposer") {
    app.closeThread();
    return;
  }
  if (app.thread) {
    app.closeThread();
    return;
  }
  app.selected = null;
}

let pending = "";
let pendingTimer: ReturnType<typeof setTimeout> | undefined;

function isEditable(t: EventTarget | null): boolean {
  if (!(t instanceof HTMLElement)) return false;
  return t.isContentEditable || t.tagName === "TEXTAREA" || (t.tagName === "INPUT" && (t as HTMLInputElement).type !== "checkbox");
}

/** Global keydown handler; returns true when it handled the key. */
export function handleKey(e: KeyboardEvent): boolean {
  if (e.key === "k" && e.metaKey && !e.shiftKey && !e.altKey) {
    e.preventDefault();
    app.viewing = null; // one modal at a time
    app.overlay = app.overlay === "switcher" ? null : "switcher";
    return true;
  }
  if (e.key === "f" && e.metaKey && !e.shiftKey && !e.altKey) {
    e.preventDefault();
    app.viewing = null;
    app.overlay = app.overlay === "search" ? null : "search";
    return true;
  }
  // Dialogs and overlays own the keyboard while open.
  if (app.overlay || app.confirmDelete || app.reactTo || app.confirmSignOut || app.viewing) return false;
  // Ctrl+H/L only outside text fields: there Ctrl+H is macOS delete-backward.
  if (e.ctrlKey && !e.metaKey && (e.key === "h" || e.key === "l") && !isEditable(e.target)) {
    e.preventDefault();
    if (e.key === "h") zoneLeft();
    else zoneRight();
    return true;
  }
  if (isEditable(e.target)) {
    if (e.key === "Escape") {
      e.preventDefault();
      if (app.editing) app.editing = null;
      app.zone = app.zone === "threadComposer" ? "thread" : "messages";
      (e.target as HTMLElement).blur();
      return true;
    }
    return false;
  }
  if (e.metaKey || e.ctrlKey || e.altKey) return false;

  let key = e.key === "Escape" ? "Esc" : e.key;
  if (key === "Enter" && (e.target as HTMLElement)?.closest?.("button, a, [role=button]")) return false;
  if (pending) {
    const combined = pending + key;
    pending = "";
    clearTimeout(pendingTimer);
    // An unfinished sequence (g then j) falls through to the second key on its own.
    if (bindings.some((x) => x.keys === combined)) key = combined;
  }
  const b = bindings.find((x) => x.keys === key);
  if (b) {
    e.preventDefault();
    b.run();
    return true;
  }
  // First half of a two-key sequence (gg, dd).
  if (bindings.some((x) => x.keys.length === 2 && x.keys.startsWith(key) && key.length === 1 && /[a-z]/.test(key))) {
    pending = key;
    pendingTimer = setTimeout(() => (pending = ""), 700);
    e.preventDefault();
    return true;
  }
  return false;
}
