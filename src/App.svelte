<script lang="ts">
  import { onMount } from "svelte";
  import { Toaster } from "$lib/components/ui/sonner/index.js";
  import ChannelView from "$lib/components/ChannelView.svelte";
  import Dialogs from "$lib/components/Dialogs.svelte";
  import Login from "$lib/components/Login.svelte";
  import QuickSwitcher from "$lib/components/QuickSwitcher.svelte";
  import SearchOverlay from "$lib/components/SearchOverlay.svelte";
  import Sidebar from "$lib/components/Sidebar.svelte";
  import ThreadPanel from "$lib/components/ThreadPanel.svelte";
  import { handleKey } from "$lib/keys";
  import { app } from "$lib/state.svelte";

  // Below 1000px the thread panel overlays the channel; the channel is then inert so
  // keyboard focus cannot land on controls hidden behind the panel.
  onMount(() => {
    const mq = window.matchMedia("(max-width: 1000px)");
    app.narrow = mq.matches;
    mq.addEventListener("change", (e) => (app.narrow = e.matches));
  });

  // Files dropped outside a composer would make the webview open them in place of kwek.
  onMount(() => {
    const swallow = (e: DragEvent) => {
      if (e.dataTransfer?.types.includes("Files")) e.preventDefault();
    };
    window.addEventListener("dragover", swallow);
    window.addEventListener("drop", swallow);
    return () => {
      window.removeEventListener("dragover", swallow);
      window.removeEventListener("drop", swallow);
    };
  });

  onMount(() => {
    void app.init();
    const onKey = (e: KeyboardEvent) => handleKey(e);
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  });
</script>

{#if !app.ready}
  <div class="h-full bg-sidebar" data-tauri-drag-region></div>
{:else if app.auth.state !== "signedIn" || app.status.state === "needsSetup"}
  <!-- Signed in but without a Google client (needsSetup): the setup step asks for it. -->
  <Login />
{:else}
  <div class="relative flex h-full">
    <Sidebar />
    <div class="flex min-w-0 flex-1" inert={app.narrow && app.thread !== null}>
      <ChannelView />
    </div>
    <ThreadPanel />
  </div>
  <QuickSwitcher />
  <SearchOverlay />
  <Dialogs />
{/if}

<div class="sr-only" aria-live="polite" aria-atomic="true">{app.announce}</div>
<!-- Lifted above the composers, so a pop-up never covers what you are typing. -->
<Toaster position="bottom-right" closeButton offset={{ bottom: 96 }} />
