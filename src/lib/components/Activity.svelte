<script lang="ts">
  import * as Dialog from "$lib/components/ui/dialog/index.js";
  import { api, errorText, type ListedMessage, type MessageView } from "$lib/api";
  import { ago } from "$lib/format";
  import { app, type ActivityTab } from "$lib/state.svelte";
  import { upcoming } from "$lib/when";
  import { cn } from "$lib/utils";
  import { toast } from "svelte-sonner";
  import type { Block } from "$lib/bindings/Block";
  import type { Inline } from "$lib/bindings/Inline";
  import Avatar from "./Avatar.svelte";
  import X from "@lucide/svelte/icons/x";

  let open = $state(false);
  $effect(() => {
    open = app.overlay === "activity";
  });
  $effect(() => {
    if (!open && app.overlay === "activity") app.overlay = null;
  });

  const TABS: Array<{ id: ActivityTab; label: string }> = [
    { id: "activity", label: "Mentions and replies" },
    { id: "saved", label: "Saved" },
    { id: "reminders", label: "Reminders" },
    { id: "scheduled", label: "Scheduled" },
  ];

  let items = $state<ListedMessage[]>([]);
  let loading = $state(false);
  $effect(() => {
    const tab = app.activityTab;
    void app.laterNonce;
    if (!open || tab === "scheduled") return;
    loading = true;
    api
      .listedMessages(tab)
      .then((x) => {
        if (app.activityTab === tab) items = x;
      })
      .catch((e) => toast.error(errorText(e)))
      .finally(() => (loading = false));
  });

  /** One line of the message, for the list. */
  function snippet(m: MessageView): string {
    const inl = (xs: Inline[]) =>
      xs.map((i) => (i.kind === "mention" ? `@${i.label}` : i.kind === "link" ? i.label : i.kind === "break" ? " " : i.text)).join("");
    const walk = (bs: Block[]): string =>
      bs
        .map((b) =>
          b.kind === "paragraph" || b.kind === "heading" ? inl(b.inlines) : b.kind === "code" ? b.text : b.kind === "quote" ? walk(b.blocks) : b.kind === "list" ? b.items.map(walk).join(" ") : "",
        )
        .join(" ");
    const t = walk(m.blocks).trim();
    return t || (m.attachments.length ? "An attachment" : "");
  }

  function go(x: ListedMessage) {
    app.overlay = null;
    const m = x.message;
    if (m.isReply && m.thread) void app.openInThread(m.space, m.thread, m.name);
    else void app.open(m.space, m.name);
  }

  function goSpace(space: string, thread: string | null) {
    app.overlay = null;
    void app.open(space, null, thread);
  }

  async function act(f: () => Promise<void>) {
    try {
      await f();
    } catch (e) {
      toast.error(errorText(e));
    }
  }

  const empty: Record<ActivityTab, string> = {
    activity: "No mentions or replies yet. Replies show here for threads you wrote in.",
    saved: "Nothing saved. Save a message with the bookmark on it, or press s.",
    reminders: "No reminders. Set one with the alarm clock on a message, or press m.",
    scheduled: "Nothing scheduled. Use the arrow next to Send to send a message later.",
  };
</script>

