<script lang="ts">
  import type { Block } from "$lib/bindings/Block";
  import Blocks from "./Blocks.svelte";
  import Inlines from "./Inlines.svelte";

  let { blocks }: { blocks: Block[] } = $props();
</script>

{#each blocks as b, idx (idx)}
  {#if b.kind === "paragraph"}
    <p class="my-0 [&+*]:mt-1.5"><Inlines inlines={b.inlines} /></p>
  {:else if b.kind === "heading"}
    <p class="my-0 font-semibold [&+*]:mt-1"><Inlines inlines={b.inlines} /></p>
  {:else if b.kind === "list"}
    {#if b.ordered}
      <ol class="my-1 list-decimal pl-5" start={b.start}>
        {#each b.items as item, j (j)}<li class="pl-0.5"><Blocks blocks={item} /></li>{/each}
      </ol>
    {:else}
      <ul class="my-1 list-disc pl-5 marker:text-muted-foreground">
        {#each b.items as item, j (j)}<li class="pl-0.5"><Blocks blocks={item} /></li>{/each}
      </ul>
    {/if}
  {:else if b.kind === "quote"}
    <blockquote class="my-1 border-l-2 border-border pl-3 text-muted-foreground"><Blocks blocks={b.blocks} /></blockquote>
  {:else if b.kind === "code"}
    <pre class="my-1 overflow-x-auto rounded-md border border-border bg-muted px-3 py-2 font-mono text-[12px] leading-relaxed"><code
        >{b.text}</code
      ></pre>
  {:else if b.kind === "rule"}
    <hr class="my-2 border-border" />
  {/if}
{/each}
