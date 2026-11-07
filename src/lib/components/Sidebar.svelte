<script lang="ts">
  import type { SpaceSummary } from "$lib/api";
  import { app } from "$lib/state.svelte";
  import { cn } from "$lib/utils";
  import Avatar from "./Avatar.svelte";
  import SyncIndicator from "./SyncIndicator.svelte";
  import Hash from "@lucide/svelte/icons/hash";
  import UsersRound from "@lucide/svelte/icons/users-round";
  import BellOff from "@lucide/svelte/icons/bell-off";
  import ChevronDown from "@lucide/svelte/icons/chevron-down";
  import Search from "@lucide/svelte/icons/search";
  import LogOut from "@lucide/svelte/icons/log-out";

  let nav: HTMLElement | undefined = $state();

  /** Highlighted row in sidebar zone (j/k), else the open conversation. */
  const focusName = $derived(app.zone === "sidebar" ? (app.sidebarOrder[app.sidebarIndex]?.name ?? app.current) : app.current);

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

<nav bind:this={nav} class="flex w-[248px] shrink-0 flex-col bg-sidebar text-sidebar-foreground" aria-label="Conversations">
  <!-- Room for the window controls; draggable like a title bar. -->
  <div data-tauri-drag-region class="h-11 shrink-0"></div>

  <div class="px-3 pb-2">
    <button
      class="flex h-8 w-full items-center gap-2 rounded-md border border-border bg-background px-2 text-left text-muted-foreground hover:border-input"
      onclick={() => (app.overlay = "switcher")}
    >
      <Search class="size-3.5" aria-hidden="true" />
      <span class="flex-1">Jump to</span>
      <kbd class="font-sans text-[11px]" aria-label="Command K">⌘K</kbd>
    </button>
  </div>

  <div class="min-h-0 flex-1 overflow-y-auto px-2 pb-3">
    {#each app.sections as section (section.id)}
      {@const total = sectionTotal(section.items)}
      <div class="mt-3 first:mt-1">
        <div class="flex items-center gap-1">
          <button
            class="flex h-6 min-w-0 flex-1 items-center gap-1 rounded-sm px-2 text-left text-muted-foreground hover:text-foreground"
            aria-expanded={!app.collapsed[section.id]}
            onclick={() => (app.collapsed[section.id] = !app.collapsed[section.id])}
          >
            <ChevronDown class={cn("size-3 shrink-0 transition-transform", app.collapsed[section.id] && "-rotate-90")} aria-hidden="true" />
            <h2 class="m-0 truncate text-[12px] font-semibold">{section.title}</h2>
            {#if app.collapsed[section.id] && total > 0}
              <span class="ml-auto rounded-full bg-badge px-1.5 text-[11px] leading-[18px] font-semibold text-badge-foreground tabular-nums"
                >{total}<span class="sr-only"> unread</span></span
              >
            {/if}
          </button>
          {#if section.id === "spaces" && !app.collapsed[section.id]}
            <button
              class="h-6 shrink-0 rounded-sm px-2 text-[11px] text-muted-foreground hover:bg-sidebar-hover hover:text-foreground"
              aria-label={app.alphabetical ? "Sorted A to Z. Sort by recent activity" : "Sorted by recent activity. Sort A to Z"}
              onclick={() => (app.alphabetical = !app.alphabetical)}>{app.alphabetical ? "A to Z" : "Recent"}</button
            >
          {/if}
        </div>
        {#if !app.collapsed[section.id]}
          <ul class="m-0 mt-0.5 list-none p-0">
            {#each section.items as s (s.name)}
              {@const active = s.name === app.current}
              {@const b = badge(s)}
              <li>
                <button
                  data-space={s.name}
                  aria-current={active ? "page" : undefined}
                  tabindex={s.name === focusName ? 0 : -1}
                  onclick={() => void app.open(s.name)}
                  onfocus={() => {
                    app.zone = "sidebar";
                    app.sidebarIndex = Math.max(0, app.sidebarOrder.findIndex((x) => x.name === s.name));
                  }}
                  class={cn(
                    "flex h-7 w-full items-center gap-2 rounded-md px-2 text-left outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-inset",
                    active ? "bg-sidebar-active font-semibold" : "hover:bg-sidebar-hover",
                    s.muted && !active && "text-muted-foreground",
                    !s.muted && s.unread > 0 && "font-semibold",
                  )}
                >
                  {#if s.kind === "dm"}
                    <Avatar user={{ displayName: s.displayName, avatarUrl: s.avatarUrl }} size={18} />
                  {:else if s.kind === "group"}
                    <UsersRound class="size-4 shrink-0 text-muted-foreground" aria-hidden="true" />
                  {:else}
                    <Hash class="size-4 shrink-0 text-muted-foreground" aria-hidden="true" />
                  {/if}
                  <span class="min-w-0 flex-1 truncate">{s.displayName}</span>
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
              </li>
            {/each}
          </ul>
        {/if}
      </div>
    {/each}
  </div>

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
