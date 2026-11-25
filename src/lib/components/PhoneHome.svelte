<script lang="ts">
  import { app } from "$lib/state.svelte";
  import { stateWords } from "$lib/presence";
  import { cn } from "$lib/utils";
  import Sidebar from "./Sidebar.svelte";
  import ActivityPanel from "./ActivityPanel.svelte";
  import Avatar from "./Avatar.svelte";
  import StatusPicker from "./StatusPicker.svelte";
  import SyncIndicator from "./SyncIndicator.svelte";
  import House from "@lucide/svelte/icons/house";
  import MessagesSquare from "@lucide/svelte/icons/messages-square";
  import Inbox from "@lucide/svelte/icons/inbox";
  import CircleUser from "@lucide/svelte/icons/circle-user";
  import CalendarIcon from "@lucide/svelte/icons/calendar";
  import LogOut from "@lucide/svelte/icons/log-out";
  import ChevronRight from "@lucide/svelte/icons/chevron-right";

  // A phone's main screen: one tab at a time and the tab bar, as in Slack's app.

  const me = $derived(app.auth.state === "signedIn" ? app.auth : null);
  /** Mentions in spaces, and unread direct messages: the tab bar's badges. */
  const homeBadge = $derived(app.spaces.filter((s) => s.kind === "space" && !s.muted && !s.hidden).reduce((n, s) => n + s.mentions, 0));
  const dmBadge = $derived(app.spaces.filter((s) => s.kind !== "space" && !s.muted && !s.hidden && s.unread > 0).length);

  let statusRow: HTMLElement | undefined = $state();

  const TABS = [
    { id: "home", label: "Home", Icon: House },
    { id: "dms", label: "DMs", Icon: MessagesSquare },
    { id: "activity", label: "Activity", Icon: Inbox },
    { id: "you", label: "You", Icon: CircleUser },
  ] as const;
</script>

<div class="flex h-full flex-col bg-sidebar">
  <div class="flex min-h-0 flex-1 flex-col">
    {#if app.tab === "home"}
      <Sidebar phone only="home" />
    {:else if app.tab === "dms"}
      <Sidebar phone only="dms" />
    {:else if app.tab === "activity"}
      <div class="flex min-h-0 flex-1 flex-col bg-background pt-[var(--safe-top)]">
        <ActivityPanel active>
          {#snippet title()}
            <h1 class="m-0 font-display text-[24px] font-bold">Activity</h1>
          {/snippet}
        </ActivityPanel>
      </div>
    {:else}
      {@const own = app.ownPresence}
      <div class="flex min-h-0 flex-1 flex-col overflow-y-auto px-4 pt-[calc(var(--safe-top)+12px)]">
        <h1 class="m-0 mb-4 font-display text-[24px] font-bold">You</h1>
        {#if me}
          <div class="mb-4 flex items-center gap-3">
            <Avatar user={{ displayName: me.name ?? me.email ?? "You", avatarUrl: null }} size={48} presence={own.state} ring="var(--sidebar)" />
            <div class="min-w-0">
              <p class="m-0 truncate text-[17px] font-semibold">{me.name ?? me.email ?? "You"}</p>
              {#if me.email && me.name}<p class="m-0 truncate text-[13px] text-muted-foreground">{me.email}</p>{/if}
            </div>
          </div>
        {/if}
        <div class="flex flex-col overflow-hidden rounded-xl border border-border bg-background">
          {#if app.ownPresence.access !== "off"}
            <button
              bind:this={statusRow}
              class="flex min-h-14 items-center gap-3 px-4 text-left active:bg-muted"
              aria-haspopup="dialog"
              onclick={() => (app.statusPicker = !app.statusPicker)}
            >
              <span class="min-w-0 flex-1">
                <span class="block text-[16px]">Status</span>
                <span class="block truncate text-[13px] text-muted-foreground">
                  {#if own.custom}{own.custom.emoji} {own.custom.text}{:else if own.state}{stateWords(own.state, own.dndUntilUs)}{:else}Set a status{/if}
                </span>
              </span>
              <ChevronRight class="size-5 text-muted-foreground" aria-hidden="true" />
            </button>
            <StatusPicker anchor={statusRow} />
          {/if}
          {#if app.calendar.state === "ok"}
            <button class="flex min-h-14 items-center gap-3 border-t border-border px-4 text-left text-[16px] active:bg-muted" onclick={() => (app.overlay = "agenda")}>
              <CalendarIcon class="size-5 text-muted-foreground" aria-hidden="true" />
              <span class="flex-1">Upcoming meetings</span>
              <ChevronRight class="size-5 text-muted-foreground" aria-hidden="true" />
            </button>
          {/if}
          <button
            class="flex min-h-14 items-center gap-3 border-t border-border px-4 text-left text-[16px] text-destructive active:bg-muted"
            onclick={() => (app.confirmSignOut = true)}
          >
            <LogOut class="size-5" aria-hidden="true" />
            Sign out
          </button>
        </div>
        <div class="mt-4 px-1"><SyncIndicator /></div>
      </div>
    {/if}
  </div>

  <nav class="flex shrink-0 border-t border-border bg-background pb-[var(--safe-bottom)]" aria-label="Tabs">
    {#each TABS as t (t.id)}
      {@const badge = t.id === "home" ? homeBadge : t.id === "dms" ? dmBadge : 0}
      <button
        class={cn("relative flex h-16 flex-1 flex-col items-center justify-center gap-1 text-[11px]", app.tab === t.id ? "font-semibold text-foreground" : "text-muted-foreground")}
        aria-current={app.tab === t.id ? "page" : undefined}
        onclick={() => (app.tab = t.id)}
      >
        <t.Icon class={cn("size-6", app.tab === t.id && "stroke-[2.25]")} aria-hidden="true" />
        {t.label}
        {#if badge > 0}
          <span
            class="absolute top-2 left-1/2 ml-2 min-w-[18px] rounded-full bg-highlight-strong px-1 text-center text-[11px] leading-[18px] font-semibold text-foreground tabular-nums"
            >{badge}<span class="sr-only"> unread</span></span
          >
        {/if}
      </button>
    {/each}
  </nav>
</div>