<Dialog.Root bind:open>
  <Dialog.Content class="top-[10%] max-w-xl translate-y-0 gap-0 p-0 sm:max-w-xl">
    <div class="border-b border-border px-5 pt-4">
      <Dialog.Title>Activity</Dialog.Title>
      <Dialog.Description class="sr-only">Mentions of you, replies in your threads, saved messages, reminders and scheduled messages.</Dialog.Description>
      <div class="mt-2 -mb-px flex gap-4" role="tablist" aria-label="Activity">
        {#each TABS as t (t.id)}
          <button
            role="tab"
            aria-selected={app.activityTab === t.id}
            class={cn(
              "border-b-2 pb-2 text-[13px]",
              app.activityTab === t.id ? "border-foreground font-medium text-foreground" : "border-transparent text-muted-foreground hover:text-foreground",
            )}
            onclick={() => (app.activityTab = t.id)}
            >{t.label}{#if t.id === "scheduled" && app.scheduled.length}<span class="ml-1 tabular-nums">({app.scheduled.length})</span>{/if}</button
          >
        {/each}
      </div>
    </div>
    <div class="max-h-[65vh] overflow-y-auto p-2" role="tabpanel">
      {#if app.activityTab === "scheduled"}
        {#if app.scheduled.length === 0}
          <p class="m-3 text-[14px] text-muted-foreground">{empty.scheduled}</p>
        {/if}
        <ul class="m-0 list-none p-0">
          {#each app.scheduled as s (s.id)}
            <li class="rounded-lg px-3 py-2 hover:bg-hover">
              <div class="flex items-baseline gap-2 text-[13px]">
                <button class="min-w-0 truncate font-medium hover:underline" onclick={() => goSpace(s.space, s.thread)}
                  >{s.spaceName || "A conversation"}{s.thread ? ", in a thread" : ""}</button
                >
                <span class="shrink-0 text-muted-foreground">{upcoming(s.atUs)}</span>
                <span class="flex-1"></span>
                <button class="shrink-0 rounded-md px-2 py-0.5 text-[12px] font-medium text-accent hover:bg-muted" onclick={() => act(() => api.sendScheduledNow(s.id))}
                  >Send now</button
                >
                <button class="shrink-0 rounded-md px-2 py-0.5 text-[12px] text-muted-foreground hover:bg-muted hover:text-destructive" onclick={() => act(() => api.cancelScheduled(s.id))}
                  >Cancel</button
                >
              </div>
              <p class="m-0 mt-0.5 line-clamp-2 text-[14px] break-words whitespace-pre-line">{s.text}</p>
              {#if s.files.length}<p class="m-0 mt-0.5 truncate text-[12px] text-muted-foreground">With {s.files.join(", ")}</p>{/if}
            </li>
          {/each}
        </ul>
      {:else if !loading && items.length === 0}
        <p class="m-3 text-[14px] text-muted-foreground">{empty[app.activityTab]}</p>
      {:else}
        <ul class="m-0 list-none p-0">
          {#each items as x (x.message.name)}
            <li class="group relative">
              <button class="flex w-full gap-3 rounded-lg px-3 py-2 text-left hover:bg-hover focus-visible:bg-hover focus-visible:outline-none" onclick={() => go(x)}>
                <Avatar user={x.message.sender} size={28} class="mt-0.5 shrink-0" />
                <span class="min-w-0 flex-1">
                  <span class="flex items-baseline gap-1.5 text-[13px]">
                    <span class="truncate font-semibold">{x.message.sender.displayName}</span>
                    <span class="shrink-0 text-muted-foreground"
                      >{x.why === "mention" ? "mentioned you" : x.why === "reply" ? "replied" : ""} in {x.spaceName || "a conversation"}</span
                    >
                    {#if x.unread}<span class="size-2 shrink-0 self-center rounded-full bg-accent" aria-label="unread"></span>{/if}
                    <span class="ml-auto shrink-0 pl-2 text-[12px] text-muted-foreground"
                      >{x.why === "reminder" ? upcoming(x.atUs) : ago(x.message.createUs)}</span
                    >
                  </span>
                  <span class="mt-0.5 line-clamp-2 block text-[14px] break-words">{snippet(x.message)}</span>
                </span>
              </button>
              {#if x.why === "saved" || x.why === "reminder"}
                <button
                  class="absolute top-2 right-2 hidden size-6 place-items-center rounded-md bg-background text-muted-foreground shadow-sm group-hover:grid group-focus-within:grid hover:text-foreground"
                  aria-label={x.why === "saved" ? "Remove from saved" : "Remove the reminder"}
                  title={x.why === "saved" ? "Remove from saved" : "Remove the reminder"}
                  onclick={() => act(() => (x.why === "saved" ? api.setSaved(x.message.name, false) : api.setReminder(x.message.name, null)))}
                >
                  <X class="size-3.5" aria-hidden="true" />
                </button>
              {/if}
            </li>
          {/each}
        </ul>
      {/if}
    </div>
  </Dialog.Content>
</Dialog.Root>
