<script lang="ts">
  import type { Block } from "$lib/bindings/Block";
  import Blocks from "./Blocks.svelte";
  import Inlines from "./Inlines.svelte";
  import CodeBlock from "./CodeBlock.svelte";

  let { blocks, tabbable = true }: { blocks: Block[]; tabbable?: boolean } = $props();
</script>

{#each blocks as b, idx (idx)}
  {#if b.kind === "paragraph"}
    <!-- Chat exports every typed line break as a paragraph, so paragraphs sit close. -->
    <p class="my-0 [&+*]:mt-1"><Inlines inlines={b.inlines} {tabbable} /></p>
  {:else if b.kind === "heading"}
    <p class="my-0 font-semibold [&+*]:mt-1.5"><Inlines inlines={b.inlines} {tabbable} /></p>
  {:else if b.kind === "list"}
    {#if b.ordered}
      <ol class="my-1.5 list-decimal space-y-0.5 pl-6" start={b.start}>
        {#each b.items as item, j (j)}<li class="pl-1"><Blocks blocks={item} {tabbable} /></li>{/each}
      </ol>
    {:else}
      <ul class="my-1.5 list-disc space-y-0.5 pl-6 marker:text-muted-foreground">
        {#each b.items as item, j (j)}<li class="pl-1"><Blocks blocks={item} {tabbable} /></li>{/each}
      </ul>
    {/if}
  {:else if b.kind === "quote"}
    <blockquote class="my-1.5 border-l-2 border-border pl-3 text-muted-foreground"><Blocks blocks={b.blocks} {tabbable} /></blockquote>
  {:else if b.kind === "code"}
    <CodeBlock text={b.text} />
  {:else if b.kind === "rule"}
    <hr class="my-3 border-border" />
  {/if}
{/each}
