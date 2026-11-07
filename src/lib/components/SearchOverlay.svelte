<script lang="ts">
  import * as Dialog from "$lib/components/ui/dialog/index.js";
  import { api, errorText, type SearchResult } from "$lib/api";
  import { ago, dayLabel } from "$lib/format";
  import { app } from "$lib/state.svelte";
  import { cn } from "$lib/utils";
  import Avatar from "./Avatar.svelte";
  import SearchIcon from "@lucide/svelte/icons/search";

  let open = $state(false);
  let query = $state("");
  let result: SearchResult | null = $state(null);
  let error = $state("");
  let active = $state(0);
  let timer: ReturnType<typeof setTimeout> | undefined;
  let listEl: HTMLUListElement | undefined = $state();

  $effect(() => {
    open = app.overlay === "search";
  });
  $effect(() => {
    if (!open && app.overlay === "search") app.overlay = null;
  });

  function run() {
    clearTimeout(timer);
    timer = setTimeout(async () => {
      try {
        result = await api.search(query, 60);
        error = "";
        active = 0;
      } catch (e) {
        error = errorText(e);
      }
    }, 120);
  }

  const groups = $derived.by(() => {
    const out: Array<{ space: string; name: string; hits: SearchResult["hits"] }> = [];
    for (const h of result?.hits ?? []) {
      let g = out.find((x) => x.space === h.message.space);
      if (!g) out.push((g = { space: h.message.space, name: h.spaceName, hits: [] }));
      g.hits.push(h);
    }
    return out;
  });
  const flat = $derived(groups.flatMap((g) => g.hits));

  function pick(i: number) {
    const h = flat[i];
    if (!h) return;
    app.overlay = null;
    const m = h.message;
    if (m.isReply && m.thread) void app.open(m.space, null, m.thread);
    else void app.open(m.space, m.name);
  }

  function onKeydown(e: KeyboardEvent) {
    if (e.key === "ArrowDown" || e.key === "ArrowUp") {
      e.preventDefault();
      if (!flat.length) return;
      active = (active + (e.key === "ArrowDown" ? 1 : -1) + flat.length) % flat.length;
      listEl?.querySelector(`#hit-${active}`)?.scrollIntoView({ block: "nearest" });
    } else if (e.key === "Enter") {
      e.preventDefault();
      pick(active);
    }
  }
</script>

<Dialog.Root bind:open>
  <Dialog.Content class="top-[12%] max-w-2xl translate-y-0 gap-0 p-0 sm:max-w-2xl" showCloseButton={false}>
    <Dialog.Title class="sr-only">Search messages</Dialog.Title>
    <Dialog.Description class="sr-only">Search all synced messages. Filters: from:, in:, before:, after:.</Dialog.Description>
    <div class="flex items-center gap-2 border-b border-border px-3">
      <SearchIcon class="size-4 text-muted-foreground" aria-hidden="true" />
      <input
        bind:value={query}
        oninput={run}
        onkeydown={onKeydown}
        class="selectable h-11 flex-1 bg-transparent text-[14px] outline-none placeholder:text-muted-foreground"
        placeholder="Search messages, e.g. deploy from:ron in:tech after:2026-10-01"
        aria-label="Search messages"
        role="combobox"
        aria-expanded={flat.length > 0}
        aria-controls="search-results"
        aria-activedescendant={flat.length ? `hit-${active}` : undefined}
        autocomplete="off"
        spellcheck="false"
      />
    </div>
    <ul id="search-results" bind:this={listEl} role="listbox" aria-label="Results" class="m-0 max-h-[60vh] list-none overflow-y-auto p-1">
      {#if error}
        <li class="px-3 py-6 text-center text-destructive" role="alert">{error}</li>
      {:else if query.trim() && result && flat.length === 0}
        <li class="px-3 py-6 text-center text-muted-foreground">No messages match. Search covers synced history only.</li>
      {:else if !query.trim()}
        <li class="px-3 py-5 text-[12px] text-muted-foreground">
          Filters: <code class="font-mono">from:name</code>, <code class="font-mono">in:space</code>,
          <code class="font-mono">before:2026-11-01</code>, <code class="font-mono">after:2026-10-01</code>.
        </li>
      {/if}
      {#each groups as g (g.space)}
        <li role="presentation" class="px-2 pt-2 pb-1 text-[12px] font-semibold text-muted-foreground">{g.name}</li>
        {#each g.hits as h (h.message.name)}
          {@const i = flat.indexOf(h)}
          <!-- Keyboard selection runs through the combobox input (arrows + Enter). -->
          <!-- svelte-ignore a11y_click_events_have_key_events -->
          <li
            id="hit-{i}"
            role="option"
            aria-selected={i === active}
            class={cn("flex cursor-default gap-2 rounded-md px-2 py-1.5", i === active && "bg-accent-soft")}
            onmousemove={() => (active = i)}
            onclick={() => pick(i)}
          >
            <Avatar user={h.message.sender} size={24} />
            <div class="min-w-0 flex-1">
              <div class="flex items-baseline gap-2">
                <span class="font-semibold">{h.message.sender.displayName}</span>
                <span class="text-[12px] text-muted-foreground">{ago(h.message.createUs)}</span>
                {#if h.message.isReply}<span class="text-[12px] text-muted-foreground">in a thread</span>{/if}
              </div>
              <p class="m-0 line-clamp-2 text-[13px]">
                {#each h.snippet as p, j (j)}{#if p.hit}<mark class="rounded-sm bg-highlight-strong px-px text-foreground">{p.text}</mark>{:else}{p.text}{/if}{/each}
              </p>
            </div>
          </li>
        {/each}
      {/each}
    </ul>
    {#if result?.coveredFromUs}
      <p class="m-0 border-t border-border px-3 py-1.5 text-[11px] text-muted-foreground">
        Searching messages synced since {dayLabel(result.coveredFromUs)}.
      </p>
    {/if}
  </Dialog.Content>
</Dialog.Root>
