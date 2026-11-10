<script lang="ts">
  import { api, errorText, type MeetingView, type SpaceSummary } from "$lib/api";
  import { dayKey, meetingWhen } from "$lib/format";
  import { app, type SpaceSort } from "$lib/state.svelte";
  import { toast } from "svelte-sonner";
  import CalendarIcon from "@lucide/svelte/icons/calendar";
  import MapPin from "@lucide/svelte/icons/map-pin";
  import X from "@lucide/svelte/icons/x";
  import { cn } from "$lib/utils";
  import { tick } from "svelte";
  import Avatar from "./Avatar.svelte";
  import SyncIndicator from "./SyncIndicator.svelte";
  import StatusPicker from "./StatusPicker.svelte";
  import { presenceWords, stateWords } from "$lib/presence";
  import Hash from "@lucide/svelte/icons/hash";
  import UsersRound from "@lucide/svelte/icons/users-round";
  import BellOff from "@lucide/svelte/icons/bell-off";
  import ChevronDown from "@lucide/svelte/icons/chevron-down";
  import Search from "@lucide/svelte/icons/search";
  import LogOut from "@lucide/svelte/icons/log-out";
  import Eye from "@lucide/svelte/icons/eye";
  import EyeOff from "@lucide/svelte/icons/eye-off";

  let nav: HTMLElement | undefined = $state();
  /** You, at the bottom: the status picker opens from here. */
  let meRow: HTMLElement | undefined = $state();

  // ------------------------------------------------------------- next meeting

  /** Ticks so "in 4 min" stays true. */
  let now = $state(Date.now() * 1000);
  $effect(() => {
    const t = setInterval(() => (now = Date.now() * 1000), 20_000);
    return () => clearInterval(t);
  });
  /** The meeting running now or next, within the coming week. */
  const next = $derived(app.calendar.meetings.find((m) => m.endUs > now));
  /** How close it is, which sets how loud the sidebar is about it: a quiet line while it's
   * days away, a card today, Join from ten minutes before (as ⌘J), green while it runs. */
  const nearness = $derived.by(() => {
    if (!next) return null;
    if (next.startUs <= now) return "live";
    if (next.startUs - 10 * 60e6 <= now) return "soon";
    return dayKey(next.startUs) === dayKey(now) ? "today" : "later";
  });

  function when(m: MeetingView): string {
    return meetingWhen(m.startUs, m.endUs, now);
  }

  let connecting = $state(false);
  let promptDismissed = $state(readDismissed());
  function readDismissed(): boolean {
    try {
      return localStorage.getItem("kwek.calendarPromptDismissed") === "1";
    } catch {
      return false;
    }
  }
  function dismissPrompt() {
    promptDismissed = true;
    try {
      localStorage.setItem("kwek.calendarPromptDismissed", "1");
    } catch {
      // Private storage unavailable: hidden until restart.
    }
  }
  async function connectCalendar() {
    connecting = true;
    try {
      await api.connectCalendar();
    } catch (e) {
      toast.error(errorText(e));
    } finally {
      connecting = false;
    }
  }

  const SORT_LABEL: Record<SpaceSort, string> = { recent: "Recent", az: "A to Z", custom: "Custom" };
  const SORT_SPOKEN: Record<SpaceSort, string> = { recent: "by recent activity", az: "A to Z", custom: "in your own order" };
  const NEXT_SORT: Record<SpaceSort, SpaceSort> = { recent: "az", az: "custom", custom: "recent" };

  // ------------------------------------------------------------- reordering spaces

  /** The space being dragged, and where it would land. */
  let dragging: string | null = $state(null);
  let dropAt: { name: string; after: boolean } | null = $state(null);

  async function move(s: SpaceSummary, items: SpaceSummary[], to: number) {
    const at = await app.moveSpace(s.name, to);
    if (at === null) app.say(to < 0 ? `${s.displayName} is already at the top.` : to >= items.length ? `${s.displayName} is already at the bottom.` : "");
    else app.say(`${s.displayName} moved to ${at + 1} of ${items.length}.`);
    await tick();
    nav?.querySelector<HTMLElement>(`[data-space="${CSS.escape(s.name)}"]`)?.focus();
  }

  /** ⌥↑/⌥↓ on a space moves it, like dragging. */
  function onRowKeydown(e: KeyboardEvent, s: SpaceSummary, sectionId: string, items: SpaceSummary[]) {
    if (sectionId !== "spaces" || !e.altKey || (e.key !== "ArrowUp" && e.key !== "ArrowDown")) return;
    e.preventDefault();
    e.stopPropagation();
    const i = items.findIndex((x) => x.name === s.name);
    void move(s, items, i + (e.key === "ArrowDown" ? 1 : -1));
  }

  function onDragOver(e: DragEvent, s: SpaceSummary) {
    if (!dragging) return;
    e.preventDefault();
    const r = (e.currentTarget as HTMLElement).getBoundingClientRect();
    dropAt = { name: s.name, after: e.clientY > r.top + r.height / 2 };
  }

  function onDrop(e: DragEvent, items: SpaceSummary[]) {
    e.preventDefault();
    const from = items.findIndex((x) => x.name === dragging);
    const over = items.findIndex((x) => x.name === dropAt?.name);
    if (from >= 0 && over >= 0 && dropAt) {
      let to = over + (dropAt.after ? 1 : 0);
      if (from < to) to -= 1;
      void move(items[from], items, to);
    }
    dragging = null;
    dropAt = null;
  }

  /** Highlighted row in sidebar zone (j/k), else the open conversation. */
  const focusName = $derived(app.zone === "sidebar" ? (app.sidebarFocus ?? app.current) : app.current);
  /** The row Tab lands on: the highlighted one when it is shown, else the first (it may sit
   * in the collapsed Hidden section). */
  const tabbable = $derived(app.sidebarOrder.some((s) => s.name === focusName) ? focusName : app.sidebarOrder[0]?.name);

  // Focus follows the keyboard highlight while the sidebar is the active zone.
  $effect(() => {
    const name = focusName;
    if (app.zone !== "sidebar" || !nav) return;
    const el = nav.querySelector<HTMLElement>(`[data-space="${CSS.escape(name ?? "")}"]`) ?? nav.querySelector<HTMLElement>("[data-space]");
    el?.focus();
    el?.scrollIntoView({ block: "nearest" });
  });

  function badge(s: SpaceSummary): { n: number; mention: boolean } | null {
    if (s.muted) return null;
    if (s.mentions > 0) return { n: s.mentions, mention: true };
    if (s.kind !== "space" && s.unread > 0) return { n: s.unread, mention: false };
    return null;
  }

  function unreadLabel(s: SpaceSummary): string {
    if (s.muted || (s.unread === 0 && s.mentions === 0)) return "";
    const parts = [`${s.unread} unread`];
    if (s.mentions > 0) parts.push(`${s.mentions} ${s.mentions === 1 ? "mention" : "mentions"}`);
    return parts.join(", ");
  }

  function sectionTotal(items: SpaceSummary[]): number {
    return items.filter((s) => !s.muted).reduce((n, s) => n + (s.kind === "space" ? s.mentions : s.unread), 0);
  }

  const me = $derived(app.auth.state === "signedIn" ? app.auth : null);
