<script lang="ts">
  import type { MessageView } from "$lib/api";
  import { dayKey, dayLabel } from "$lib/format";
  import { app } from "$lib/state.svelte";
  import { tick } from "svelte";
  import Message from "./Message.svelte";

  let {
    messages,
    key,
    where = "channel",
    newMarker = null,
    label,
  }: {
    messages: MessageView[];
    key: string;
    where?: "channel" | "thread";
    newMarker?: number | null;
    label: string;
  } = $props();

  let scroller: HTMLDivElement | undefined = $state();
  let content: HTMLDivElement | undefined = $state();
  let lastKey = "";
  let lastCount = 0;
  /** Following the newest message; content that grows (images loading) keeps us there. */
  let stickToBottom = true;

  function toBottom() {
    if (scroller) scroller.scrollTop = scroller.scrollHeight;
  }

  $effect(() => {
    if (!content) return;
    const ro = new ResizeObserver(() => {
      if (stickToBottom) toBottom();
    });
    ro.observe(content);
    return () => ro.disconnect();
  });

  const selected = $derived(where === "thread" ? app.threadSelected : app.selected);
  const zoneActive = $derived(where === "thread" ? app.zone === "thread" : app.zone === "messages");

  const rows = $derived.by(() => {
    const out: Array<
      | { type: "day"; id: string; label: string }
      | { type: "new"; id: string }
      | { type: "msg"; id: string; m: MessageView; grouped: boolean; pos: number }
    > = [];
    let prev: MessageView | null = null;
    let newShown = false;
    let pos = 0;
    for (const m of messages) {
      const day = dayKey(m.createUs);
      const dayChanged = !prev || dayKey(prev.createUs) !== day;
      if (dayChanged) out.push({ type: "day", id: `day-${day}`, label: dayLabel(m.createUs) });
      let crossedNew = false;
      if (!newShown && newMarker !== null && m.createUs > newMarker && !m.isOwn && where === "channel") {
        out.push({ type: "new", id: "new" });
        newShown = true;
        crossedNew = true;
      }
      const grouped =
        !!prev &&
        !dayChanged &&
        !crossedNew &&
        prev.sender.name === m.sender.name &&
        m.createUs - prev.createUs < 5 * 60 * 1_000_000 &&
        !prev.threadSummary;
      out.push({ type: "msg", id: m.name, m, grouped, pos: ++pos });
      prev = m;
    }
    return out;
  });

  /** In a thread, the root is the first message row; a "N replies" line follows it. */
  const firstMessageRow = $derived(rows.findIndex((r) => r.type === "msg"));

  function isAtBottom() {
    if (!scroller) return true;
    return scroller.scrollHeight - scroller.scrollTop - scroller.clientHeight < 48;
  }

  function onScroll() {
    if (!scroller) return;
    stickToBottom = isAtBottom();
    if (where === "channel") {
      const was = app.atBottom;
      app.atBottom = stickToBottom;
      if (stickToBottom && !was) app.maybeMarkRead();
      if (scroller.scrollTop < 200 && !app.loadingOlder) void older();
    }
  }

  async function older() {
    if (!scroller || where !== "channel") return;
    const before = scroller.scrollHeight;
    const top = scroller.scrollTop;
    await app.loadOlder();
    await tick();
    if (scroller) scroller.scrollTop = top + (scroller.scrollHeight - before);
  }

  // Entering a space, once its messages are there: the New line if there is unread, else
  // the bottom. New messages: follow them if we were at the bottom, and always after
  // sending one ourselves.
  $effect(() => {
    const count = messages.length;
    const k = key;
    const newest = messages[count - 1];
    void tick().then(() => {
      if (!scroller) return;
      if (k !== lastKey && count > 0) {
        lastKey = k;
        const newLine = scroller.querySelector<HTMLElement>("[data-new-line]");
        if (newLine && newLine.offsetTop > scroller.clientHeight / 2) {
          // Unread on entry: start at the New line so nothing is marked read unseen.
          scroller.scrollTop = Math.max(0, newLine.offsetTop - 64);
          stickToBottom = isAtBottom();
        } else {
          stickToBottom = true;
          toBottom();
        }
        if (where === "channel") {
          app.atBottom = stickToBottom;
          app.maybeMarkRead();
        }
      } else if (k === lastKey && count > lastCount && (stickToBottom || newest?.isOwn)) {
        stickToBottom = true;
        toBottom();
      }
      if (k === lastKey) lastCount = count;
    });
  });

  // Keyboard selection moves real focus, so screen readers follow it.
  $effect(() => {
    const name = selected;
    if (!name || !zoneActive || !scroller) return;
    const el = scroller.querySelector<HTMLElement>(`[data-message="${CSS.escape(name)}"]`);
    if (el && document.activeElement !== el) {
      el.focus({ preventScroll: true });
      el.scrollIntoView({ block: "nearest" });
    }
  });

  // Jump target from search or a notification.
  $effect(() => {
    const target = app.jumpTo;
    if (!target || where !== "channel" || !scroller) return;
    void tick().then(() => {
      const el = scroller?.querySelector<HTMLElement>(`[data-message="${CSS.escape(target)}"]`);
      if (el) {
        el.scrollIntoView({ block: "center" });
        el.focus({ preventScroll: true });
        app.jumpTo = null;
      }
    });
  });

  // Zone focus with nothing selected: focus the list itself so j/k start from here.
  $effect(() => {
    void app.focusNonce;
    if (zoneActive && !selected && scroller && !scroller.contains(document.activeElement)) scroller.focus({ preventScroll: true });
    if (zoneActive && selected && scroller) {
      const el = scroller.querySelector<HTMLElement>(`[data-message="${CSS.escape(selected)}"]`);
      if (el && document.activeElement !== el) el.focus({ preventScroll: true });
    }
  });

  /** Roving tab stop: the selected message, else the newest. */
  const tabStop = $derived(selected ?? messages[messages.length - 1]?.name ?? null);

  function select(name: string) {
    if (where === "thread") {
      app.threadSelected = name;
      app.zone = "thread";
    } else {
      app.selected = name;
      app.zone = "messages";
    }
  }
