<script lang="ts">
  import { app } from "$lib/state.svelte";
  import { api } from "$lib/api";
  import { cn } from "$lib/utils";

  const s = $derived(app.status);
  // `text` is always visible, so the button's accessible name starts with it (WCAG 2.5.3).
  const view = $derived.by(() => {
    switch (s.state) {
      case "live":
        return { dot: "bg-success", text: "Live", detail: "New messages arrive as they are sent." };
      case "bootstrapping":
        return { dot: "bg-muted-foreground animate-pulse", text: s.detail ?? "Syncing", detail: "First sync in progress." };
      case "reconciling":
        return { dot: "bg-muted-foreground animate-pulse", text: "Catching up", detail: "Fetching messages missed while away." };
      case "polling":
        return {
          dot: "bg-muted-foreground",
          text: "Checking every minute",
          detail: "Live updates are not available, so kwek checks for new messages every minute.",
        };
      case "offline":
        return { dot: "bg-highlight-strong", text: "Offline", detail: "No connection. Showing cached messages; kwek retries." };
      case "authRequired":
        return { dot: "bg-destructive", text: "Signed out", detail: "Sign in to Google again." };
      case "error":
        return { dot: "bg-destructive", text: "Sync problem", detail: `${s.detail ?? "Sync stopped"}. kwek retries.` };
      default:
        return { dot: "bg-muted-foreground", text: "Starting", detail: "" };
    }
  });
</script>

<button
  class="flex h-7 min-w-0 items-center gap-1.5 rounded-md px-1.5 text-[12px] text-muted-foreground hover:bg-sidebar-hover hover:text-foreground"
  title="{view.detail} Click to check now."
  aria-label="{view.text}. {view.detail} Check now."
  onclick={() => api.refresh()}
>
  <span class={cn("size-2 shrink-0 rounded-full", view.dot)} aria-hidden="true"></span>
  <span class="truncate">{view.text}</span>
</button>
