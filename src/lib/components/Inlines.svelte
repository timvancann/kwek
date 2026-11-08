<script lang="ts">
  import type { Inline } from "$lib/bindings/Inline";
  import { api } from "$lib/api";
  import { cn } from "$lib/utils";
  import SiteIcon from "./SiteIcon.svelte";

  let { inlines, tabbable = true }: { inlines: Inline[]; tabbable?: boolean } = $props();
  const tab = $derived(tabbable ? 0 : -1);

  function open(e: MouseEvent, url: string) {
    e.preventDefault();
    void api.openUrl(url);
  }

  function hostOf(url: string): string {
    try {
      return new URL(url).hostname.toLowerCase();
    } catch {
      return "";
    }
  }

  /** A bare link reads as its address without the noise (scheme, www, trailing slash), and
   * a very long one is trimmed; the full address is in the tooltip. Named links keep their
   * text. */
  function linkText(label: string, url: string): string {
    if (!label.trim()) return hostOf(url) || url;
    if (label !== url && !/^https?:\/\//i.test(label)) return label;
    const short = [...label.replace(/^https?:\/\//i, "").replace(/^www\./i, "").replace(/\/$/, "")];
    return short.length > 64 ? short.slice(0, 61).join("") + "…" : short.join("");
  }

  /** The first character (a whole emoji, not half of one) and the rest. */
  function head(text: string): [string, string] {
    const [first = "", ...rest] = [...text];
    return [first, rest.join("")];
  }
</script>

{#each inlines as i, idx (idx)}
  {#if i.kind === "text"}<span class={cn(i.bold && "font-semibold", i.italic && "italic", i.strike && "line-through")}>{i.text}</span>
  {:else if i.kind === "code"}<code class="rounded-sm bg-muted px-1 py-px font-mono text-[0.85em] text-foreground">{i.text}</code>
  {:else if i.kind === "mention"}<span
      class={cn("rounded-sm px-0.5 font-medium", i.me ? "bg-highlight-strong text-foreground" : "bg-accent-soft text-accent")}
      >@{i.label}</span
    >
  {:else if i.kind === "link"}
    {@const [first, rest] = head(linkText(i.label, i.url))}
    <!-- The site's icon, then the link: GitHub, Docs, Meet and the rest recognisable at a glance.
         The icon sticks to the first letter, so a line wrap never strands it. -->
    <a
      href={i.url}
      title={i.url}
      tabindex={tab}
      onclick={(e) => open(e, i.url)}
      class="break-words text-accent no-underline decoration-accent/50 underline-offset-2 hover:underline"
      ><span class="whitespace-nowrap"><SiteIcon host={hostOf(i.url)} />{first}</span>{rest}</a
    >
  {:else if i.kind === "break"}<br />
  {/if}
{/each}
