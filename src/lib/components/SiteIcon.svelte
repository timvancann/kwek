<script lang="ts" module>
  /** Hosts without an icon this session, so a missing one isn't asked for on every render. */
  const missing = new Set<string>();
</script>

<script lang="ts">
  import Link from "@lucide/svelte/icons/link";

  /** A site's icon (fetched and cached by Rust), or a plain link mark when it has none. */
  let { host }: { host: string } = $props();
  /** The host whose icon just failed to load here. */
  let failedFor = $state<string | null>(null);
  const failed = $derived(failedFor === host || missing.has(host));
</script>

{#if host && !failed}
  <img
    src="media://localhost/favicon/{encodeURIComponent(host)}"
    alt=""
    aria-hidden="true"
    class="mr-1 inline-block size-[1.05em] rounded-[3px] align-[-0.18em]"
    draggable="false"
    onerror={() => {
      missing.add(host);
      failedFor = host;
    }}
  />
{:else}
  <Link class="mr-0.5 inline-block size-[0.95em] align-[-0.12em]" aria-hidden="true" />
{/if}