</script>

<div
  bind:this={scroller}
  onscroll={onScroll}
  class="min-h-0 flex-1 overflow-y-auto outline-none focus:ring-2 focus:ring-ring focus:ring-inset"
  tabindex="-1"
>
  {#if where === "channel"}
    <div class="px-6 pt-8 pb-2 text-center text-[13px] text-muted-foreground">
      {#if app.loadingOlder}
        Loading older messages
      {:else if app.currentSpace?.historyComplete}
        This is the start of the conversation.
      {:else}
        <button class="inline-flex h-6 items-center underline underline-offset-2 hover:text-foreground" onclick={older}>Load older messages</button>
      {/if}
    </div>
  {/if}
  <div bind:this={content} role="feed" aria-label={label} aria-busy={app.loadingOlder}>
    {#each rows as r, i (r.id)}
      {#if r.type === "day"}
        <div class="flex items-center gap-3 px-6 pt-6 pb-1" role="separator" aria-label={r.label}>
          <div class="h-px flex-1 bg-border"></div>
          <span class="text-[12px] font-medium text-muted-foreground" aria-hidden="true">{r.label}</span>
          <div class="h-px flex-1 bg-border"></div>
        </div>
      {:else if r.type === "new"}
        <div data-new-line class="flex items-center gap-2 px-6 pt-3" role="separator" aria-label="New messages">
          <div class="h-0.5 flex-1 rounded-full bg-highlight-strong"></div>
          <span class="rounded-sm bg-highlight-strong px-1.5 text-[11px] font-semibold text-foreground" aria-hidden="true">New</span>
        </div>
      {:else}
        <Message
          m={r.m}
          grouped={r.grouped}
          selected={selected === r.m.name}
          tabbable={tabStop === r.m.name}
          position={r.pos}
          {where}
          onselect={() => select(r.m.name)}
        />
        {#if where === "thread" && i === firstMessageRow && messages.length > 1}
          <div class="flex items-center gap-3 px-6 pt-3 pb-1" role="separator" aria-label="Replies">
            <span class="text-[12px] font-medium text-muted-foreground" aria-hidden="true"
              >{messages.length - 1} {messages.length === 2 ? "reply" : "replies"}</span
            >
            <div class="h-px flex-1 bg-border"></div>
          </div>
        {/if}
      {/if}
    {/each}
  </div>
  <div class="h-5"></div>
</div>
