<script lang="ts">
  import { app } from "$lib/state.svelte";
  import Composer from "./Composer.svelte";
  import MessageList from "./MessageList.svelte";
  import X from "@lucide/svelte/icons/x";
  import ChevronLeft from "@lucide/svelte/icons/chevron-left";
  import { cn } from "$lib/utils";

  const t = $derived(app.thread);
  const spaceName = $derived(app.spaces.find((s) => s.name === t?.space)?.displayName ?? "");
</script>

{#if t}
  <!-- Below 1000px the panel overlays the channel instead of squeezing it. -->
  <aside
    class="flex w-[420px] shrink-0 flex-col border-l border-border bg-background max-[1000px]:absolute max-[1000px]:inset-y-0 max-[1000px]:right-0 max-[1000px]:z-20 max-[1000px]:w-[min(420px,100%)] max-[1000px]:shadow-xl"
    aria-label="Thread"
  >
    <header
      data-tauri-drag-region
      class={cn("flex h-14 shrink-0 items-center gap-2.5 border-b border-border pr-3 pl-6", app.phone && "h-auto min-h-14 gap-1.5 pt-[var(--safe-top)] pl-1")}
    >
      {#if app.phone}
        <button
          class="inline-flex size-11 shrink-0 items-center justify-center rounded-md text-muted-foreground"
          aria-label="Back"
          onclick={() => history.back()}
        >
          <ChevronLeft class="size-6" />
        </button>
      {/if}
      <h2 data-tauri-drag-region class="font-display text-[17px] font-semibold">Thread</h2>
      <span data-tauri-drag-region class="min-w-0 truncate text-[12px] text-muted-foreground">{spaceName}</span>
      {#if !app.phone}
      <button
        class="ml-auto inline-flex size-8 items-center justify-center rounded-md text-muted-foreground hover:bg-muted hover:text-foreground"
        aria-label="Close thread"
        title="Close thread (Esc)"
        onclick={() => app.closeThread()}
      >
        <X class="size-4" />
      </button>
      {/if}
    </header>
    <MessageList messages={t.messages} key={t.name} where="thread" label="Thread in {spaceName}" />
    <Composer where="thread" space={t.space} thread={t.name} label="Reply in thread" />
  </aside>
{/if}
