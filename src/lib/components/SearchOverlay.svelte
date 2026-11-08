<script lang="ts">
  import { tick, untrack } from "svelte";
  import * as Dialog from "$lib/components/ui/dialog/index.js";
  import { api, errorText, type SearchResult, type UserRef } from "$lib/api";
  import { ago, dayLabel } from "$lib/format";
  import { app } from "$lib/state.svelte";
  import { cn } from "$lib/utils";
  import Avatar from "./Avatar.svelte";
  import SearchIcon from "@lucide/svelte/icons/search";
  import XIcon from "@lucide/svelte/icons/x";

  const LIMIT = 40;
  const SUGGESTIONS = 6;

  /** A from: or in: filter picked from the suggestions; `value` is a resource name. */
  type Chip = { kind: "from" | "in"; value: string; label: string };
  type Suggestion = { value: string; label: string; detail: string | null; avatar: UserRef };

  let open = $state(false);
  /** What is typed in the box; filters picked as chips live in `chips`. */
  let query = $state("");
  let chips = $state<Chip[]>([]);
  let people = $state<UserRef[]>([]);
  let caret = $state(0);
  let suggestActive = $state(0);
  /** The from:/in: word the user dismissed with Escape; no suggestions for it again. */
  let dismissed = $state("");
  /** The latest answer and the query it answers. */
  let result = $state<SearchResult | null>(null);
  let resultFor = $state("");
  let error = $state("");
  let active = $state(0);
  let timer: ReturnType<typeof setTimeout> | undefined;
  let sayTimer: ReturnType<typeof setTimeout> | undefined;
  let refreshTimer: ReturnType<typeof setInterval> | undefined;
  let seq = 0;
  let lastSaid = "";
  let picked = false;
  let listEl: HTMLDivElement | undefined = $state();
  let inputEl: HTMLInputElement | undefined = $state();

  $effect(() => {
    open = app.overlay === "search";
  });
  $effect(() => {
    if (!open && app.overlay === "search") app.overlay = null;
  });
  // Fresh results and status on every open; while indexing or downloading, refresh the
  // status every few seconds.
  $effect(() => {
    if (!open) return;
    // Untracked: search() reads `query`, and typing must go through the debounce.
    untrack(() => {
      caret = query.length;
      dismissed = "";
      void search();
      api
        .listSenders()
        .then((p) => (people = p))
        .catch(() => {});
    });
    refreshTimer = setInterval(() => {
      if (incomplete) void refreshStatus();
    }, 3000);
    return () => clearInterval(refreshTimer);
  });

  /** Updates the status line only; results stay put under the user's cursor. */
  async function refreshStatus() {
    try {
      const s = await api.search("", 1);
      if (result) result = { ...result, historyLeft: s.historyLeft, meaning: s.meaning, coveredFromUs: s.coveredFromUs };
    } catch {
      // The next search shows any error.
    }
  }

  /** Runs the search now. Answers to older requests are dropped, so a slow first query
   * (the model warming up) never overwrites a newer one. */
  async function search() {
    clearTimeout(timer);
    const q = full;
    const mine = ++seq;
    try {
      const r = await api.search(q, LIMIT);
      if (mine !== seq) return;
      const changed = q !== resultFor;
      result = r;
      resultFor = q;
      error = "";
      if (changed) {
        active = 0;
        if (listEl) listEl.scrollTop = 0;
        announce();
      }
    } catch (e) {
      if (mine === seq) error = errorText(e);
    }
  }

  function onInput() {
    syncCaret();
    suggestActive = 0;
  }

  // Whatever changes what is asked (typing, moving the caret out of a from: word, dismissing
  // suggestions) searches once it settles.
  $effect(() => {
    const f = full;
    if (!open || f === resultFor) return;
    untrack(() => {
      clearTimeout(timer);
      timer = setTimeout(search, 120);
    });
  });

  function syncCaret() {
    caret = inputEl?.selectionStart ?? query.length;
  }

  /** The from: or in: word being typed at the caret. */
  const token = $derived.by(() => {
    const before = query.slice(0, caret);
    const m = /(?:^|\s)(from|in):(\S*)$/.exec(before);
    if (!m) return null;
    const kind = m[1] as Chip["kind"];
    return { kind, text: m[2], start: before.length - kind.length - 1 - m[2].length };
  });

  const suggestions = $derived.by((): Suggestion[] => {
    if (!token || `${token.kind}:${token.text}` === dismissed) return [];
    const t = token.text.toLowerCase();
    if (token.kind === "from") {
      return people
        .filter((p) => !t || p.displayName.toLowerCase().includes(t) || !!p.email?.toLowerCase().startsWith(t))
        .slice(0, SUGGESTIONS)
        .map((p) => ({ value: p.name, label: p.displayName, detail: p.email, avatar: p }));
    }
    return app.spaces
      .filter((s) => !t || s.displayName.toLowerCase().includes(t))
      .slice(0, SUGGESTIONS)
      .map((s) => ({
        value: s.name,
        label: s.displayName,
        detail: s.kind === "space" ? "Space" : "Direct message",
        avatar: { name: s.name, displayName: s.displayName, email: null, avatarUrl: s.avatarUrl },
      }));
  });
  const suggesting = $derived(suggestions.length > 0);
  /** The highlighted suggestion, kept inside the list when it shrinks under the cursor
   * (people still loading, a space renamed). */
  const sug = $derived(Math.min(suggestActive, Math.max(suggestions.length - 1, 0)));

  /** What is sent: the typed text without the word that is still being completed, then the
   * chips as exact filters. The parser keeps the last filter of a kind, so a visible chip
   * wins over a from: or in: left typed out. */
  const full = $derived.by(() => {
    const typed = token && suggesting ? query.slice(0, token.start) + query.slice(caret) : query;
    return [typed.trim(), ...chips.map((c) => `${c.kind}:${c.value}`)].filter(Boolean).join(" ");
  });

  /** Turns the word being typed into a chip. The search takes one filter of each kind, so a
   * new one replaces the old. */
  async function choose(i: number) {
    const s = suggestions[i];
    const t = token;
    if (!s || !t) return;
    chips = [...chips.filter((c) => c.kind !== t.kind), { kind: t.kind, value: s.value, label: s.label }];
    query = query.slice(0, t.start) + query.slice(caret).replace(/^\s+/, "");
    caret = t.start;
    suggestActive = 0;
    app.say(`Filter added: ${t.kind} ${s.label}.`);
    await tick();
    inputEl?.focus();
    inputEl?.setSelectionRange(t.start, t.start);
    void search();
  }

  function removeChip(i: number) {
    const c = chips[i];
    chips = chips.filter((_, j) => j !== i);
    app.say(`Filter removed: ${c.kind} ${c.label}.`);
    inputEl?.focus();
    void search();
  }

  /** Escape closes the suggestions first, the dialog only after. */
  function onEscapeKeydown(e: KeyboardEvent) {
    if (suggesting && token) {
      e.preventDefault();
      dismissed = `${token.kind}:${token.text}`;
      void search();
    }
  }

  /** Spoken once typing settles, and only when the outcome changed. */
  function announce() {
    clearTimeout(sayTimer);
    sayTimer = setTimeout(() => {
      if (!resultFor.trim() || !result) return;
      const n = result.hits.length;
      let text = n === 0 ? "No conversations match." : `${n}${n === LIMIT ? " or more" : ""} ${n === 1 ? "conversation" : "conversations"}.`;
      if (n === 0 && incomplete) text += " Some older messages are not searchable yet.";
      if (text !== lastSaid) app.say(text);
      lastSaid = text;
    }, 700);
  }

  const pending = $derived(full !== resultFor);
  const hits = $derived(resultFor.trim() ? (result?.hits ?? []) : []);
  const hasFilter = $derived(chips.length > 0 || /\b(from|in|before|after):/.test(query));
  const incomplete = $derived(
    !!result && (result.historyLeft > 0 || (result.meaning.available && result.meaning.indexed < result.meaning.total)),
  );

  /** What search does not cover yet. */
  const status = $derived.by(() => {
    if (!result) return "";
    const parts: string[] = [];
    if (result.historyLeft > 0) {
      parts.push(`Older messages are still downloading, ${result.historyLeft} ${result.historyLeft === 1 ? "conversation" : "conversations"} to go.`);
    } else if (result.coveredFromUs) {
      parts.push(`Searching messages since ${dayLabel(result.coveredFromUs)}.`);
    }
    const m = result.meaning;
    if (m.available && m.total > 0 && m.indexed < m.total) {
      parts.push(`Related results cover ${Math.floor((m.indexed / m.total) * 100)}% of your messages so far.`);
    }
    return parts.join(" ");
  });

  async function pick(i: number) {
    // Enter while the list still shows an older query's answer: wait for the current one.
    if (pending) await search();
    // That search failed: the list still answers an older query, so open nothing.
    if (pending) return;
    const h = hits[i];
    if (!h) return;
    picked = true;
    app.overlay = null;
    if (h.thread) void app.openInThread(h.space, h.thread, h.anchor);
    else void app.open(h.space, h.anchor);
  }

  function onKeydown(e: KeyboardEvent) {
    if (suggesting) {
      if (e.key === "ArrowDown" || e.key === "ArrowUp") {
        e.preventDefault();
        const n = suggestions.length;
        suggestActive = (sug + (e.key === "ArrowDown" ? 1 : -1) + n) % n;
        return;
      }
      // Enter, Tab, or a space after a few letters: take the highlighted suggestion, so
      // names with spaces ("in:01. Algemeen") never need typing out.
      if (e.key === "Enter" || e.key === "Tab" || (e.key === " " && token?.text)) {
        e.preventDefault();
        void choose(sug);
        return;
      }
    }
    const el = e.currentTarget as HTMLInputElement;
    if (e.key === "Backspace" && chips.length && el.selectionStart === 0 && el.selectionEnd === 0) {
      e.preventDefault();
      removeChip(chips.length - 1);
      return;
    }
    if (e.key === "ArrowDown" || e.key === "ArrowUp") {
      e.preventDefault();
      if (!hits.length) return;
      active = (active + (e.key === "ArrowDown" ? 1 : -1) + hits.length) % hits.length;
      listEl?.querySelector(`#hit-${active}`)?.scrollIntoView({ block: "nearest" });
    } else if (e.key === "Enter") {
      e.preventDefault();
      void pick(active);
    }
  }

  // Reopening keeps the last query, selected, so typing replaces it.
  function onOpenAutoFocus(e: Event) {
    e.preventDefault();
    inputEl?.focus();
    inputEl?.select();
  }

  // After a pick the app focuses the message; don't let the dialog put focus back.
  function onCloseAutoFocus(e: Event) {
    if (picked) e.preventDefault();
    picked = false;
  }

  const firstName = (n: string) => n.split(/\s+/)[0] ?? n;
