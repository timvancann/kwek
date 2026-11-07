<script lang="ts">
  import type { UserRef } from "$lib/api";
  import { initials } from "$lib/format";
  import { cn } from "$lib/utils";

  let { user, size = 32, class: className = "" }: { user: Pick<UserRef, "displayName" | "avatarUrl">; size?: number; class?: string } =
    $props();
  let failed = $state(false);
</script>

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
