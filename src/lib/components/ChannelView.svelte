<script lang="ts">
  import { api } from "$lib/api";
  import { app } from "$lib/state.svelte";
  import { cn } from "$lib/utils";
  import { presenceWords } from "$lib/presence";
  import Composer from "./Composer.svelte";
  import PresenceDot from "./PresenceDot.svelte";
  import MessageList from "./MessageList.svelte";
  import Star from "@lucide/svelte/icons/star";
  import BellOff from "@lucide/svelte/icons/bell-off";
  import Bell from "@lucide/svelte/icons/bell";
  import EyeOff from "@lucide/svelte/icons/eye-off";
  import Eye from "@lucide/svelte/icons/eye";
  import External from "@lucide/svelte/icons/external-link";
  import Users from "@lucide/svelte/icons/users";

  const s = $derived(app.currentSpace);
  const title = $derived(s?.displayName ?? "");
</script>

{#if s}
  <section class="flex min-w-0 flex-1 flex-col" aria-label={title}>
    <header data-tauri-drag-region class="flex h-14 shrink-0 items-center gap-2.5 border-b border-border pr-4 pl-6">
      {#if s.kind === "dm" && s.peer}
        {@const p = app.presence[s.peer]}
        <div class="min-w-0">
          <h1 class="m-0 min-w-0 truncate font-display text-[17px] leading-tight font-semibold">
            <button
              class="rounded-sm hover:underline hover:decoration-border hover:underline-offset-2"
              aria-label="{title}, show profile"
              onclick={(e) => (app.profileOf = { user: s.peer!, from: e.currentTarget })}>{title}</button
            >
          </h1>
          {#if presenceWords(p) || p?.custom}
            <p data-tauri-drag-region class="m-0 flex min-w-0 items-center gap-1.5 truncate text-[12px] text-muted-foreground">
              <PresenceDot state={p?.state} size={8} ring="transparent" />
              <span class="truncate"
                >{[presenceWords(p), p?.custom ? `${p.custom.emoji} ${p.custom.text}`.trim() : null].filter(Boolean).join(", ")}</span
              >
            </p>
          {/if}
        </div>
      {:else}
        <h1 data-tauri-drag-region class="min-w-0 truncate font-display text-[17px] font-semibold">{title}</h1>
      {/if}
      {#if s.kind !== "dm" && s.memberCount}
        <span class="flex items-center gap-1 text-[12px] text-muted-foreground" title="{s.memberCount} members">
          <Users class="size-3.5" aria-hidden="true" />
          <span class="tabular-nums">{s.memberCount}</span>
          <span class="sr-only">members</span>
        </span>
      {/if}
      <div class="ml-auto flex items-center gap-0.5">
        <button
          class={cn("hdr", s.starred && "text-foreground")}
          aria-pressed={s.starred}
          aria-label="Star"
          title={s.starred ? "Unstar" : "Star"}
          onclick={() => app.toggleStar(s)}
        >
          <Star class={cn(s.starred && "fill-highlight-strong stroke-foreground")} />
        </button>
        <button class="hdr" aria-pressed={s.muted} aria-label="Mute" title={s.muted ? "Unmute" : "Mute"} onclick={() => app.toggleMute(s)}>
          {#if s.muted}<BellOff />{:else}<Bell />{/if}
        </button>
        <button
          class="hdr"
          aria-label={s.hidden ? "Show in the sidebar again" : "Hide from the sidebar"}
          title={s.hidden ? "Unhide: back in the sidebar" : "Hide: moves it under Hidden at the bottom of the sidebar"}
          onclick={() => app.toggleHidden(s)}
        >
          {#if s.hidden}<Eye />{:else}<EyeOff />{/if}
        </button>
        {#if s.webUrl}
          <button class="hdr" aria-label="Open in Google Chat" title="Open in Google Chat" onclick={() => api.openUrl(s.webUrl!)}>
            <External />
          </button>
        {/if}
      </div>
    </header>

    <MessageList messages={app.currentMessages} key={s.name} newMarker={app.newMarker[s.name] ?? null} label="Messages in {title}" />
    <Composer where="channel" space={s.name} label="Message {title}" />
  </section>
{:else}
  <section class="flex flex-1 flex-col">
    <header data-tauri-drag-region class="h-14 shrink-0 border-b border-border"></header>
    <div class="flex flex-1 items-center justify-center p-8 text-center text-muted-foreground">
      {#if app.spaces.length === 0}
        <p>Your conversations appear here as soon as the first sync finishes.</p>
      {:else}
        <p>Pick a conversation on the left, or press <kbd class="kbd">⌘K</kbd> to jump to one.</p>
      {/if}
    </div>
  </section>
{/if}

<style>
  .hdr {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 30px;
    height: 30px;
    border-radius: 6px;
    color: var(--muted-foreground);
  }
  .hdr:hover {
    background: var(--muted);
    color: var(--foreground);
  }
  .hdr :global(svg) {
    width: 16px;
    height: 16px;
  }
  .kbd {
    font-family: var(--font-sans);
    border: 1px solid var(--input);
    border-radius: 4px;
    padding: 0 4px;
    font-size: 12px;
  }
</style>
