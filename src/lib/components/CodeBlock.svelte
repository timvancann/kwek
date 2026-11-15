<script lang="ts">
  /** A code block: plain at once, coloured once the highlighter has loaded and is sure of the
   * language. */
  let { text }: { text: string } = $props();

  let html = $state<string | null>(null);
  $effect(() => {
    const t = text;
    let live = true;
    void import("$lib/highlight").then(({ highlight }) => {
      if (live) html = highlight(t);
    });
    return () => {
      live = false;
    };
  });
</script>

<pre class="my-2 overflow-x-auto rounded-md border border-border bg-muted px-3.5 py-2.5 font-mono text-[13px] leading-relaxed"><code
    class="hl"
    >{#if html}{@html html}{:else}{text}{/if}</code
  ></pre>

<style>
  /* A quiet palette on the light grey block; contrast checked against --muted (#efeff3). */
  .hl :global(.hljs-keyword),
  .hl :global(.hljs-selector-tag),
  .hl :global(.hljs-built_in),
  .hl :global(.hljs-literal) {
    color: #8a3ab9;
  }
  .hl :global(.hljs-string),
  .hl :global(.hljs-regexp),
  .hl :global(.hljs-addition) {
    color: #1a6b2f;
  }
  .hl :global(.hljs-number),
  .hl :global(.hljs-symbol),
  .hl :global(.hljs-attr),
  .hl :global(.hljs-attribute) {
    color: #a3480b;
  }
  .hl :global(.hljs-title),
  .hl :global(.hljs-section),
  .hl :global(.hljs-type),
  .hl :global(.hljs-name) {
    color: #0b5f8f;
  }
  .hl :global(.hljs-comment),
  .hl :global(.hljs-quote),
  .hl :global(.hljs-meta) {
    color: #5b5b66;
    font-style: italic;
  }
  .hl :global(.hljs-deletion) {
    color: #b42318;
  }
</style>
