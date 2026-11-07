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
    <div class="grid max-h-[60vh] gap-4 overflow-y-auto">
      {#each groups as g (g)}
        <section>
          <h3 class="mb-1 text-[12px] font-semibold text-muted-foreground">{g}</h3>
          <dl class="m-0 grid grid-cols-[auto_1fr] gap-x-4 gap-y-1">
            {#each bindings.filter((b) => b.group === g && b.keys !== "i") as b (b.keys)}
              <dt><kbd class="inline-block min-w-7 rounded border border-input px-1.5 text-center font-sans text-[12px]">{b.keys}</kbd></dt>
              <dd class="m-0">{b.label}</dd>
            {/each}
          </dl>
        </section>
      {/each}
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
    <div class="flex gap-1" role="group" aria-label="Frequent">
      {#each QUICK as e (e)}
        <button class="emoji" onclick={() => pick(e)} aria-label="React with {e}">{e}</button>
      {/each}
    </div>
    <input
      bind:value={filter}
      class="selectable h-8 rounded-md border border-input px-2 outline-none focus:border-accent"
      placeholder="Find an emoji by name"
      aria-label="Find an emoji by name"
      onkeydown={(e) => {
        if (e.key === "Enter" && shown[0]) {
          e.preventDefault();
          pick(shown[0]);
        }
      }}
    />
    <div class="grid max-h-48 grid-cols-8 gap-1 overflow-y-auto" role="group" aria-label="All emoji">
      {#each shown as e (e)}
        <button class="emoji" onclick={() => pick(e)} aria-label="React with {e}">{e}</button>
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
  .emoji:hover,
  .emoji:focus-visible {
    background: var(--muted);
  }
</style>
