<script lang="ts">
  import { localInput, presets, upcoming } from "$lib/when";
  import type { Snippet } from "svelte";

  /** Quick times plus a date and time of your own, for reminders and send later. */
  let { title, onpick, children }: { title: string; onpick: (atUs: number) => void; children?: Snippet } = $props();

  const choices = presets();
  let custom = $state(localInput(Date.now() * 1000 + 3_600_000_000));
  const customUs = $derived(custom ? new Date(custom).getTime() * 1000 : NaN);
  const valid = $derived(Number.isFinite(customUs) && customUs > Date.now() * 1000);
</script>

<div class="flex flex-col gap-0.5">
  <p class="m-0 px-2 pb-1 text-[12px] font-semibold text-muted-foreground">{title}</p>
  {#each choices as c (c.label)}
    <button class="flex items-baseline justify-between gap-4 rounded-md px-2 py-1.5 text-left text-[13px] hover:bg-muted focus-visible:bg-muted focus-visible:outline-none" onclick={() => onpick(c.atUs)}>
      <span>{c.label}</span>
      <span class="text-[12px] text-muted-foreground tabular-nums">{upcoming(c.atUs)}</span>
    </button>
  {/each}
  <form
    class="mt-1 flex items-center gap-1.5 border-t border-border px-1 pt-2"
    onsubmit={(e) => {
      e.preventDefault();
      if (valid) onpick(customUs);
    }}
  >
    <input
      type="datetime-local"
      bind:value={custom}
      aria-label="Date and time"
      class="selectable h-8 min-w-0 flex-1 rounded-md border border-input bg-background px-2 text-[13px] outline-none focus:border-accent focus:ring-2 focus:ring-accent/30"
    />
    <button type="submit" disabled={!valid} class="h-8 rounded-md bg-primary px-3 text-[13px] font-medium text-primary-foreground hover:bg-primary/85 disabled:opacity-40">Set</button>
  </form>
  {@render children?.()}
</div>
