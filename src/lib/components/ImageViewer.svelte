<script lang="ts">
  import * as Dialog from "$lib/components/ui/dialog/index.js";
  import { api, errorText } from "$lib/api";
  import { stamp } from "$lib/format";
  import { app, viewable } from "$lib/state.svelte";
  import { toast } from "svelte-sonner";
  import ChevronLeft from "@lucide/svelte/icons/chevron-left";
  import ChevronRight from "@lucide/svelte/icons/chevron-right";
  import Download from "@lucide/svelte/icons/download";
  import X from "@lucide/svelte/icons/x";

  let open = $state(false);
  /** The image or video that opened the viewer: focus goes back to it on close. */
  let from: HTMLElement | undefined;
  $effect(() => {
    open = app.viewing !== null;
    if (app.viewing?.from) from = app.viewing.from;
  });
  $effect(() => {
    if (!open && app.viewing) app.viewing = null;
  });

  function onCloseAutoFocus(e: Event) {
    if (from?.isConnected) {
      e.preventDefault();
      from.focus();
    }
    from = undefined;
  }

  /** The message as it is now: an edit or deletion while the viewer is open shows here. */
  const message = $derived.by(() => {
    const v = app.viewing;
    if (!v) return null;
    const name = v.message.name;
    return app.messages[v.message.space]?.find((m) => m.name === name) ?? app.thread?.messages.find((m) => m.name === name) ?? v.message;
  });
  const images = $derived(message && !message.deleted ? viewable(message) : []);
  const index = $derived(app.viewing ? Math.min(app.viewing.index, Math.max(images.length - 1, 0)) : 0);
  const current = $derived(images[index]);
  /** A video WebKit couldn't play (an unsupported format, or the download failed). */
  let failed = $state<string | null>(null);

  // Nothing left to show (the message was deleted): close rather than show an empty frame.
  $effect(() => {
    if (open && app.viewing && !current) app.viewing = null;
  });

  function step(by: number) {
    if (!app.viewing || images.length < 2) return;
    app.viewing = { ...app.viewing, index: (index + by + images.length) % images.length };
    app.say(`${current?.name}, ${index + 1} of ${images.length}`);
  }

  function onKeydown(e: KeyboardEvent) {
    // On a video, the arrow keys seek.
    if (e.target instanceof HTMLVideoElement) return;
    if (e.key === "ArrowRight" || e.key === "ArrowLeft") {
      e.preventDefault();
      step(e.key === "ArrowRight" ? 1 : -1);
    }
  }

  async function save() {
    if (!current?.resource) return;
    try {
      await api.downloadAttachment(current.resource, current.name);
      toast.success(`Saved ${current.name} to Downloads`);
    } catch (e) {
      toast.error(errorText(e));
    }
  }

  const tool =
    "inline-flex size-8 items-center justify-center rounded-full text-muted-foreground hover:bg-muted hover:text-foreground disabled:opacity-40";
</script>

<Dialog.Root bind:open>
  <Dialog.Content
    showCloseButton={false}
    overlayClass="bg-black/80"
    class="flex w-auto max-w-[96vw] flex-col items-center gap-3 bg-transparent p-0 shadow-none ring-0 sm:max-w-[96vw]"
    onkeydown={onKeydown}
    {onCloseAutoFocus}
  >
    {#if current && message}
      <Dialog.Title class="sr-only">{current.name}</Dialog.Title>
      <Dialog.Description class="sr-only">
        Sent by {message.sender.displayName}, {stamp(message.createUs)}.
        {images.length > 1 ? "Left and right arrow keys show the others." : ""} Escape closes.
      </Dialog.Description>
      {#if current.kind === "video"}
        {#if failed === current.mediaUrl}
          <p class="m-0 grid aspect-video w-[min(960px,96vw)] max-h-[82vh] place-items-center rounded-lg bg-black p-6 text-center text-[14px] text-white/80">
            kwek can't play this video. Save it to Downloads to watch it.
          </p>
        {:else}
          <!-- svelte-ignore a11y_media_has_caption: Chat videos come without captions. -->
          <video
            src={current.mediaUrl}
            controls
            autoplay
            playsinline
            aria-label={current.name}
            class="block aspect-video max-h-[82vh] w-[min(960px,96vw)] rounded-lg bg-black object-contain shadow-2xl"
            onerror={() => (failed = current.mediaUrl)}
          ></video>
        {/if}
      {:else}
        <img
          src={current.mediaUrl}
          alt={current.name}
          class="block max-h-[82vh] max-w-[96vw] rounded-lg bg-background object-contain shadow-2xl"
          draggable="false"
        />
      {/if}
      <div class="flex max-w-[96vw] items-center gap-1 rounded-full bg-popover py-1 pr-1 pl-4 text-[13px] shadow-lg">
        <span class="max-w-80 truncate font-medium" title={current.name}>{current.name}</span>
        <span class="mr-2 ml-2 shrink-0 text-muted-foreground">
          {message.sender.displayName}, {stamp(message.createUs)}
        </span>
        {#if images.length > 1}
          <button type="button" class={tool} aria-label="Previous" title="Previous (Left arrow)" onclick={() => step(-1)}>
            <ChevronLeft class="size-4" aria-hidden="true" />
          </button>
          <span class="shrink-0 text-muted-foreground tabular-nums">{index + 1} of {images.length}</span>
          <button type="button" class={tool} aria-label="Next" title="Next (Right arrow)" onclick={() => step(1)}>
            <ChevronRight class="size-4" aria-hidden="true" />
          </button>
        {/if}
        <button type="button" class={tool} aria-label="Save to Downloads" title="Save to Downloads" onclick={save} disabled={!current.resource}>
          <Download class="size-4" aria-hidden="true" />
        </button>
        <Dialog.Close class={tool} aria-label="Close" title="Close (Esc)">
          <X class="size-4" aria-hidden="true" />
        </Dialog.Close>
      </div>
    {/if}
  </Dialog.Content>
</Dialog.Root>
