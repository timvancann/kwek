<script lang="ts" module>
  /** An image in the composer: shown from `url`, kept for sending once `id` is set. */
  export type DraftImage = { key: string; name: string; url: string; id: string | null; error: string | null };

  // Unsent drafts per conversation, text and images.
  const drafts = new Map<string, string>();
  const draftImages = new Map<string, DraftImage[]>();
</script>

<script lang="ts">
  import { api, errorText, type UserRef } from "$lib/api";
  import { searchEmoji } from "$lib/emoji";
  import { app } from "$lib/state.svelte";
  import { cn } from "$lib/utils";
  import { tick } from "svelte";
  import { toast } from "svelte-sonner";
  import SendHorizontal from "@lucide/svelte/icons/send-horizontal";
  import ImagePlus from "@lucide/svelte/icons/image-plus";
  import X from "@lucide/svelte/icons/x";
  import LoaderCircle from "@lucide/svelte/icons/loader-circle";

  let { where, space, thread = null, label }: { where: "channel" | "thread"; space: string; thread?: string | null; label: string } =
    $props();

  const draftKey = $derived(thread ?? space);
  const zone = $derived(where === "thread" ? "threadComposer" : "composer");
  const editing = $derived(app.editing && app.editing.where === where ? app.editing : null);

  let el: HTMLTextAreaElement | undefined = $state();
  let fileInput: HTMLInputElement | undefined = $state();
  let text = $state("");
  let images = $state<DraftImage[]>([]);
  /** A file is being dragged over the composer. */
  let dragging = $state(false);
  let members: UserRef[] = $state([]);
  /** Display name -> users/ID for mentions inserted via autocomplete. */
  let mentionMap = new Map<string, string>();

  type Suggestion = { id: string; label: string; detail?: string; insert: string; mention?: string };
  let suggestions: Suggestion[] = $state([]);
  let active = $state(0);
  let trigger: { start: number; end: number } | null = $state(null);
  const listId = $derived(`suggest-${where}`);

  // Switch drafts with the conversation.
  let loadedKey = "";
  $effect(() => {
    const k = draftKey;
    if (k !== loadedKey) {
      if (loadedKey) {
        drafts.set(loadedKey, text);
        draftImages.set(loadedKey, images);
      }
      loadedKey = k;
      text = drafts.get(k) ?? "";
      images = draftImages.get(k) ?? [];
      mentionMap = new Map();
      suggestions = [];
      void tick().then(autosize);
    }
  });

  $effect(() => {
    const s = space;
    void api
      .listMembers(s)
      .then((m) => (members = m))
      .catch(() => (members = []));
  });

  // Entering edit mode loads the message text.
  let editingName: string | null = null;
  $effect(() => {
    const ed = editing;
    if (ed && ed.name !== editingName) {
      editingName = ed.name;
      drafts.set(draftKey, text);
      text = ed.text;
      void tick().then(() => {
        autosize();
        el?.focus();
        el?.setSelectionRange(text.length, text.length);
      });
    } else if (!ed && editingName) {
      editingName = null;
      text = drafts.get(draftKey) ?? "";
      void tick().then(autosize);
    }
  });

  $effect(() => {
    void app.focusNonce;
    if (app.zone === zone && el && document.activeElement !== el && !app.overlay) el.focus();
  });

  // Tell screen reader users that suggestions appeared (a textarea cannot be a combobox).
  let lastCount = 0;
  $effect(() => {
    const n = suggestions.length;
    if (n > 0 && lastCount === 0) app.say(`${n} ${n === 1 ? "suggestion" : "suggestions"}. Arrow keys to choose, Enter to insert.`);
    lastCount = n;
  });

  function autosize() {
    if (!el) return;
    el.style.height = "auto";
    el.style.height = Math.min(el.scrollHeight, window.innerHeight * 0.4) + "px";
  }

  function updateSuggestions() {
    if (!el) return;
    const caret = el.selectionStart;
    const before = text.slice(0, caret);
    const mention = /(^|\s)@([\p{L}\p{N}._-]{0,30})$/u.exec(before);
    const emoji = /(^|\s):([a-z0-9_+-]{2,30})$/.exec(before);
    if (mention) {
      const q = mention[2].toLowerCase();
      trigger = { start: caret - mention[2].length - 1, end: caret };
      suggestions = members
        .filter((u) => u.displayName.toLowerCase().includes(q) || (u.email ?? "").toLowerCase().startsWith(q))
        .slice(0, 8)
        .map((u) => ({ id: u.name, label: u.displayName, detail: u.email ?? undefined, insert: `@${u.displayName} `, mention: u.name }));
      if ("all".startsWith(q)) suggestions.push({ id: "users/all", label: "all", detail: "Everyone in this conversation", insert: "@all ", mention: "users/all" });
    } else if (emoji) {
      trigger = { start: caret - emoji[2].length - 1, end: caret };
      suggestions = searchEmoji(emoji[2]).map(([n, e]) => ({ id: n, label: `${e}  :${n}:`, insert: e }));
    } else {
      trigger = null;
      suggestions = [];
    }
    active = 0;
  }

  function choose(s: Suggestion) {
    if (!trigger || !el) return;
    text = text.slice(0, trigger.start) + s.insert + text.slice(trigger.end);
    if (s.mention) mentionMap.set(s.label, s.mention);
    const pos = trigger.start + s.insert.length;
    suggestions = [];
    trigger = null;
    void tick().then(() => {
      el?.focus();
      el?.setSelectionRange(pos, pos);
      autosize();
    });
  }

  /** "@Ada Lovelace" from autocomplete becomes a real mention: <users/ID>. */
  function withMentions(t: string): string {
    let out = t;
    for (const [name, id] of [...mentionMap.entries()].sort((a, b) => b[0].length - a[0].length)) {
      out = out.split(`@${name}`).join(`<${id}>`);
    }
    return out;
  }

  // ------------------------------------------------------------- images

  const staging = $derived(images.some((i) => !i.id && !i.error));
  const broken = $derived(images.some((i) => i.error));
  const canSend = $derived(editing ? !!text.trim() : (!!text.trim() || images.length > 0) && !staging && !broken);

  function patch(key: string, change: Partial<DraftImage>) {
    images = images.map((i) => (i.key === key ? { ...i, ...change } : i));
    // The draft may have moved on to another conversation meanwhile.
    for (const [k, list] of draftImages) draftImages.set(k, list.map((i) => (i.key === key ? { ...i, ...change } : i)));
  }

  /** Adds images to the draft and keeps each for sending straight away, so sending is quick. */
  /** Pasted screenshots all arrive as "image.png"; number them so each can be told apart. */
  let pasted = 0;
  function displayName(file: File) {
    return !file.name || /^image\.(png|jpe?g|gif|webp|tiff?)$/i.test(file.name) ? `Pasted image ${++pasted}` : file.name;
  }

  function addFiles(files: Iterable<File>) {
    if (editing) {
      app.say("Images can't be added to an edit.");
      return;
    }
    let added = 0;
    for (const file of files) {
      if (!file.type.startsWith("image/")) {
        toast.error(`${file.name || "That file"} isn't an image. kwek can only send images for now.`);
        continue;
      }
      const item: DraftImage = { key: crypto.randomUUID(), name: displayName(file), url: URL.createObjectURL(file), id: null, error: null };
      images = [...images, item];
      added++;
      api
        .stageAttachment(file)
        .then((id) => {
          // Removed while it was being kept: let it go again.
          if (!images.some((i) => i.key === item.key) && ![...draftImages.values()].flat().some((i) => i.key === item.key)) {
            void api.unstageAttachment(id);
            return;
          }
          patch(item.key, { id });
          if (images.length && images.every((i) => i.id)) app.say("Ready to send.");
        })
        .catch((e) => patch(item.key, { error: errorText(e) }));
    }
    if (added) app.say(`${added} ${added === 1 ? "image" : "images"} added, preparing.`);
    el?.focus();
  }

  /** Keyboard users remove images one after another: focus moves to the next remove button,
   * then the previous, and back to the text once none are left. */
  async function removeImage(item: DraftImage) {
    const at = images.findIndex((i) => i.key === item.key);
    if (item.id) void api.unstageAttachment(item.id);
    URL.revokeObjectURL(item.url);
    images = images.filter((i) => i.key !== item.key);
    // The saved copy of this draft is stale now; a staging that finishes later must see the
    // image is gone and let it go.
    draftImages.delete(draftKey);
    app.say(`Removed ${item.name}.`);
    await tick();
    const next = images[Math.min(at, images.length - 1)];
    const button = next && document.getElementById(`remove-${next.key}`);
    if (button) button.focus();
    else el?.focus();
  }

  /** Why Send is off, when images are the reason. */
  const blockedBy = $derived(staging ? "Preparing images" : broken ? "Remove the image that couldn't be added to send" : null);

  function onPaste(e: ClipboardEvent) {
    const files = [...(e.clipboardData?.files ?? [])].filter((f) => f.type.startsWith("image/"));
    if (files.length && !editing) {
      e.preventDefault();
      addFiles(files);
    }
  }

  function onDrop(e: DragEvent) {
    e.preventDefault();
    dragging = false;
    addFiles(e.dataTransfer?.files ?? []);
  }

  function onDragOver(e: DragEvent) {
    if (!e.dataTransfer?.types.includes("Files") || editing) return;
    e.preventDefault();
    e.dataTransfer.dropEffect = "copy";
    dragging = true;
  }

  async function submit() {
    const value = text.trim();
    if (!canSend) return;
    if (editing) {
      text = "";
      await app.saveEdit(withMentions(value));
      return;
    }
    const body = withMentions(value);
    const ids = images.map((i) => i.id).filter((id): id is string => !!id);
    // The kept images now belong to the outgoing message; only the previews go.
    for (const i of images) URL.revokeObjectURL(i.url);
    images = [];
    draftImages.delete(draftKey);
    text = "";
    drafts.delete(draftKey);
    mentionMap = new Map();
    void tick().then(autosize);
    try {
      await app.send(body, where === "thread" ? thread : null, ids);
    } catch {
      // The pending row shows the failure with retry; keep nothing here.
    }
  }

  function onKeydown(e: KeyboardEvent) {
    if (suggestions.length) {
      if (e.key === "ArrowDown" || e.key === "ArrowUp") {
        e.preventDefault();
        active = (active + (e.key === "ArrowDown" ? 1 : -1) + suggestions.length) % suggestions.length;
        return;
      }
      if (e.key === "Enter" || e.key === "Tab") {
        e.preventDefault();
        choose(suggestions[active]);
        return;
      }
      if (e.key === "Escape") {
        e.preventDefault();
        e.stopPropagation();
        suggestions = [];
        return;
      }
    }
    if (e.key === "Enter" && !e.shiftKey && !e.isComposing) {
      e.preventDefault();
      if (!canSend && blockedBy && !editing) app.say(`${blockedBy}.`);
      void submit();
      return;
    }
    if (e.key === "ArrowUp" && text === "" && !editing) {
      e.preventDefault();
      void app.editLast(where);
    }
  }
