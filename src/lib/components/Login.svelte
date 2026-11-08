<script lang="ts">
  import { onMount, tick } from "svelte";
  import { api, errorText, type GoogleSetupView } from "$lib/api";
  import { app } from "$lib/state.svelte";
  import { Button } from "$lib/components/ui/button/index.js";

  const GUIDE = "https://github.com/timvancann/kwek/blob/main/docs/setup.md";

  /** undefined while loading; null until the user gives kwek their client file. */
  let setup = $state<GoogleSetupView | null | undefined>(undefined);
  /** Showing the setup step again, to switch projects. */
  let changing = $state(false);
  let pasted = $state("");
  let busy = $state(false);
  let error = $state("");
  let fileInput: HTMLInputElement | undefined = $state();
  /** Focus a step's main action once it has rendered. */
  const focusById = (id: string) => document.getElementById(id)?.focus();

  const needsSetup = $derived(setup === null || changing);

  onMount(async () => {
    try {
      setup = await api.googleSetup();
    } catch (e) {
      setup = null;
      error = errorText(e);
    }
  });

  async function useClientFile(text: string) {
    busy = true;
    error = "";
    try {
      setup = await api.setGoogleSetup(text);
      changing = false;
      pasted = "";
      if (app.auth.state === "signedIn") {
        // Already signed in: the waiting session carries on, back to the conversations.
        app.status = { state: "starting", detail: null };
        app.say(`Google Cloud project ${setup.project} set.`);
        return;
      }
      app.say(`Google Cloud project ${setup.project} set. Next, connect to Google.`);
      // Enabled first: a disabled button can't take focus.
      busy = false;
      await tick();
      focusById("connect-button");
    } catch (e) {
      error = errorText(e);
    } finally {
      busy = false;
    }
  }

  async function onFile(e: Event) {
    const input = e.currentTarget as HTMLInputElement;
    const file = input.files?.[0];
    input.value = "";
    if (file) await useClientFile(await file.text());
  }

  /** Switches between the setup and connect steps, keeping focus on the step's main action. */
  async function showSetup(on: boolean) {
    changing = on;
    error = "";
    await tick();
    focusById(on ? "choose-button" : "connect-button");
  }

  function onPasteKeydown(e: KeyboardEvent) {
    if (e.key === "Enter" && e.metaKey && pasted.trim()) {
      e.preventDefault();
      void useClientFile(pasted);
    }
  }

  async function signOut() {
    busy = true;
    error = "";
    try {
      await api.logout();
      app.auth = await api.authStatus();
    } catch (e) {
      error = errorText(e);
    } finally {
      busy = false;
    }
  }

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
  <div class="w-full max-w-[24rem]">
    <img src="/icon.png" alt="" width="56" height="56" class="mb-5 rounded-xl" draggable="false" />
    {#if setup === undefined}
      <h1 class="m-0 font-display text-[26px] font-bold">kwek</h1>
    {:else if needsSetup}
      <h1 class="m-0 font-display text-[26px] font-bold">Set up kwek</h1>
      {#snippet fileName()}(named <span class="font-mono text-[13px]">client_secret_….json</span>, probably in Downloads){/snippet}
      <p class="mt-2 mb-5 text-[14px] leading-relaxed text-muted-foreground">
        {#if app.auth.state === "signedIn"}
          You're signed in, but this copy of kwek doesn't have your Google client file. Choose the one you set kwek up with {@render fileName()}.
        {:else}
          Finished the setup guide? Choose the client file you downloaded at the end {@render fileName()}. New to kwek? The guide takes about 15
          minutes. kwek signs in through your own Google Cloud project, so your messages only travel between your Mac and Google.
        {/if}
      </p>
      <Button id="choose-button" class="h-10 w-full text-[14px]" onclick={() => fileInput?.click()} disabled={busy}>Choose client file</Button>
      <input bind:this={fileInput} type="file" accept=".json,application/json" class="hidden" onchange={onFile} tabindex="-1" aria-hidden="true" />
      <Button variant="outline" class="mt-2 h-10 w-full text-[14px]" onclick={() => void api.openUrl(GUIDE)}>Open the setup guide in your browser</Button>
      <details class="mt-4 text-[13px] text-muted-foreground">
        <summary class="cursor-default select-none">Or paste the file's contents</summary>
        <label for="client-json" class="sr-only">Client file contents</label>
        <textarea
          id="client-json"
          bind:value={pasted}
          onkeydown={onPasteKeydown}
          rows="4"
          spellcheck="false"
          aria-describedby="login-status"
          aria-invalid={!!error}
          class="selectable mt-2 w-full resize-none rounded-md border border-input bg-background p-2 font-mono text-[12px] text-foreground"
          placeholder={'{"installed":{"client_id":"…"}}'}
        ></textarea>
        <Button variant="secondary" class="mt-1 h-8 text-[13px]" onclick={() => void useClientFile(pasted)} disabled={busy || !pasted.trim()}>
          Use pasted contents <span class="text-muted-foreground">⌘↩</span>
        </Button>
      </details>
      {#if changing}
        <button type="button" class="mt-3 text-[13px] text-muted-foreground underline underline-offset-4 hover:text-foreground" onclick={() => void showSetup(false)}>
          Keep project {setup?.project}
        </button>
      {:else if app.auth.state === "signedIn"}
        <!-- No file at hand: start over from signed out. -->
        <button type="button" class="mt-3 text-[13px] text-muted-foreground underline underline-offset-4 hover:text-foreground" onclick={signOut} disabled={busy}>
          Sign out instead
        </button>
      {/if}
    {:else}
      <h1 class="m-0 font-display text-[26px] font-bold">kwek</h1>
      <p class="mt-1 mb-6 text-[14px] text-muted-foreground">
        Google Chat for your Mac. Connect your Google account once; after that kwek keeps your conversations in sync and works offline.
      </p>
      <Button id="connect-button" class="h-10 w-full text-[14px]" onclick={connect} disabled={busy}>
        {busy ? "Waiting for Google" : "Connect to Google"}
      </Button>
      <p class="mt-3 mb-0 text-[12px] text-muted-foreground">
        Google Cloud project <span class="font-medium text-foreground">{setup?.project}</span>.
        <button type="button" class="underline underline-offset-4 hover:text-foreground" onclick={() => void showSetup(true)} disabled={busy}>Use a different one</button>
      </p>
    {/if}
    <p id="login-status" class="mt-3 min-h-10 text-[12px] text-muted-foreground" role="status" aria-live="polite">
      {#if busy && !needsSetup}
        Finish in the browser tab that just opened, then come back here.
      {:else if error}
        <span class="text-destructive">{error}</span>
      {/if}
    </p>
  </div>
</main>
