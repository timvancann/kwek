<script lang="ts">
  import * as Dialog from "$lib/components/ui/dialog/index.js";
  import { app } from "$lib/state.svelte";
  import ActivityPanel from "./ActivityPanel.svelte";

  let open = $state(false);
  $effect(() => {
    open = app.overlay === "activity";
  });
  $effect(() => {
    if (!open && app.overlay === "activity") app.overlay = null;
  });
</script>

<Dialog.Root bind:open>
  <Dialog.Content class="top-[10%] max-w-xl translate-y-0 gap-0 p-0 sm:max-w-xl">
    <ActivityPanel active={open}>
      {#snippet title()}
        <Dialog.Title>Activity</Dialog.Title>
        <Dialog.Description class="sr-only">Mentions of you, replies in your threads, saved messages, reminders and scheduled messages.</Dialog.Description>
      {/snippet}
    </ActivityPanel>
  </Dialog.Content>
</Dialog.Root>
