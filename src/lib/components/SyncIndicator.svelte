<script lang="ts">
  import { app } from "$lib/state.svelte";
  import { api } from "$lib/api";
  import { cn } from "$lib/utils";

  const s = $derived(app.status);
  const view = $derived.by(() => {
    switch (s.state) {
      case "live":
        return { dot: "bg-accent", text: "", title: "Up to date. New messages arrive live." };
      case "bootstrapping":
        return { dot: "bg-muted-foreground animate-pulse", text: s.detail ?? "Syncing", title: s.detail ?? "First sync in progress" };
      case "reconciling":
        return { dot: "bg-muted-foreground animate-pulse", text: "", title: "Catching up on missed messages" };
      case "polling":
        return {
          dot: "bg-muted-foreground",
          text: "Checking every minute",
          title: "Live updates are not available, so kwek checks for new messages every minute.",
        };
      case "offline":
        return { dot: "bg-highlight-strong", text: "Offline", title: "No connection. kwek shows cached messages and retries." };
      case "authRequired":
        return { dot: "bg-destructive", text: "Signed out", title: "Sign in to Google again." };
      case "error":
        return { dot: "bg-destructive", text: "Sync problem", title: `${s.detail ?? "Sync stopped"}. Click to retry.` };
      default:
        return { dot: "bg-muted-foreground", text: "Starting", title: "Starting" };
    }
  });
</script>

<button
  class="flex min-w-0 shrink-0 items-center gap-1.5 rounded-sm text-[12px] text-muted-foreground"
  title={view.title}
  aria-label="Sync status: {view.title}"
  onclick={() => api.refresh()}
>
  <span class={cn("size-2 shrink-0 rounded-full", view.dot)} aria-hidden="true"></span>
  {#if view.text}<span class="truncate">{view.text}</span>{/if}
</button>
