<script lang="ts">
  import type { UserRef } from "$lib/api";
  import { initials } from "$lib/format";
  import { cn } from "$lib/utils";
  import PresenceDot from "./PresenceDot.svelte";

  /** `presence` adds a status dot (undefined: no dot slot at all); `ring` is the colour
   * behind it. */
  let {
    user,
    size = 32,
    class: className = "",
    presence,
    ring,
  }: {
    user: Pick<UserRef, "displayName" | "avatarUrl">;
    size?: number;
    class?: string;
    presence?: string | null;
    ring?: string;
  } = $props();
  let failed = $state(false);
</script>

{#snippet face()}
  <!-- Decorative: the name is always next to it. -->
  <span
    aria-hidden="true"
    class={cn(
      "inline-flex shrink-0 items-center justify-center overflow-hidden rounded-md bg-sidebar-active font-semibold text-foreground",
      className,
    )}
    style:width="{size}px"
    style:height="{size}px"
    style:font-size="{Math.round(size * 0.38)}px"
  >
    {#if user.avatarUrl && !failed}
      <img src={user.avatarUrl} alt="" class="h-full w-full object-cover" onerror={() => (failed = true)} draggable="false" />
    {:else}
      {initials(user.displayName)}
    {/if}
  </span>
{/snippet}

{#if presence === undefined}
  {@render face()}
{:else}
  <span class="relative inline-flex shrink-0">
    {@render face()}
    <PresenceDot
      state={presence}
      size={Math.max(8, Math.round(size * 0.36))}
      {ring}
      class="absolute -right-[3px] -bottom-[3px]"
    />
  </span>
{/if}
