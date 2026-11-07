<script lang="ts">
  import type { Inline } from "$lib/bindings/Inline";
  import { api } from "$lib/api";
  import { cn } from "$lib/utils";
  import Video from "@lucide/svelte/icons/video";
  import FileText from "@lucide/svelte/icons/file-text";

  let { inlines, tabbable = true }: { inlines: Inline[]; tabbable?: boolean } = $props();
  const tab = $derived(tabbable ? 0 : -1);

  function open(e: MouseEvent, url: string) {
    e.preventDefault();
    void api.openUrl(url);
  }
</script>

{#each inlines as i, idx (idx)}
  {#if i.kind === "text"}<span class={cn(i.bold && "font-semibold", i.italic && "italic", i.strike && "line-through")}>{i.text}</span>
  {:else if i.kind === "code"}<code class="rounded-sm bg-muted px-1 py-px font-mono text-[12px] text-foreground">{i.text}</code>
  {:else if i.kind === "mention"}<span
      class={cn("rounded-sm px-0.5 font-medium", i.me ? "bg-highlight-strong text-foreground" : "bg-accent-soft text-accent")}
      >@{i.label}</span
    >
  {:else if i.kind === "link"}
    {#if i.rich}
      <a
        href={i.url}
        tabindex={tab}
        onclick={(e) => open(e, i.url)}
        class="mx-px inline-flex items-center gap-1 rounded-sm border border-border bg-muted px-1.5 py-px align-baseline text-accent no-underline hover:border-accent"
      >
        {#if i.rich === "meet"}<Video class="size-3.5" aria-hidden="true" />{:else}<FileText class="size-3.5" aria-hidden="true" />{/if}
        <span class="max-w-[28ch] truncate">{i.label}</span>
      </a>
    {:else}
      <a href={i.url} tabindex={tab} onclick={(e) => open(e, i.url)} class="break-words text-accent underline decoration-accent/40 underline-offset-2 hover:decoration-accent"
        >{i.label}</a
      >
    {/if}
  {:else if i.kind === "break"}<br />
  {/if}
{/each}
