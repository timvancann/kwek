<script lang="ts">
  import * as Command from "$lib/components/ui/command/index.js";
  import { app } from "$lib/state.svelte";
  import Avatar from "./Avatar.svelte";
  import Hash from "@lucide/svelte/icons/hash";
  import UsersRound from "@lucide/svelte/icons/users-round";

  let open = $state(false);
  $effect(() => {
    open = app.overlay === "switcher";
  });
  $effect(() => {
    if (!open && app.overlay === "switcher") app.overlay = null;
  });

  function go(name: string) {
    app.overlay = null;
    void app.open(name).then(() => {
      app.zone = "composer";
      // The dialog returns focus to where it was opened from; take it back after that.
      setTimeout(() => app.focusNonce++, 0);
    });
  }
</script>

<Command.Dialog bind:open title="Jump to a conversation" description="Type a name to find a space or person">
  <Command.Input placeholder="Jump to a space or person" />
  <Command.List class="max-h-[50vh]">
    <Command.Empty>No conversation with that name.</Command.Empty>
    <Command.Group heading="Conversations">
      {#each app.spaces as s, idx (s.name)}
        <!-- value must be unique; the zero-width suffix keeps duplicate names apart without being searchable. -->
        <Command.Item value="{s.displayName}{'​'.repeat(idx + 1)}" onSelect={() => go(s.name)}>
          {#if s.kind === "dm"}
            <Avatar user={{ displayName: s.displayName, avatarUrl: s.avatarUrl }} size={18} />
          {:else if s.kind === "group"}
            <UsersRound class="text-muted-foreground" aria-hidden="true" />
          {:else}
            <Hash class="text-muted-foreground" aria-hidden="true" />
          {/if}
          <span class="flex-1 truncate" class:font-semibold={s.unread > 0 && !s.muted}>{s.displayName}</span>
          {#if !s.muted && (s.mentions > 0 || (s.kind !== "space" && s.unread > 0))}
            <span class="rounded-full bg-badge px-1.5 text-[11px] text-badge-foreground tabular-nums" aria-hidden="true">{s.mentions || s.unread}</span>
            <span class="sr-only">, {s.mentions ? `${s.mentions} mentions` : `${s.unread} unread`}</span>
          {/if}
        </Command.Item>
      {/each}
    </Command.Group>
  </Command.List>
</Command.Dialog>
