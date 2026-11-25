<script lang="ts">
  import { onMount } from "svelte";
  import { Toaster } from "$lib/components/ui/sonner/index.js";
  import ChannelView from "$lib/components/ChannelView.svelte";
  import Dialogs from "$lib/components/Dialogs.svelte";
  import Login from "$lib/components/Login.svelte";
  import QuickSwitcher from "$lib/components/QuickSwitcher.svelte";
  import SearchOverlay from "$lib/components/SearchOverlay.svelte";
  import ImageViewer from "$lib/components/ImageViewer.svelte";
  import Agenda from "$lib/components/Agenda.svelte";
  import Activity from "$lib/components/Activity.svelte";
  import ProfileCard from "$lib/components/ProfileCard.svelte";
  import Sidebar from "$lib/components/Sidebar.svelte";
  import ThreadPanel from "$lib/components/ThreadPanel.svelte";
  import PhoneHome from "$lib/components/PhoneHome.svelte";
  import PhoneSheets from "$lib/components/PhoneSheets.svelte";
  import { handleKey } from "$lib/keys";
  import { app } from "$lib/state.svelte";

  // Below 1000px the thread panel overlays the channel; the channel is then inert so
  // keyboard focus cannot land on controls hidden behind the panel.
  onMount(() => {
    const mq = window.matchMedia("(max-width: 1000px)");
    app.narrow = mq.matches;
    mq.addEventListener("change", (e) => (app.narrow = e.matches));
    // Phone-sized: one screen at a time (PhoneHome). Set before init, so a phone starts on
    // its conversation list.
    const phone = window.matchMedia("(max-width: 700px)");
    app.phone = phone.matches;
    phone.addEventListener("change", (e) => (app.phone = e.matches));
  });

  // The on-screen keyboard: Android pans the page up under it, which pushes the header off the
  // top. Instead kwek takes the height left above the keyboard, and the page never scrolls.
  onMount(() => {
    const vv = window.visualViewport;
    if (!vv) return;
    const root = document.documentElement;
    const fit = () => {
      if (!app.phone) {
        root.style.removeProperty("--app-h");
        return;
      }
      root.style.setProperty("--app-h", `${vv.height}px`);
      // The gesture bar sits behind the keyboard while it is open.
      if (vv.height < window.innerHeight - 120) root.style.setProperty("--safe-bottom", "0px");
      else root.style.removeProperty("--safe-bottom");
      if (window.scrollY !== 0) window.scrollTo(0, 0);
    };
    fit();
    vv.addEventListener("resize", fit);
    vv.addEventListener("scroll", fit);
    return () => {
      vv.removeEventListener("resize", fit);
      vv.removeEventListener("scroll", fit);
    };
  });

  // Android's Back goes through the webview's history: one history entry per open layer
  // (another tab, the conversation, the thread, a dialog or sheet). An entry is added as the
  // tap that opens its layer happens, because the webview skips entries added without one.
  // Back pops an entry and kwek closes the top layer; a layer closed another way takes its
  // entry with it. With nothing open, Back leaves kwek as usual.
  let pushed = 0;
  /** Pops kwek made itself, whose popstate must not close another layer. */
  let ignore = 0;
  $effect(() => {
    const depth = app.phone ? app.phoneDepth : 0;
    if (depth > pushed) {
      for (; pushed < depth; pushed++) history.pushState({ kwek: pushed + 1 }, "");
    } else if (depth < pushed) {
      ignore++;
      history.go(depth - pushed);
      pushed = depth;
    }
  });
  onMount(() => {
    const onPop = () => {
      if (ignore > 0) {
        ignore--;
        return;
      }
      if (pushed > 0) pushed--;
      if (app.phone) app.phoneBack();
    };
    window.addEventListener("popstate", onPop);
    return () => window.removeEventListener("popstate", onPop);
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
{:else if app.phone}
  <div class="relative flex flex-col overflow-hidden bg-background" style="height: var(--app-h, 100%)">
    {#if app.phoneView === "conversation" && app.currentSpace}
      <div class="flex min-h-0 flex-1" inert={app.thread !== null}>
        <ChannelView />
      </div>
    {:else}
      <PhoneHome />
    {/if}
    <ThreadPanel />
  </div>
  <PhoneSheets />
{:else}
  <div class="relative flex h-full">
    <Sidebar />
    <div class="flex min-w-0 flex-1" inert={app.narrow && app.thread !== null}>
      <ChannelView />
    </div>
    <ThreadPanel />
  </div>
{/if}
{#if app.ready && app.auth.state === "signedIn" && app.status.state !== "needsSetup"}
  <QuickSwitcher />
  <SearchOverlay />
  <ImageViewer />
  <Agenda />
  <Activity />
  <ProfileCard />
  <Dialogs />
{/if}

<div class="sr-only" aria-live="polite" aria-atomic="true">{app.announce}</div>
<!-- Lifted above the composers, so a pop-up never covers what you are typing. -->
<Toaster position="bottom-right" closeButton offset={{ bottom: 96 }} />
