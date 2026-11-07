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

  onMount(() => {
    void app.init();
    const onKey = (e: KeyboardEvent) => handleKey(e);
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  });
</script>

{#if !app.ready}
  <div class="h-full bg-sidebar" data-tauri-drag-region></div>
{:else if app.auth.state !== "signedIn"}
  <Login />
{:else}
  <div class="relative flex h-full">
    <Sidebar />
    <ChannelView />
    <ThreadPanel />
  </div>
  <QuickSwitcher />
  <SearchOverlay />
  <Dialogs />
{/if}

<div class="sr-only" aria-live="polite" aria-atomic="true">{app.announce}</div>
<Toaster position="bottom-right" closeButton />
