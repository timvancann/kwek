<script lang="ts">
  import * as Dialog from "$lib/components/ui/dialog/index.js";
  import { api, errorText, type MessageView } from "$lib/api";
  import { quickRow, usedEmoji } from "$lib/emoji.svelte";
  import { app } from "$lib/state.svelte";
  import { upcoming } from "$lib/when";
  import { toast } from "svelte-sonner";
  import type { Block } from "$lib/bindings/Block";
  import type { Inline } from "$lib/bindings/Inline";
  import WhenPicker from "./WhenPicker.svelte";
  import SmilePlus from "@lucide/svelte/icons/smile-plus";
  import MessageSquare from "@lucide/svelte/icons/message-square-text";
  import Copy from "@lucide/svelte/icons/copy";
  import LinkIcon from "@lucide/svelte/icons/link";
  import Bookmark from "@lucide/svelte/icons/bookmark";
  import AlarmClock from "@lucide/svelte/icons/alarm-clock";
  import Pencil from "@lucide/svelte/icons/pencil";
  import Trash from "@lucide/svelte/icons/trash-2";
  import External from "@lucide/svelte/icons/external-link";
  import Star from "@lucide/svelte/icons/star";
  import BellOff from "@lucide/svelte/icons/bell-off";
  import Bell from "@lucide/svelte/icons/bell";
  import EyeOff from "@lucide/svelte/icons/eye-off";
  import Eye from "@lucide/svelte/icons/eye";
  import CheckCheck from "@lucide/svelte/icons/check-check";

  // The long-press sheets on a phone: one for a message, one for a conversation in the list.

  const SHEET =
    "top-auto bottom-0 left-0 max-w-full translate-x-0 translate-y-0 gap-1 rounded-b-none p-2 pb-[calc(var(--safe-bottom)+12px)] sm:max-w-full data-open:zoom-in-100 data-closed:zoom-out-100 data-open:slide-in-from-bottom-10";

  let msgOpen = $state(false);
  let remindStep = $state(false);
  $effect(() => {
    msgOpen = app.sheet !== null;
    if (msgOpen) remindStep = false;
  });
  $effect(() => {
    if (!msgOpen && app.sheet) app.sheet = null;
  });

  let spaceOpen = $state(false);
  $effect(() => {
    spaceOpen = app.spaceSheet !== null;
  });
  $effect(() => {
    if (!spaceOpen && app.spaceSheet) app.spaceSheet = null;
  });

  /** Closes the sheet, then does `f`. */
  function then(f: () => void) {
    app.sheet = null;
    app.spaceSheet = null;
    f();
  }

  function plain(m: MessageView): string {
    const inl = (xs: Inline[]) =>
      xs.map((i) => (i.kind === "mention" ? `@${i.label}` : i.kind === "link" ? (i.label === i.url ? i.url : `${i.label} (${i.url})`) : i.kind === "break" ? "\n" : i.text)).join("");
    const walk = (bs: Block[]): string[] =>
      bs.flatMap((b) =>
        b.kind === "paragraph" || b.kind === "heading"
          ? [inl(b.inlines)]
          : b.kind === "code"
            ? [b.text]
            : b.kind === "quote"
              ? walk(b.blocks).map((l) => `> ${l}`)
              : b.kind === "list"
                ? b.items.flatMap((item, i) => walk(item).map((l, j) => (j === 0 ? `${b.ordered ? `${b.start + i}.` : "-"} ${l}` : l)))
                : [],
      );
    return walk(m.blocks).join("\n");
  }

  async function copy(text: string, what: string) {
    try {
      await navigator.clipboard.writeText(text);
      toast.success(`${what} copied`);
    } catch {
      toast.error(`Couldn't copy the ${what.toLowerCase()}.`);
    }
  }

  async function markRead(space: string, upTo: number) {
    try {
      await api.markRead(space, upTo);
    } catch (e) {
      toast.error(errorText(e));
    }
  }
</script>

