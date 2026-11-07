<script lang="ts">
  import { api, errorText } from "$lib/api";
  import { app } from "$lib/state.svelte";
  import { Button } from "$lib/components/ui/button/index.js";

  let busy = $state(false);
  let error = $state("");

  async function connect() {
    busy = true;
    error = "";
    try {
      await api.login();
      app.auth = await api.authStatus();
      await app.loadSpaces();
    } catch (e) {
      error = errorText(e);
    } finally {
      busy = false;
    }
  }
</script>

<main data-tauri-drag-region class="flex h-full items-center justify-center bg-sidebar px-6">
  <div class="w-full max-w-[22rem]">
    <img src="/icon.png" alt="" width="56" height="56" class="mb-5 rounded-xl" draggable="false" />
    <h1 class="m-0 text-[22px] font-semibold tracking-tight">kwek</h1>
    <p class="mt-1 mb-6 text-[14px] text-muted-foreground">
      Google Chat for your Mac. Connect your Google account once; after that kwek keeps your conversations in sync, also offline.
    </p>
    <Button class="h-10 w-full text-[14px]" onclick={connect} disabled={busy}>
      {busy ? "Waiting for Google" : "Connect to Google"}
    </Button>
    <p class="mt-3 min-h-10 text-[12px] text-muted-foreground" role="status" aria-live="polite">
      {#if busy}
        Finish signing in in the browser tab that just opened, then come back here.
      {:else if error}
        <span class="text-destructive">{error}</span>
      {/if}
    </p>
  </div>
</main>
