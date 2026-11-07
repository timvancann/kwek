<script lang="ts">
  import { app } from "$lib/state.svelte";
  import Composer from "./Composer.svelte";
  import MessageList from "./MessageList.svelte";
  import X from "@lucide/svelte/icons/x";

  const t = $derived(app.thread);
  const spaceName = $derived(app.spaces.find((s) => s.name === t?.space)?.displayName ?? "");
</script>

{#if t}
  <!-- Below 1000px the panel overlays the channel instead of squeezing it. -->
  <aside
    class="flex w-[384px] shrink-0 flex-col border-l border-border bg-background max-[1000px]:absolute max-[1000px]:inset-y-0 max-[1000px]:right-0 max-[1000px]:z-20 max-[1000px]:w-[min(384px,100%)] max-[1000px]:shadow-xl"
    aria-label="Thread"
  >
    <header data-tauri-drag-region class="flex h-12 shrink-0 items-center gap-2 border-b border-border pr-2 pl-4">
      <h2 data-tauri-drag-region class="text-[15px] font-semibold">Thread</h2>
      <span data-tauri-drag-region class="min-w-0 truncate text-[12px] text-muted-foreground">{spaceName}</span>
      <button
        class="ml-auto inline-flex size-8 items-center justify-center rounded-md text-muted-foreground hover:bg-muted hover:text-foreground"
        aria-label="Close thread"
        title="Close thread (Esc)"
        onclick={() => app.closeThread()}
      >
        <X class="size-4" />
      </button>
    </header>
    <MessageList messages={t.messages} key={t.name} where="thread" label="Thread in {spaceName}" />
    <Composer where="thread" space={t.space} thread={t.name} label="Reply in thread" />
  </aside>
{/if}
