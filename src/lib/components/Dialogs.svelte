<script lang="ts">
  import * as Dialog from "$lib/components/ui/dialog/index.js";
  import { Button } from "$lib/components/ui/button/index.js";
  import { api, errorText } from "$lib/api";
  import { EMOJI, QUICK } from "$lib/emoji";
  import { bindings } from "$lib/keys";
  import { app } from "$lib/state.svelte";
  import { toast } from "svelte-sonner";

  // Keyboard help
  let keysOpen = $state(false);
  $effect(() => {
    keysOpen = app.overlay === "keys";
  });
  $effect(() => {
    if (!keysOpen && app.overlay === "keys") app.overlay = null;
  });
  const groups = ["Navigate", "Messages", "General"] as const;

  // Delete confirmation
  let deleteOpen = $state(false);
  $effect(() => {
    deleteOpen = app.confirmDelete !== null;
  });
  $effect(() => {
    if (!deleteOpen && app.confirmDelete) app.confirmDelete = null;
  });

  // Sign-out confirmation
  async function signOut() {
    app.confirmSignOut = false;
    try {
      await api.logout();
    } catch (e) {
      toast.error(errorText(e));
    }
  }

  // Reaction picker
  let reactOpen = $state(false);
  let filter = $state("");
  $effect(() => {
    reactOpen = app.reactTo !== null;
    if (reactOpen) filter = "";
  });
  $effect(() => {
    if (!reactOpen && app.reactTo) app.reactTo = null;
  });
  const shown = $derived(
    filter.trim() ? EMOJI.filter(([n]) => n.includes(filter.trim().toLowerCase())).map(([, e]) => e) : [...new Set(EMOJI.map(([, e]) => e))],
  );
  const mine = $derived(new Set(app.reactTo?.reactions.filter((r) => r.mine).map((r) => r.emoji) ?? []));
  function pick(e: string) {
    const m = app.reactTo;
    app.reactTo = null;
    if (m) void app.react(m, e);
  }
</script>

<Dialog.Root bind:open={keysOpen}>
  <Dialog.Content class="sm:max-w-lg">
    <Dialog.Header>
      <Dialog.Title>Keyboard shortcuts</Dialog.Title>
      <Dialog.Description>Letter keys work whenever you are not typing in a text field.</Dialog.Description>
    </Dialog.Header>
    <!-- Focusable so the list scrolls from the keyboard when the window is short. -->
    <!-- svelte-ignore a11y_no_noninteractive_tabindex -->
    <div class="max-h-[min(70vh,560px)] overflow-y-auto rounded-sm" tabindex="0" role="region" aria-label="Shortcut list">
      <dl class="m-0 grid grid-cols-[5.5rem_1fr] items-baseline gap-x-4 gap-y-1">
        {#each groups as g (g)}
          <dt class="col-span-2 mt-3 mb-0.5 text-[12px] font-semibold text-muted-foreground first:mt-0">{g}</dt>
          {#each bindings.filter((b) => b.group === g && b.keys !== "i") as b (b.keys)}
            <dt>
              <kbd class="inline-block min-w-7 rounded border border-input px-1.5 text-center font-sans text-[12px]">{b.keys}</kbd>
              {#if b.keys === "Enter"}<span class="text-[12px] text-muted-foreground"> or </span><kbd
                  class="inline-block min-w-5 rounded border border-input px-1.5 text-center font-sans text-[12px]">i</kbd
                >{/if}
            </dt>
            <dd class="m-0">{b.label}</dd>
          {/each}
        {/each}
      </dl>
    </div>
  </Dialog.Content>
</Dialog.Root>

<Dialog.Root bind:open={deleteOpen}>
  <Dialog.Content class="sm:max-w-sm" showCloseButton={false}>
    <Dialog.Header>
      <Dialog.Title>Delete this message?</Dialog.Title>
      <Dialog.Description>It disappears for everyone in the conversation. This cannot be undone.</Dialog.Description>
    </Dialog.Header>
    <Dialog.Footer>
      <Button variant="outline" onclick={() => (app.confirmDelete = null)}>Cancel</Button>
      <Button variant="destructive" onclick={() => app.confirmDelete && app.doDelete(app.confirmDelete)}>Delete message</Button>
    </Dialog.Footer>
  </Dialog.Content>
</Dialog.Root>

<Dialog.Root bind:open={app.confirmSignOut}>
  <Dialog.Content class="sm:max-w-sm" showCloseButton={false}>
    <Dialog.Header>
      <Dialog.Title>Sign out of kwek?</Dialog.Title>
      <Dialog.Description>
        kwek forgets your Google sign-in and deletes its local copy of your messages. Signing in again syncs them back.
      </Dialog.Description>
    </Dialog.Header>
    <Dialog.Footer>
      <Button variant="outline" onclick={() => (app.confirmSignOut = false)}>Cancel</Button>
      <Button variant="destructive" onclick={signOut}>Sign out</Button>
    </Dialog.Footer>
  </Dialog.Content>
</Dialog.Root>

<Dialog.Root bind:open={reactOpen}>
  <Dialog.Content class="sm:max-w-sm" showCloseButton={false}>
    <Dialog.Header>
      <Dialog.Title>React</Dialog.Title>
      <Dialog.Description class="sr-only">Pick an emoji to add or remove your reaction.</Dialog.Description>
    </Dialog.Header>
    <input
      bind:value={filter}
      class="selectable h-8 rounded-md border border-input px-2 outline-none focus:border-accent focus:ring-2 focus:ring-accent/30"
      placeholder="Find an emoji by name, Enter picks the first"
      aria-label="Find an emoji by name"
      onkeydown={(e) => {
        if (e.key === "Enter" && shown[0]) {
          e.preventDefault();
          pick(shown[0]);
        }
      }}
    />
    {#if !filter.trim()}
      <div class="flex gap-1" role="group" aria-label="Frequent">
        {#each QUICK as e (e)}
          <button class="emoji" class:mine={mine.has(e)} aria-pressed={mine.has(e)} onclick={() => pick(e)} aria-label="React with {e}">{e}</button>
        {/each}
      </div>
    {/if}
    <div class="grid max-h-48 grid-cols-8 gap-1 overflow-y-auto" role="group" aria-label={filter.trim() ? "Matching emoji" : "All emoji"}>
      {#each shown as e (e)}
        <button class="emoji" class:mine={mine.has(e)} aria-pressed={mine.has(e)} onclick={() => pick(e)} aria-label="React with {e}">{e}</button>
      {/each}
    </div>
  </Dialog.Content>
</Dialog.Root>

<style>
  .emoji {
    width: 36px;
    height: 36px;
    border-radius: 6px;
    font-size: 20px;
    line-height: 36px;
    text-align: center;
  }
  .emoji:hover {
    background: var(--muted);
  }
  .emoji:focus-visible {
    outline: 2px solid var(--ring);
    outline-offset: -2px;
  }
  .emoji.mine {
    background: var(--accent-soft);
    box-shadow: inset 0 0 0 1px var(--accent);
  }
</style>
