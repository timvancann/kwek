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

  let collapsed: Record<string, boolean> = $state({});
  let alphabetical = $state(false);
  let nav: HTMLElement | undefined = $state();

  function sorted(items: SpaceSummary[]) {
    return alphabetical ? [...items].sort((a, b) => a.displayName.localeCompare(b.displayName)) : items;
  }

  // Focus follows the keyboard zone.
  $effect(() => {
    if (app.zone !== "sidebar" || !nav) return;
    const el = nav.querySelector<HTMLElement>(`[data-space="${CSS.escape(app.current ?? "")}"]`) ?? nav.querySelector<HTMLElement>("[data-space]");
    el?.focus();
  });

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
      <kbd class="text-[11px]">⌘K</kbd>
    </button>
  </div>

  <div class="min-h-0 flex-1 overflow-y-auto px-2 pb-3">
    {#each app.sections as section (section.id)}
      <div class="mt-3 first:mt-1">
        <div class="flex items-center pr-1">
          <button
            class="flex flex-1 items-center gap-1 rounded-sm px-2 py-0.5 text-left text-[12px] font-semibold text-muted-foreground hover:text-foreground"
            aria-expanded={!collapsed[section.id]}
            onclick={() => (collapsed[section.id] = !collapsed[section.id])}
          >
            <ChevronDown class={cn("size-3 transition-transform", collapsed[section.id] && "-rotate-90")} aria-hidden="true" />
            <h2 class="m-0 text-[12px] font-semibold">{section.title}</h2>
          </button>
          {#if section.id === "spaces"}
            <button
              class="rounded-sm px-1.5 text-[11px] text-muted-foreground hover:text-foreground"
              aria-pressed={alphabetical}
              title={alphabetical ? "Sort by recent activity" : "Sort alphabetically"}
              onclick={() => (alphabetical = !alphabetical)}>{alphabetical ? "A to Z" : "Recent"}</button
            >
          {/if}
        </div>
        {#if !collapsed[section.id]}
          <ul class="m-0 mt-0.5 list-none p-0">
            {#each sorted(section.items) as s (s.name)}
              {@const active = s.name === app.current}
              {@const loud = !s.muted && (s.mentions > 0 || (s.kind !== "space" && s.unread > 0))}
              <li>
                <button
                  data-space={s.name}
                  aria-current={active ? "page" : undefined}
                  tabindex={active ? 0 : -1}
                  onclick={() => void app.open(s.name)}
                  onfocus={() => (app.zone = "sidebar")}
                  class={cn(
                    "flex h-7 w-full items-center gap-2 rounded-md px-2 text-left",
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
                    <BellOff class="size-3.5 shrink-0" aria-label="Muted" />
                  {:else if loud}
                    <span class="min-w-5 rounded-full bg-badge px-1.5 text-center text-[11px] leading-[18px] font-semibold text-badge-foreground tabular-nums"
                      >{s.mentions > 0 ? s.mentions : s.unread}</span
                    >
                  {/if}
                  {#if !s.muted && (s.unread > 0 || s.mentions > 0)}
                    <span class="sr-only"
                      >{s.unread} unread{s.mentions > 0 ? `, ${s.mentions} ${s.mentions === 1 ? "mention" : "mentions"}` : ""}</span
                    >
                  {/if}
                </button>
              </li>
            {/each}
          </ul>
        {/if}
      </div>
    {/each}
  </div>

  <div class="flex items-center gap-2 border-t border-border px-3 py-2">
    <SyncIndicator />
    <span class="min-w-0 flex-1 truncate text-[12px] text-muted-foreground" title={me?.email ?? ""}>{me?.name ?? me?.email ?? ""}</span>
    <button
      class="inline-flex size-7 items-center justify-center rounded-md text-muted-foreground hover:bg-sidebar-hover hover:text-foreground"
      aria-label="Sign out"
      title="Sign out"
      onclick={() => (app.confirmSignOut = true)}
    >
      <LogOut class="size-4" />
    </button>
  </div>
</nav>
