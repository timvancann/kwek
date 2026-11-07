<script lang="ts">
  import type { MessageView } from "$lib/api";
  import { api, errorText } from "$lib/api";
  import { ago, clock, stamp } from "$lib/format";
  import { app } from "$lib/state.svelte";
  import { cn } from "$lib/utils";
  import { toast } from "svelte-sonner";
  import Avatar from "./Avatar.svelte";
  import Blocks from "./Blocks.svelte";
  import SmilePlus from "@lucide/svelte/icons/smile-plus";
  import MessageSquare from "@lucide/svelte/icons/message-square-text";
  import Pencil from "@lucide/svelte/icons/pencil";
  import Trash from "@lucide/svelte/icons/trash-2";
  import LinkIcon from "@lucide/svelte/icons/link";
  import External from "@lucide/svelte/icons/external-link";
  import Paperclip from "@lucide/svelte/icons/paperclip";
  import HardDrive from "@lucide/svelte/icons/hard-drive";

  let {
    m,
    grouped = false,
    selected = false,
    where = "channel",
    onselect,
  }: {
    m: MessageView;
    grouped?: boolean;
    selected?: boolean;
    where?: "channel" | "thread";
    onselect?: () => void;
  } = $props();

  const label = $derived(`${m.sender.displayName}, ${stamp(m.createUs)}${m.edited ? ", edited" : ""}`);

  async function download(resource: string, name: string) {
    try {
      await api.downloadAttachment(resource, name);
      toast.success(`Saved ${name} to Downloads`);
    } catch (e) {
      toast.error(errorText(e));
    }
  }

  function retry() {
    void api.retrySend(m.name).catch((e) => toast.error(errorText(e)));
  }
  function discard() {
    void api.discardFailed(m.name).catch((e) => toast.error(errorText(e)));
  }
</script>

<!-- WAI-ARIA feed pattern: articles are focusable and receive focus as j/k move the
     selection, so screen readers follow it. Click selects; the keyboard path is j/k. -->
<!-- svelte-ignore a11y_no_noninteractive_tabindex, a11y_click_events_have_key_events, a11y_no_noninteractive_element_interactions -->
<article
  data-message={m.name}
  tabindex={selected ? 0 : -1}
  aria-label={label}
  aria-current={selected ? "true" : undefined}
  onclick={onselect}
  onfocus={onselect}
  class={cn(
    "group relative flex gap-2.5 px-4 outline-none",
    grouped ? "py-0.5" : "pt-2 pb-0.5",
    m.mentionsMe && !m.deleted && "bg-highlight/70",
    selected ? "bg-accent-soft" : "hover:bg-muted",
    m.localState === "pending" && "opacity-60",
  )}