<Dialog.Root bind:open={msgOpen}>
  <Dialog.Content class={SHEET} showCloseButton={false}>
    {#if app.sheet}
      {@const m = app.sheet.m}
      {@const where = app.sheet.where}
      <Dialog.Title class="sr-only">Message from {m.sender.displayName}</Dialog.Title>
      <Dialog.Description class="sr-only">What to do with this message.</Dialog.Description>
      {#if remindStep}
        <WhenPicker title="Remind me about this message" onpick={(at) => then(() => void app.remind(m, at))} />
      {:else}
        {@const mine = new Set(m.reactions.filter((r) => r.mine).map((r) => r.emoji))}
        <div class="flex justify-between px-1 pb-1" role="group" aria-label="React">
          {#each quickRow(6) as e (e)}
            <button
              class="grid size-11 place-items-center rounded-full text-[24px] {mine.has(e) ? 'bg-accent-soft' : 'bg-muted'}"
              aria-label="React with {e}"
              aria-pressed={mine.has(e)}
              onclick={() =>
                then(() => {
                  usedEmoji(e);
                  void app.react(m, e);
                })}>{e}</button
            >
          {/each}
          <button class="grid size-11 place-items-center rounded-full bg-muted text-muted-foreground" aria-label="More reactions" onclick={() => then(() => (app.reactTo = m))}
            ><SmilePlus class="size-5" /></button
          >
        </div>
        {#snippet row(label: string, Icon: typeof Copy, run: () => void, danger = false)}
          <button class="flex h-12 items-center gap-3.5 rounded-lg px-3 text-left text-[16px] active:bg-muted {danger ? 'text-destructive' : ''}" onclick={run}>
            <Icon class="size-5 shrink-0 {danger ? '' : 'text-muted-foreground'}" aria-hidden="true" />{label}
          </button>
        {/snippet}
        {#if where === "channel" && m.thread}
          {@render row("Reply in thread", MessageSquare, () => then(() => void app.openThread(m.thread!)))}
        {/if}
        {@render row("Copy text", Copy, () => then(() => void copy(plain(m), "Text")))}
        {#if m.webUrl}
          {@render row("Copy link", LinkIcon, () => then(() => void app.copyLink(m)))}
        {/if}
        {@render row(m.saved ? "Remove from saved" : "Save for later", Bookmark, () => then(() => void app.toggleSaved(m)))}
        {@render row(m.remindAtUs ? `Reminder ${upcoming(m.remindAtUs)}, change` : "Remind me", AlarmClock, () => (remindStep = true))}
        {#if m.remindAtUs}
          {@render row("Remove the reminder", AlarmClock, () => then(() => void app.remind(m, null)))}
        {/if}
        {#if m.isOwn}
          {@render row("Edit", Pencil, () => then(() => app.startEdit(m, where)))}
          {@render row("Delete", Trash, () => then(() => (app.confirmDelete = m)), true)}
        {/if}
        {#if m.webUrl}
          {@render row("Open in Google Chat", External, () => then(() => void api.openUrl(m.webUrl!)))}
        {/if}
      {/if}
    {/if}
  </Dialog.Content>
</Dialog.Root>

<Dialog.Root bind:open={spaceOpen}>
  <Dialog.Content class={SHEET} showCloseButton={false}>
    {#if app.spaceSheet}
      {@const s = app.spaceSheet}
      <Dialog.Title class="px-3 pt-2 pb-1 text-[15px]">{s.displayName}</Dialog.Title>
      <Dialog.Description class="sr-only">What to do with this conversation.</Dialog.Description>
      {#snippet row(label: string, Icon: typeof Star, run: () => void)}
        <button class="flex h-12 items-center gap-3.5 rounded-lg px-3 text-left text-[16px] active:bg-muted" onclick={run}>
          <Icon class="size-5 shrink-0 text-muted-foreground" aria-hidden="true" />{label}
        </button>
      {/snippet}
      {#if s.unread > 0 || s.mentions > 0}
        {@render row("Mark as read", CheckCheck, () => then(() => void markRead(s.name, s.lastActiveUs)))}
      {/if}
      {@render row(s.starred ? "Unstar" : "Star", Star, () => then(() => void app.toggleStar(s)))}
      {@render row(s.muted ? "Unmute" : "Mute", s.muted ? Bell : BellOff, () => then(() => void app.toggleMute(s)))}
      {@render row(s.hidden ? "Show in the list again" : "Hide from the list", s.hidden ? Eye : EyeOff, () => then(() => void app.toggleHidden(s)))}
      {#if s.webUrl}
        {@render row("Open in Google Chat", External, () => then(() => void api.openUrl(s.webUrl!)))}
      {/if}
    {/if}
  </Dialog.Content>
</Dialog.Root>
