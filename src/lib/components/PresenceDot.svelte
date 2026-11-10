<script lang="ts">
  import { cn } from "$lib/utils";

  /** `ring` is the colour behind the dot, so it reads as cut out of the avatar. */
  let {
    state,
    size = 9,
    ring = "var(--background)",
    class: className = "",
  }: { state: string | null | undefined; size?: number; ring?: string; class?: string } = $props();
</script>

<!-- Decorative: the state is always said in words nearby (label, header, card). -->
{#if state}
  <span
    aria-hidden="true"
    class={cn("dot relative", state, className)}
    style:width="{size}px"
    style:height="{size}px"
    style:--dot-ring={ring}
    style:--dot-line="{Math.max(2, Math.round(size * 0.24))}px"
  ></span>
{/if}

<style>
  .dot {
    display: inline-block;
    flex: none;
    border-radius: 9999px;
    box-sizing: border-box;
    box-shadow: 0 0 0 2px var(--dot-ring);
  }
  .active {
    background: var(--success);
  }
  .idle {
    background: var(--background);
    border: var(--dot-line) solid var(--success);
  }
  .away {
    background: var(--background);
    border: var(--dot-line) solid var(--input);
  }
  .dnd {
    background: var(--destructive);
  }
  .dnd::after {
    content: "";
    position: absolute;
    left: 22%;
    right: 22%;
    top: 50%;
    height: max(1.5px, 18%);
    transform: translateY(-50%);
    border-radius: 1px;
    background: var(--background);
  }
</style>
