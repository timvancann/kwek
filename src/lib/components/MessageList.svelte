<script lang="ts" module>
  // Scroll position per space survives switching away and back.
  const scrollMemory = new Map<string, number | "bottom">();
</script>

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
  let lastKey = "";
  let lastCount = 0;
  let stickToBottom = true;

  const selected = $derived(where === "thread" ? app.threadSelected : app.selected);
  const zoneActive = $derived(where === "thread" ? app.zone === "thread" : app.zone === "messages");

  const rows = $derived.by(() => {
    const out: Array<
      { type: "day"; id: string; label: string } | { type: "new"; id: string } | { type: "msg"; id: string; m: MessageView; grouped: boolean }
    > = [];
    let prev: MessageView | null = null;
    let newShown = false;
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
      out.push({ type: "msg", id: m.name, m, grouped });
      prev = m;
    }
    return out;
  });

  function isAtBottom() {
    if (!scroller) return true;
    return scroller.scrollHeight - scroller.scrollTop - scroller.clientHeight < 48;
  }

  function onScroll() {
    if (!scroller) return;
    stickToBottom = isAtBottom();
    scrollMemory.set(key, stickToBottom ? "bottom" : scroller.scrollTop);
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

  // New space: restore the remembered position, or start at the bottom.
  // Same space, new messages: stay at the bottom if we were there.
  $effect(() => {
    const count = messages.length;
    const k = key;
    void tick().then(() => {
      if (!scroller) return;
      if (k !== lastKey) {
        lastKey = k;
        const mem = scrollMemory.get(k);
        if (typeof mem === "number") scroller.scrollTop = mem;
        else scroller.scrollTop = scroller.scrollHeight;
        stickToBottom = isAtBottom();
        if (where === "channel") app.atBottom = stickToBottom;
      } else if (count > lastCount && stickToBottom) {
        scroller.scrollTop = scroller.scrollHeight;
      }
      lastCount = count;
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
    if (zoneActive && !selected && scroller && !scroller.contains(document.activeElement)) scroller.focus({ preventScroll: true });
  });

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
  class="min-h-0 flex-1 overflow-y-auto outline-none"
  role="feed"
  aria-label={label}
  aria-busy={app.loadingOlder}
  tabindex="-1"
>
  {#if where === "channel"}
    <div class="px-4 pt-6 pb-2 text-center text-[12px] text-muted-foreground">
      {#if app.loadingOlder}
        Loading older messages
      {:else if app.currentSpace?.historyComplete}
        This is the start of the conversation.
      {:else}
        <button class="underline underline-offset-2 hover:text-foreground" onclick={older}>Load older messages</button>
      {/if}
    </div>
  {/if}
  {#each rows as r (r.id)}
    {#if r.type === "day"}
      <div class="flex items-center gap-3 px-4 pt-4 pb-1" role="separator" aria-label={r.label}>
        <div class="h-px flex-1 bg-border"></div>
        <span class="text-[12px] font-medium text-muted-foreground">{r.label}</span>
        <div class="h-px flex-1 bg-border"></div>
      </div>
    {:else if r.type === "new"}
      <div class="flex items-center gap-2 px-4 pt-2" role="separator" aria-label="New messages">
        <div class="h-0.5 flex-1 rounded-full bg-highlight-strong"></div>
        <span class="rounded-sm bg-highlight-strong px-1.5 text-[11px] font-semibold text-foreground">New</span>
      </div>
    {:else}
      <Message m={r.m} grouped={r.grouped} selected={selected === r.m.name} {where} onselect={() => select(r.m.name)} />
    {/if}
  {/each}
  <div class="h-3"></div>
</div>