</script>

<Dialog.Root bind:open>
  <Dialog.Content
    class="top-[10%] max-w-2xl translate-y-0 gap-0 p-0 sm:max-w-2xl"
    showCloseButton={false}
    {onOpenAutoFocus}
    {onCloseAutoFocus}
    {onEscapeKeydown}
  >
    <Dialog.Title class="sr-only">Search messages</Dialog.Title>
    <Dialog.Description class="sr-only">
      Finds conversations that contain your words, even across a few messages, plus related ones. Narrow down with from, in, before and after.
    </Dialog.Description>
    <div class="flex min-h-12 flex-wrap items-center gap-x-2 gap-y-1.5 border-b border-border px-4 py-2">
      <SearchIcon class="size-4 text-muted-foreground" aria-hidden="true" />
      {#each chips as c, i (c.kind)}
        <span class="inline-flex h-7 items-center gap-1 rounded-md border border-border bg-muted pr-0.5 pl-2 text-[13px]">
          <span class="text-muted-foreground">{c.kind === "from" ? "From" : "In"}</span>
          <span class="max-w-48 truncate font-medium">{c.label}</span>
          <button
            type="button"
            class="grid size-6 place-items-center rounded-sm text-muted-foreground hover:bg-background hover:text-foreground"
            aria-label="Remove filter: {c.kind} {c.label}"
            onclick={() => removeChip(i)}
          >
            <XIcon class="size-3.5" aria-hidden="true" />
          </button>
        </span>
      {/each}
      <input
        bind:this={inputEl}
        bind:value={query}
        oninput={onInput}
        onkeydown={onKeydown}
        onkeyup={syncCaret}
        onclick={syncCaret}
        class="selectable h-8 min-w-32 flex-1 bg-transparent text-[15px] outline-none placeholder:text-muted-foreground"
        placeholder={chips.length ? "Add words" : "Search, e.g. demo dataset from:name"}
        aria-label={chips.length ? `Search messages, filtered ${chips.map((c) => `${c.kind} ${c.label}`).join(", ")}` : "Search messages"}
        aria-describedby={status ? "search-status" : undefined}
        role="combobox"
        aria-expanded={suggesting || hits.length > 0}
        aria-controls={suggesting ? "search-suggestions" : "search-results"}
        aria-activedescendant={suggesting ? `sug-${sug}` : hits.length ? `hit-${active}` : undefined}
        aria-autocomplete="list"
        autocomplete="off"
        spellcheck="false"
      />
    </div>
    {#if suggesting && token}
      <div id="search-suggestions" role="listbox" aria-label={token.kind === "from" ? "People" : "Conversations"} class="p-1.5">
        <p class="m-0 flex justify-between px-3 pt-1 pb-1.5 text-[12px] text-muted-foreground" aria-hidden="true">
          <span>{token.kind === "from" ? "Messages from" : "Messages in"}</span><span>Enter to choose</span>
        </p>
        {#each suggestions as s, i (s.value)}
          <!-- svelte-ignore a11y_click_events_have_key_events -->
          <div
            id="sug-{i}"
            role="option"
            tabindex="-1"
            aria-selected={i === sug}
            class={cn("flex cursor-default items-center gap-2.5 rounded-lg px-3 py-1.5", i === sug && "bg-accent-soft")}
            onmousemove={() => (suggestActive = i)}
            onclick={() => void choose(i)}
          >
            <Avatar user={s.avatar} size={22} />
            <span class="truncate text-[14px] font-medium">{s.label}</span>
            {#if s.detail}<span class="truncate text-[12px] text-muted-foreground">{s.detail}</span>{/if}
          </div>
        {/each}
      </div>
    {:else if error}
      <p class="m-0 px-4 py-6 text-center text-destructive" role="alert">{error}</p>
    {:else if resultFor.trim() && result && hits.length === 0}
      <div class="space-y-1 px-4 py-6 text-center text-muted-foreground">
        <p class="m-0">No conversations match.</p>
        {#if incomplete}<p class="m-0 text-[13px]">Some older messages are not searchable yet.</p>{/if}
        {#if hasFilter}<p class="m-0 text-[13px]">Try removing a filter.</p>{/if}
      </div>
    {:else if !full.trim()}
      <div class="space-y-1.5 px-4 py-4 text-[13px] text-muted-foreground">
        <p class="m-0">Type a few words you remember. They don't have to be in the same message.</p>
        <p class="m-0">
          Narrow it down: type <code class="font-mono text-[12px]">from:</code> or <code class="font-mono text-[12px]">in:</code> and pick a
          name, or add <code class="font-mono text-[12px]">before:2026-11-01</code> or <code class="font-mono text-[12px]">after:2026-10-01</code>.
        </p>
      </div>
    {/if}
    <div
      id="search-results"
      bind:this={listEl}
      role="listbox"
      aria-label="Conversations"
      aria-busy={pending}
      class={cn("max-h-[62vh] overflow-y-auto p-1.5 transition-opacity", pending && "opacity-60")}
      class:hidden={!hits.length || suggesting}
    >
      {#each hits as h, i (h.anchor)}
        {@const time = h.lines[0]?.createUs ?? h.firstUs}
        <!-- Keyboard selection runs through the combobox input (arrows + Enter). -->
        <!-- svelte-ignore a11y_click_events_have_key_events -->
        <div
          id="hit-{i}"
          role="option"
          tabindex="-1"
          aria-selected={i === active}
          class={cn("cursor-default rounded-lg px-3 py-2", i === active && "bg-accent-soft ring-1 ring-accent ring-inset")}
          onmousemove={() => (active = i)}
          onclick={() => void pick(i)}
        >
          <div class="flex items-baseline gap-2 text-[12px] text-muted-foreground">
            <span class="truncate text-[13px] font-semibold text-foreground">{h.spaceName}</span>
            {#if h.thread}<span class="shrink-0">in a thread</span>{/if}
            <span class="shrink-0">{ago(time)}</span>
            {#if h.meaning && !h.words}
              <span
                class="ml-auto shrink-0 rounded-sm border border-border bg-background px-1.5 text-[11px] font-medium"
                title="Close in meaning; doesn't contain your words"
                >Related<span class="sr-only">, doesn't contain your words</span></span
              >
            {/if}
            <span class="sr-only">.</span>
          </div>
          <div class="mt-1 space-y-0.5">
            {#each h.lines as l, li (l.message)}
              <div class="flex min-w-0 items-start gap-2">
                <Avatar user={l.sender} size={20} class="mt-px" />
                <p class={cn("m-0 min-w-0 flex-1 text-[14px] leading-snug", li === 0 ? "line-clamp-2" : "line-clamp-1")}>
                  <span class="font-semibold" title={l.sender.displayName}>{firstName(l.sender.displayName)}</span><span class="sr-only">:</span>
                  {#each l.parts as p, j (j)}{#if p.hit}<mark class="rounded-sm bg-highlight-strong px-px text-foreground">{p.text}</mark>{:else}{p.text}{/if}{/each}<span
                    class="sr-only">.</span
                  >
                </p>
              </div>
            {/each}
          </div>
          {#if h.more > 0}
            <p class="m-0 mt-0.5 pl-7 text-[12px] text-muted-foreground">and {h.more} more {h.more === 1 ? "message" : "messages"}</p>
          {/if}
        </div>
      {/each}
    </div>
    {#if status}
      <p id="search-status" class="m-0 border-t border-border px-4 py-2 text-[12px] text-muted-foreground">{status}</p>
    {/if}
  </Dialog.Content>
</Dialog.Root>