</script>

<div class="relative px-6 pt-1 pb-5">
  {#if suggestions.length}
    <ul
      id={listId}
      role="listbox"
      aria-label={trigger && text[trigger.start] === "@" ? "People" : "Emoji"}
      class="absolute right-4 bottom-full left-4 mb-1 max-h-64 overflow-y-auto rounded-md border border-border bg-popover p-1 shadow-md"
    >
      {#each suggestions as s, i (s.id)}
        <li
          id="{listId}-{i}"
          role="option"
          aria-selected={i === active}
          class={cn("flex cursor-default items-baseline gap-2 rounded-sm px-2 py-1", i === active && "bg-accent-soft")}
          onmousedown={(e) => {
            e.preventDefault();
            choose(s);
          }}
          onmousemove={() => (active = i)}
        >
          <span class="font-medium">{s.label}</span>
          {#if s.detail}<span class="truncate text-[12px] text-muted-foreground">{s.detail}</span>{/if}
        </li>
      {/each}
    </ul>
  {/if}

  {#if editing}
    <p class="mb-1 flex items-center gap-2 text-[12px] text-muted-foreground" id="edit-hint-{where}">
      <span class="rounded-sm bg-highlight-strong px-1.5 font-medium text-foreground">Editing</span>
      Enter saves, Esc cancels.
    </p>
  {/if}

  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div
    class={cn(
      "relative rounded-xl border bg-background shadow-xs focus-within:border-accent focus-within:ring-2 focus-within:ring-accent/20",
      editing ? "border-highlight-strong" : "border-input",
      dragging && "border-dashed border-accent",
    )}
    ondragover={onDragOver}
    ondragleave={(e) => {
      if (!e.currentTarget.contains(e.relatedTarget as Node)) dragging = false;
    }}
    ondrop={onDrop}
  >
    {#if dragging}
      <div class="pointer-events-none absolute inset-0 z-10 grid place-items-center rounded-xl bg-accent-soft/90 text-[14px] font-medium text-accent">
        Drop images to add them
      </div>
    {/if}
    {#if images.length}
      <ul class="m-0 flex list-none flex-wrap gap-2 px-3 pt-3" aria-label="Images to send">
        {#each images as img (img.key)}
          <li class={cn("relative size-16 overflow-hidden rounded-lg border", img.error ? "border-destructive" : "border-border")}>
            <img
              src={img.url}
              alt={img.error ? `${img.name}, couldn't be added` : img.id ? img.name : `${img.name}, preparing`}
              class={cn("size-full object-cover", (!img.id || img.error) && "opacity-50")}
              draggable="false"
            />
            {#if !img.id && !img.error}
              <span class="absolute inset-0 grid place-items-center" aria-hidden="true">
                <LoaderCircle class="size-5 animate-spin text-foreground" />
              </span>
            {/if}
            <button
              id="remove-{img.key}"
              type="button"
              class="absolute top-0.5 right-0.5 grid size-5 place-items-center rounded-full bg-foreground/75 text-background hover:bg-foreground"
              aria-label="Remove image {img.name}"
              title={img.error ?? `Remove ${img.name}`}
              onclick={() => removeImage(img)}
            >
              <X class="size-3" aria-hidden="true" />
            </button>
          </li>
        {/each}
      </ul>
      {#each images.filter((i) => i.error) as img (img.key)}
        <p class="m-0 px-3 pt-1.5 text-[12px] text-destructive" role="alert" title={img.error}>
          {img.name} couldn't be added. Remove it to send the rest.
        </p>
      {/each}
    {/if}
    <div class="flex items-end gap-2 py-2.5 pr-2.5 pl-2">
    {#if !editing}
      <button
        type="button"
        class="inline-flex size-8 shrink-0 items-center justify-center rounded-lg text-muted-foreground hover:bg-muted hover:text-foreground"
        aria-label="Add images"
        title="Add images (or paste or drop them here)"
        onclick={() => fileInput?.click()}
      >
        <ImagePlus class="size-4" aria-hidden="true" />
      </button>
      <input
        bind:this={fileInput}
        type="file"
        accept="image/*"
        multiple
        class="hidden"
        tabindex="-1"
        aria-hidden="true"
        onchange={(e) => {
          addFiles([...(e.currentTarget.files ?? [])]);
          e.currentTarget.value = "";
        }}
      />
    {:else}
      <!-- Same width as the button, so the text doesn't jump when editing starts. -->
      <span class="size-8 shrink-0" aria-hidden="true"></span>
    {/if}
    <textarea
      bind:this={el}
      bind:value={text}
      oninput={() => {
        autosize();
        updateSuggestions();
      }}
      onkeydown={onKeydown}
      onpaste={onPaste}
      onclick={updateSuggestions}
      onfocus={() => (app.zone = zone)}
      onblur={() => drafts.set(draftKey, text)}
      rows="1"
      aria-label={label}
      aria-describedby={editing ? `edit-hint-${where}` : undefined}
      aria-controls={suggestions.length ? listId : undefined}
      aria-autocomplete="list"
      aria-activedescendant={suggestions.length ? `${listId}-${active}` : undefined}
      placeholder={label}
      spellcheck="true"
      class="selectable max-h-[40vh] min-h-6 flex-1 resize-none self-center bg-transparent text-[15px] leading-6 outline-none placeholder:text-muted-foreground"
    ></textarea>
    <button
      class="inline-flex size-8 shrink-0 items-center justify-center rounded-lg text-muted-foreground enabled:bg-primary enabled:text-primary-foreground enabled:hover:bg-primary/85 disabled:opacity-40"
      aria-label={editing ? "Save edit" : "Send"}
      title={editing ? "Save (Enter)" : (blockedBy ?? "Send (Enter)")}
      disabled={!canSend}
      onclick={submit}
    >
      <SendHorizontal class="size-4" />
    </button>
    </div>
  </div>
</div>