</script>

<nav bind:this={nav} class="flex w-[268px] shrink-0 flex-col bg-sidebar text-sidebar-foreground" aria-label="Conversations">
  <!-- Room for the window controls; draggable like a title bar. -->
  <div data-tauri-drag-region class="h-12 shrink-0"></div>

  <div class="flex gap-1.5 px-3 pb-3">
    <button
      class="flex h-9 min-w-0 flex-1 items-center gap-2 rounded-lg border border-border bg-background px-3 text-left text-muted-foreground hover:border-input"
      onclick={() => (app.overlay = "switcher")}
    >
      <span class="flex-1">Jump to</span>
      <kbd class="font-sans text-[11px]" aria-label="Command K">⌘K</kbd>
    </button>
    <button
      class="inline-flex size-9 shrink-0 items-center justify-center rounded-lg border border-border bg-background text-muted-foreground hover:border-input hover:text-foreground"
      aria-label="Search messages"
      aria-keyshortcuts="Meta+F"
      title="Search messages (⌘F)"
      onclick={() => (app.overlay = "search")}
    >
      <Search class="size-4" aria-hidden="true" />
    </button>
    {#if app.calendar.state === "ok"}
      <button
        class="inline-flex size-9 shrink-0 items-center justify-center rounded-lg border border-border bg-background text-muted-foreground hover:border-input hover:text-foreground"
        aria-label="Upcoming meetings"
        aria-keyshortcuts="Meta+Shift+J"
        title="Upcoming meetings (⌘⇧J)"
        onclick={() => (app.overlay = "agenda")}
      >
        <CalendarIcon class="size-4" aria-hidden="true" />
      </button>
    {/if}
  </div>

  <div class="min-h-0 flex-1 overflow-y-auto px-3 pb-4">
    {#each app.sections as section (section.id)}
      {@const total = sectionTotal(section.items)}
      <div class="mt-5 first:mt-1">
        <div class="flex items-center gap-1">
          <!-- The heading wraps the toggle so heading navigation still finds it. -->
          <h2 class="m-0 min-w-0 flex-1 text-[12px] font-semibold">
            <button
              class="flex h-6 w-full items-center gap-1 rounded-sm px-2 text-left text-muted-foreground hover:text-foreground"
              aria-expanded={!app.collapsed[section.id]}
              onclick={() => (app.collapsed[section.id] = !app.collapsed[section.id])}
            >
              <ChevronDown class={cn("size-3 shrink-0 transition-transform", app.collapsed[section.id] && "-rotate-90")} aria-hidden="true" />
              <span class="truncate">{section.title}</span>
              {#if app.collapsed[section.id] && total > 0}
                <span class="ml-auto rounded-full bg-badge px-1.5 text-[11px] leading-[18px] font-semibold text-badge-foreground tabular-nums"
                  >{total}<span class="sr-only"> unread</span></span
                >
              {/if}
            </button>
          </h2>
          {#if section.id === "spaces" && !app.collapsed[section.id]}
            <button
              class="h-6 shrink-0 rounded-sm px-2 text-[11px] text-muted-foreground hover:bg-sidebar-hover hover:text-foreground"
              aria-label="Sorted {SORT_SPOKEN[app.spaceSort]}. Sort {SORT_SPOKEN[NEXT_SORT[app.spaceSort]]}"
              title={app.spaceSort === "custom" ? "Your own order: drag spaces, or Option+Up/Down" : "Drag a space to put them in your own order"}
              onclick={() => app.setSpaceSort(NEXT_SORT[app.spaceSort], true)}>{SORT_LABEL[app.spaceSort]}</button
            >
          {/if}
        </div>
        {#if !app.collapsed[section.id]}
          <!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
          <ul
            class="m-0 mt-0.5 list-none p-0"
            ondragleave={(e) => {
              if (!e.currentTarget.contains(e.relatedTarget as Node)) dropAt = null;
            }}
          >
            {#each section.items as s (s.name)}
              {@const active = s.name === app.current}
              {@const b = badge(s)}
              {@const movable = section.id === "spaces"}
              <!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
              <li
                class="group relative"
                draggable={movable}
                ondragstart={(e) => {
                  if (!movable || !e.dataTransfer) return;
                  dragging = s.name;
                  e.dataTransfer.setData("text/x-kwek-space", s.name);
                  e.dataTransfer.effectAllowed = "move";
                }}
                ondragover={(e) => movable && onDragOver(e, s)}
                ondrop={(e) => movable && onDrop(e, section.items)}
                ondragend={() => {
                  dragging = null;
                  dropAt = null;
                }}
              >
                {#if dropAt?.name === s.name && dragging !== s.name}
                  <span
                    class={cn("pointer-events-none absolute inset-x-2 z-10 h-0.5 rounded-full bg-accent", dropAt.after ? "-bottom-px" : "-top-px")}
                    aria-hidden="true"
                  ></span>
                {/if}
                <button
                  data-space={s.name}
                  aria-current={active ? "page" : undefined}
                  aria-keyshortcuts={movable ? "Alt+ArrowUp Alt+ArrowDown" : undefined}
                  tabindex={s.name === tabbable ? 0 : -1}
                  onkeydown={(e) => onRowKeydown(e, s, section.id, section.items)}
                  onclick={() => void app.open(s.name)}
                  onfocus={() => {
                    app.zone = "sidebar";
                    app.sidebarFocus = s.name;
                  }}
                  class={cn(
                    "flex h-8 w-full items-center gap-2.5 rounded-md px-2.5 text-left text-[14.5px] outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-inset",
                    // The whole row, so it stays highlighted while the pointer is on its hide button.
                    active ? "bg-sidebar-active font-semibold" : "group-hover:bg-sidebar-hover",
                    s.muted && !active && "text-muted-foreground",
                    !s.muted && s.unread > 0 && "font-semibold",
                    dragging === s.name && "opacity-50",
                  )}
                >
                  {#if s.kind === "dm"}
                    <Avatar
                      user={{ displayName: s.displayName, avatarUrl: s.avatarUrl }}
                      size={20}
                      presence={s.peer ? (app.presence[s.peer]?.state ?? null) : undefined}
                      ring={active ? "var(--sidebar-active)" : "var(--sidebar)"}
                    />
                  {:else if s.kind === "group"}
                    <UsersRound class="size-4 shrink-0 text-muted-foreground" aria-hidden="true" />
                  {:else}
                    <Hash class="size-4 shrink-0 text-muted-foreground" aria-hidden="true" />
                  {/if}
                  <span class="min-w-0 flex-1 truncate">{s.displayName}</span>
                  {#if s.peer}
                    {@const p = app.presence[s.peer]}
                    {#if p?.custom?.emoji}<span class="shrink-0 text-[13px]" title={p.custom.text} aria-hidden="true">{p.custom.emoji}</span>{/if}
                    {#if presenceWords(p)}<span class="sr-only">, {presenceWords(p)}{p?.custom ? `, ${p.custom.text}` : ""}</span>{/if}
                  {/if}
                  {#if s.muted}
                    <BellOff class="size-3.5 shrink-0" aria-hidden="true" />
                    <span class="sr-only">, muted</span>
                  {:else if b}
                    <span
                      aria-hidden="true"
                      class={cn(
                        "min-w-5 rounded-full px-1.5 text-center text-[11px] leading-[18px] font-semibold tabular-nums",
                        b.mention ? "bg-highlight-strong text-foreground" : "bg-badge text-badge-foreground",
                      )}>{b.mention ? `@${b.n}` : b.n}</span
                    >
                  {/if}
                  {#if unreadLabel(s)}<span class="sr-only">, {unreadLabel(s)}</span>{/if}
                </button>
                <!-- Hide or show on hover, a mouse shortcut over the badge. Out of reach until
                     the row is hovered, and out of the accessibility tree: the conversation
                     header's Hide/Unhide is the way for keyboard and screen readers. -->
                <button
                  class={cn(
                    "pointer-events-none absolute top-1 right-1 grid size-6 place-items-center rounded-md text-muted-foreground opacity-0 group-hover:pointer-events-auto group-hover:opacity-100 hover:text-foreground",
                    active ? "bg-sidebar-active" : "bg-sidebar-hover",
                  )}
                  tabindex="-1"
                  aria-hidden="true"
                  aria-label={s.hidden ? `Show ${s.displayName} in the sidebar again` : `Hide ${s.displayName}`}
                  title={s.hidden ? "Show in the sidebar again" : "Hide: moves it under Hidden"}
                  onclick={() => void app.toggleHidden(s)}
                >
                  {#if s.hidden}<Eye class="size-3.5" aria-hidden="true" />{:else}<EyeOff class="size-3.5" aria-hidden="true" />{/if}
                </button>
              </li>
            {/each}
          </ul>
        {/if}
      </div>
    {/each}
  </div>

  {#if next && nearness === "later"}
    <section aria-label="Next meeting" class="mx-3 mb-2">
      <button
        class="flex w-full items-center gap-2 rounded-md px-1.5 py-1 text-left text-[12px] text-muted-foreground hover:bg-sidebar-hover"
        title="Upcoming meetings (⌘⇧J)"
        aria-keyshortcuts="Meta+Shift+J"
        onclick={() => (app.overlay = "agenda")}
      >
        <CalendarIcon class="size-3.5 shrink-0" aria-hidden="true" />
        <span class="inline-block shrink-0 first-letter:uppercase">{when(next)}</span>
        <span class="min-w-0 truncate font-medium text-foreground">{next.title}</span>
      </button>
    </section>
  {:else if next}
    {@const place = next.joinUrl ? null : next.location}
    <section
      aria-label="Next meeting"
      class={cn(
        "mx-3 mb-2 flex items-center gap-2 rounded-lg border py-1.5 pr-1.5 pl-3",
        nearness === "live" ? "border-accent/30 bg-accent-soft" : "border-border bg-background",
      )}
    >
      {#if place}
        <MapPin class="size-4 shrink-0 text-muted-foreground" aria-hidden="true" />
      {:else}
        <CalendarIcon class="size-4 shrink-0 text-muted-foreground" aria-hidden="true" />
      {/if}
      <button
        class="min-w-0 flex-1 text-left"
        title="Upcoming meetings (⌘⇧J)"
        aria-keyshortcuts="Meta+Shift+J"
        onclick={() => (app.overlay = "agenda")}
      >
        <span class="block truncate text-[13px] font-medium">{next.title}</span>
        <span class="block truncate text-[12px] text-muted-foreground" title={place ?? undefined}
          ><span class={cn(nearness === "live" && "font-medium text-accent")}>{when(next)}</span>{#if place}, {place}{/if}</span
        >
      </button>
      {#if next.joinUrl && (nearness === "live" || nearness === "soon")}
        <button
          class="shrink-0 rounded-md bg-primary px-2.5 py-1 text-[12px] font-medium text-primary-foreground hover:bg-primary/85"
          aria-label="Join {next.title}"
          aria-keyshortcuts="Meta+J"
          title="Join in your browser (⌘J)"
          onclick={() => next?.joinUrl && void api.openUrl(next.joinUrl)}>Join</button
        >
      {/if}
    </section>
  {:else if app.calendar.state === "ok"}
    <button
      class="mx-3 mb-2 flex items-center gap-2 rounded-lg border border-border px-3 py-2 text-left text-[12px] text-muted-foreground hover:bg-sidebar-hover hover:text-foreground"
      onclick={() => (app.overlay = "agenda")}
    >
      <CalendarIcon class="size-4 shrink-0" aria-hidden="true" />
      No meetings in the coming week
    </button>
  {:else if app.calendar.state === "needsPermission" && !promptDismissed}
    <div class="mx-3 mb-2 flex items-center gap-2 rounded-lg border border-dashed border-input py-1.5 pr-1 pl-3 text-[12px] text-muted-foreground">
      <CalendarIcon class="size-4 shrink-0" aria-hidden="true" />
      <button class="min-w-0 flex-1 text-left hover:text-foreground" onclick={connectCalendar} disabled={connecting}>
        {connecting ? "Finish in the browser, then come back" : "Connect your calendar for meeting reminders"}
      </button>
      <button
        class="grid size-6 shrink-0 place-items-center rounded-md hover:bg-sidebar-hover hover:text-foreground"
        aria-label="Not now"
        title="Not now"
        onclick={dismissPrompt}
      >
        <X class="size-3.5" aria-hidden="true" />
      </button>
    </div>
  {/if}

  {#if me && app.ownPresence.access !== "off"}
    {@const own = app.ownPresence}
    <button
      bind:this={meRow}
      class={cn(
        "mx-2 mb-1 flex items-center gap-2.5 rounded-md px-2 py-1.5 text-left hover:bg-sidebar-hover",
        app.statusPicker && "bg-sidebar-hover",
      )}
      aria-haspopup="dialog"
      aria-expanded={app.statusPicker}
      aria-label="Your status: {own.state ? stateWords(own.state, own.dndUntilUs) : 'unknown'}{own.custom ? `, ${own.custom.text}` : ''}. Change"
      onclick={() => (app.statusPicker = !app.statusPicker)}
    >
      <Avatar user={{ displayName: me.name ?? me.email ?? "You", avatarUrl: null }} size={24} presence={own.state} ring="var(--sidebar)" />
      <span class="min-w-0 flex-1">
        <span class="block truncate text-[13px] font-semibold">{me.name ?? me.email ?? "You"}</span>
        <span class="block truncate text-[12px] text-muted-foreground">
          {#if own.access === "needsPermission"}Set a status
          {:else if own.custom}{own.custom.emoji} {own.custom.text}
          {:else if own.state}{stateWords(own.state, own.dndUntilUs)}
          {:else}Set a status{/if}
        </span>
      </span>
    </button>
    <StatusPicker anchor={meRow} />
  {/if}

  <div class="flex items-center gap-1 border-t border-border px-2 py-1.5">
    <SyncIndicator />
    <button
      class="ml-auto inline-flex size-7 shrink-0 items-center justify-center rounded-md text-muted-foreground hover:bg-sidebar-hover hover:text-foreground"
      aria-label={me?.email ? `Sign out ${me.email}` : "Sign out"}
      title={me?.email ? `Signed in as ${me.email}. Sign out` : "Sign out"}
      onclick={() => (app.confirmSignOut = true)}
    >
      <LogOut class="size-4" />
    </button>
  </div>
</nav>
