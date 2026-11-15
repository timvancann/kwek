<script lang="ts">
  import { emoji, GROUPS, quickRow, searchEmoji, usedEmoji, type Emoji } from "$lib/emoji.svelte";
  import { cn } from "$lib/utils";

  /** The reaction picker and the composer's emoji button. `verb` names what a pick does, for
   * screen readers ("React with", "Insert"); `marked` are shown as chosen (your reactions). */
  let {
    onpick,
    verb,
    marked = new Set<string>(),
  }: { onpick: (e: string) => void; verb: string; marked?: Set<string> } = $props();

  let filter = $state("");
  let scroller: HTMLDivElement | undefined = $state();
  const matches = $derived(filter.trim() ? searchEmoji(filter, 200) : null);
  const sections = $derived(
    GROUPS.map((g) => ({ ...g, items: emoji.list.filter((x) => x.group === g.id) })).filter((g) => g.items.length > 0),
  );
  const quick = $derived(quickRow());
  const byChar = $derived(new Map(emoji.list.map((x) => [x.e, x])));

  function pick(e: string) {
    usedEmoji(e);
    onpick(e);
  }

  function name(x: Emoji | undefined, e: string) {
    return x ? `:${x.codes[0]}:` : e;
  }

  function jump(id: number) {
    scroller?.querySelector(`[data-group="${id}"]`)?.scrollIntoView({ block: "start" });
  }
</script>

<div class="flex flex-col gap-2">
  <!-- svelte-ignore a11y_autofocus: The picker opens to find an emoji; typing goes here. -->
  <input
    bind:value={filter}
    autofocus
    class="selectable h-8 rounded-md border border-input bg-background px-2 outline-none focus:border-accent focus:ring-2 focus:ring-accent/30"
    placeholder="Find an emoji, Enter picks the first"
    aria-label="Find an emoji by name"
    onkeydown={(e) => {
      if (e.key === "Enter" && matches?.[0]) {
        e.preventDefault();
        pick(matches[0].e);
      }
    }}
  />
  {#if !matches}
    <div class="flex gap-1" role="group" aria-label="Recently used">
      {#each quick as e (e)}
        <button
          class={cn("emoji", marked.has(e) && "mine")}
          aria-pressed={marked.size ? marked.has(e) : undefined}
          title={name(byChar.get(e), e)}
          aria-label="{verb} {e}"
          onclick={() => pick(e)}>{e}</button
        >
      {/each}
    </div>
    <div class="flex justify-between border-b border-border pb-1" role="group" aria-label="Jump to a section">
      {#each sections as g (g.id)}
        <button class="nav" title={g.label} aria-label="Jump to {g.label}" onclick={() => jump(g.id)}>{g.items[0].e}</button>
      {/each}
    </div>
  {/if}
  <div bind:this={scroller} class="h-56 overflow-y-auto" role="region" aria-label={matches ? "Matching emoji" : "All emoji"}>
    {#if matches}
      {#if matches.length === 0}
        <p class="m-0 px-1 py-2 text-[13px] text-muted-foreground">{emoji.list.length ? "No emoji by that name." : "Loading emoji…"}</p>
      {/if}
      <div class="grid grid-cols-8 gap-0.5">
        {#each matches as x (x.e)}
          <button
            class={cn("emoji", marked.has(x.e) && "mine")}
            aria-pressed={marked.size ? marked.has(x.e) : undefined}
            title={name(x, x.e)}
            aria-label="{verb} {x.e} {x.label}"
            onclick={() => pick(x.e)}>{x.e}</button
          >
        {/each}
      </div>
    {:else}
      {#each sections as g (g.id)}
        <h3 data-group={g.id} class="sticky top-0 m-0 bg-popover py-1 text-[11px] font-semibold text-muted-foreground">{g.label}</h3>
        <div class="grid grid-cols-8 gap-0.5" role="group" aria-label={g.label}>
          {#each g.items as x (x.e)}
            <button
              class={cn("emoji", marked.has(x.e) && "mine")}
              aria-pressed={marked.size ? marked.has(x.e) : undefined}
              title={name(x, x.e)}
              aria-label="{verb} {x.e} {x.label}"
              onclick={() => pick(x.e)}>{x.e}</button
            >
          {/each}
        </div>
      {/each}
    {/if}
  </div>
</div>

<style>
  .emoji,
  .nav {
    border-radius: 6px;
    text-align: center;
  }
  .emoji {
    width: 34px;
    height: 34px;
    font-size: 20px;
    line-height: 34px;
  }
  .nav {
    width: 28px;
    height: 28px;
    font-size: 16px;
    line-height: 28px;
    filter: grayscale(1);
    opacity: 0.7;
  }
  .nav:hover {
    filter: none;
    opacity: 1;
  }
  .emoji:hover,
  .nav:hover {
    background: var(--muted);
  }
  .emoji:focus-visible,
  .nav:focus-visible {
    outline: 2px solid var(--ring);
    outline-offset: -2px;
  }
  .emoji.mine {
    background: var(--accent-soft);
    box-shadow: inset 0 0 0 1px var(--accent);
  }
</style>