>
  <div class="w-8 shrink-0">
    {#if !grouped}
      <Avatar user={m.sender} />
    {:else}
      <time
        datetime={new Date(m.createUs / 1000).toISOString()}
        class={cn("block pt-0.5 text-right text-[11px] leading-5 text-muted-foreground opacity-0 group-hover:opacity-100", selected && "opacity-100")}
        >{clock(m.createUs)}</time
      >
    {/if}
  </div>

  <div class="min-w-0 flex-1">
    {#if !grouped}
      <div class="flex items-baseline gap-2">
        <span class="font-semibold">{m.sender.displayName}</span>
        <time datetime={new Date(m.createUs / 1000).toISOString()} class="text-[12px] text-muted-foreground" title={stamp(m.createUs)}
          >{clock(m.createUs)}</time
        >
      </div>
    {/if}

    {#if m.deleted}
      <p class="my-0 text-muted-foreground italic">Message deleted</p>
    {:else}
      <div class="selectable leading-[1.5] break-words">
        <Blocks blocks={m.blocks} />
        {#if m.edited}<span class="ml-1 text-[12px] text-muted-foreground">(edited)</span>{/if}
      </div>

      {#if m.hasCards}
        <p class="my-1 text-[12px] text-muted-foreground">
          Card content from an app.
          {#if m.webUrl}<button class="text-accent underline underline-offset-2" onclick={() => api.openUrl(m.webUrl!)}>Open in Google Chat</button>{/if}
        </p>
      {/if}

      {#if m.attachments.length}
        <div class="mt-1.5 flex flex-wrap gap-2">
          {#each m.attachments as a, i (i)}
            {#if a.kind === "image" && a.mediaUrl}
              <button
                class="block overflow-hidden rounded-md border border-border"
                onclick={() => a.resource && download(a.resource, a.name)}
                aria-label="Image {a.name}, save to Downloads"
              >
                <img src={a.mediaUrl} alt={a.name} loading="lazy" class="block max-h-72 max-w-[min(360px,100%)] object-contain" draggable="false" />
              </button>
            {:else}
              <button
                class="inline-flex max-w-xs items-center gap-1.5 rounded-md border border-border bg-muted px-2 py-1 text-left hover:border-accent"
                onclick={() => (a.openUrl ? api.openUrl(a.openUrl) : a.resource && download(a.resource, a.name))}
              >
                {#if a.kind === "drive"}<HardDrive class="size-4 shrink-0 text-muted-foreground" aria-hidden="true" />{:else}<Paperclip
                    class="size-4 shrink-0 text-muted-foreground"
                    aria-hidden="true"
                  />{/if}
                <span class="truncate">{a.name}</span>
                <span class="sr-only">{a.kind === "drive" ? ", open in Google Drive" : ", save to Downloads"}</span>
              </button>
            {/if}
          {/each}
        </div>
      {/if}

      {#if m.reactions.length}
        <div class="mt-1 flex flex-wrap gap-1">
          {#each m.reactions as r (r.key)}
            <button
              class={cn(
                "inline-flex h-6 items-center gap-1 rounded-full border px-2 text-[12px]",
                r.mine ? "border-accent bg-accent-soft text-accent" : "border-border bg-background hover:border-input",
              )}
              aria-pressed={r.mine}
              aria-label="{r.emoji} {r.count} {r.count === 1 ? 'reaction' : 'reactions'}{r.mine ? ', including yours' : ''}"
              onclick={() => app.react(m, r.emoji)}
            >
              <span aria-hidden="true">{r.emoji}</span><span aria-hidden="true" class="tabular-nums">{r.count}</span>
            </button>
          {/each}
        </div>
      {/if}
    {/if}

    {#if m.localState === "failed"}
      <p class="my-1 flex items-center gap-2 text-[12px] text-destructive" role="status">
        Not sent.
        <button class="font-medium underline underline-offset-2" onclick={retry}>Retry</button>
        <button class="text-muted-foreground underline underline-offset-2" onclick={discard}>Discard</button>
      </p>
    {:else if m.localState === "pending"}
      <span class="sr-only" role="status">Sending</span>
    {/if}

    {#if m.threadSummary && where === "channel"}
      <button
        class="-ml-1 mt-1 flex items-center gap-2 rounded-md px-1 py-0.5 text-[12px] hover:bg-background"
        onclick={(e) => {
          e.stopPropagation();
          if (m.thread) void app.openThread(m.thread);
        }}
      >
        <span class="flex -space-x-1">
          {#each m.threadSummary.participants as p (p.name)}
            <Avatar user={p} size={18} class="ring-2 ring-background" />
          {/each}
        </span>
        <span class="font-semibold text-accent">
          {m.threadSummary.replyCount}
          {m.threadSummary.replyCount === 1 ? "reply" : "replies"}
        </span>
        {#if m.threadSummary.unread}<span class="rounded-full bg-highlight-strong px-1.5 font-medium text-foreground">New</span>{/if}
        <span class="text-muted-foreground">Last reply {ago(m.threadSummary.lastReplyUs)}</span>
      </button>
    {/if}
  </div>

  {#if !m.deleted && m.localState === "synced"}
    <div
      class={cn(
        "absolute -top-3 right-4 z-10 hidden items-center rounded-md border border-border bg-background p-0.5 shadow-sm group-hover:flex group-focus-within:flex",
        selected && "flex",
      )}
      role="toolbar"
      aria-label="Message actions"
    >
      <button class="act" aria-label="React" title="React (+)" onclick={(e) => (e.stopPropagation(), (app.reactTo = m))}><SmilePlus /></button>
      {#if where === "channel" && m.thread}
        <button class="act" aria-label="Reply in thread" title="Reply in thread (r)" onclick={(e) => (e.stopPropagation(), app.openThread(m.thread!))}
          ><MessageSquare /></button
        >
      {/if}
      {#if m.isOwn}
        <button class="act" aria-label="Edit" title="Edit (e)" onclick={(e) => (e.stopPropagation(), app.startEdit(m, where))}><Pencil /></button>
        <button class="act" aria-label="Delete" title="Delete (dd)" onclick={(e) => (e.stopPropagation(), (app.confirmDelete = m))}><Trash /></button>
      {/if}
      {#if m.webUrl}
        <button class="act" aria-label="Copy link" title="Copy link (y)" onclick={(e) => (e.stopPropagation(), app.copyLink(m))}><LinkIcon /></button>
        <button class="act" aria-label="Open in Google Chat" title="Open in Google Chat" onclick={(e) => (e.stopPropagation(), api.openUrl(m.webUrl!))}
          ><External /></button
        >
      {/if}
    </div>
  {/if}
</article>

<style>
  .act {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 28px;
    height: 28px;
    border-radius: 4px;
    color: var(--muted-foreground);
  }
  .act:hover {
    background: var(--muted);
    color: var(--foreground);
  }
  .act :global(svg) {
    width: 16px;
    height: 16px;
  }
</style>
