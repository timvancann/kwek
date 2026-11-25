<script lang="ts" module>
  /** A file in the composer: images and videos are previewed from `url`; kept for sending once
   * `id` is set. */
  export type DraftImage = {
    key: string;
    name: string;
    url: string;
    kind: "image" | "video" | "file";
    id: string | null;
    error: string | null;
  };

  /** Matches the limit in Rust; checked here too so a big file isn't read only to be refused. */
  const MAX_ATTACHMENT = 50 * 1024 * 1024;

  // Unsent drafts per conversation, text and attachments.
  const drafts = new Map<string, string>();
  const draftImages = new Map<string, DraftImage[]>();
</script>

<script lang="ts">
  import { api, errorText, type UserRef } from "$lib/api";
  import { emoji as emojiData, searchEmoji, usedEmoji } from "$lib/emoji.svelte";
  import * as Popover from "$lib/components/ui/popover/index.js";
  import EmojiPicker from "./EmojiPicker.svelte";
  import Smile from "@lucide/svelte/icons/smile";
  import Type from "@lucide/svelte/icons/type";
  import Bold from "@lucide/svelte/icons/bold";
  import Italic from "@lucide/svelte/icons/italic";
  import Strikethrough from "@lucide/svelte/icons/strikethrough";
  import LinkIcon from "@lucide/svelte/icons/link";
  import List from "@lucide/svelte/icons/list";
  import Quote from "@lucide/svelte/icons/text-quote";
  import Code from "@lucide/svelte/icons/code";
  import SquareCode from "@lucide/svelte/icons/square-code";
  import Eye from "@lucide/svelte/icons/eye";
  import ChevronUp from "@lucide/svelte/icons/chevron-up";
  import WhenPicker from "./WhenPicker.svelte";
  import Blocks from "./Blocks.svelte";
  import type { Block } from "$lib/bindings/Block";
  import { codeBlock, isUrl, link, prefixLines, wrap, type Edit } from "$lib/markup";
  import { MOD, MOD_SHIFT, mod, isMac } from "$lib/keys";
  import { app } from "$lib/state.svelte";
  import { cn } from "$lib/utils";
  import { tick } from "svelte";
  import { toast } from "svelte-sonner";
  import SendHorizontal from "@lucide/svelte/icons/send-horizontal";
  import Paperclip from "@lucide/svelte/icons/paperclip";
  import FileIcon from "@lucide/svelte/icons/file";
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

  // Not on a phone: there focus means the keyboard, which only a tap on the box should bring up.
  $effect(() => {
    void app.focusNonce;
    if (app.zone === zone && el && document.activeElement !== el && !app.overlay && !app.phone) el.focus();
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
    const closed = /(^|\s):([a-z0-9_+-]{1,40}):$/.exec(before);
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
      suggestions = searchEmoji(emoji[2]).map((x) => ({ id: x.e, label: `${x.e}  :${x.codes[0]}:`, insert: x.e }));
    } else if (closed) {
      // A whole `:shortcode:` typed out becomes its emoji, as in Slack.
      const x = emojiData.list.find((x) => x.codes.includes(closed[2]));
      if (x) {
        const start = caret - closed[2].length - 2;
        text = text.slice(0, start) + x.e + text.slice(caret);
        usedEmoji(x.e);
        void tick().then(() => el?.setSelectionRange(start + x.e.length, start + x.e.length));
      }
      trigger = null;
      suggestions = [];
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
    else usedEmoji(s.insert);
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

  // ------------------------------------------------------------- formatting

  /** Applies a formatting edit as typing would, so ⌘Z undoes it. */
  function apply(ed: Edit) {
    if (!el) return;
    el.focus();
    el.setSelectionRange(ed.start, ed.end);
    // Deprecated, but the one way to edit a textarea that its undo history keeps.
    if (!document.execCommand("insertText", false, ed.insert)) {
      text = text.slice(0, ed.start) + ed.insert + text.slice(ed.end);
    }
    void tick().then(() => {
      el?.setSelectionRange(ed.selStart, ed.selEnd);
      autosize();
    });
  }

  type Format = "bold" | "italic" | "strike" | "code" | "block" | "list" | "quote" | "link";
  function format(f: Format) {
    if (!el) return;
    const [s, e] = [el.selectionStart, el.selectionEnd];
    const ed =
      f === "bold"
        ? wrap(text, s, e, "*", "bold")
        : f === "italic"
          ? wrap(text, s, e, "_", "italic")
          : f === "strike"
            ? wrap(text, s, e, "~", "strikethrough")
            : f === "code"
              ? wrap(text, s, e, "`", "code")
              : f === "block"
                ? codeBlock(text, s, e)
                : f === "list"
                  ? prefixLines(text, s, e, "- ")
                  : f === "quote"
                    ? prefixLines(text, s, e, "> ")
                    : link(text, s, e);
    apply(ed);
  }

  /** ⌘B and friends, as in Slack; true when the key was one. */
  function formatKey(e: KeyboardEvent): boolean {
    if (!mod(e)) return false;
    const f: Format | null =
      !e.shiftKey && !e.altKey
        ? e.code === "KeyB"
          ? "bold"
          : e.code === "KeyI"
            ? "italic"
            : null
        : e.shiftKey && e.altKey
          ? e.code === "KeyC"
            ? "block"
            : null
          : // Not with Alt: AltGr is Ctrl+Alt on Windows and types [ ] ć and friends.
            !e.shiftKey || e.altKey
            ? null
            : e.code === "KeyX"
            ? "strike"
            : e.code === "KeyC"
              ? "code"
              : e.code === "KeyU"
                ? "link"
                : e.code === "Digit8"
                  ? "list"
                  : e.code === "Digit9"
                    ? "quote"
                    : null;
    if (!f) return false;
    e.preventDefault();
    format(f);
    return true;
  }

  let showFormatting = $state(loadFlag("kwek.formatBar"));
  let preview = $state(false);
  let previewBlocks = $state<Block[]>([]);
  function loadFlag(key: string) {
    try {
      return localStorage.getItem(key) === "1";
    } catch {
      return false;
    }
  }
  function toggleFormatting() {
    showFormatting = !showFormatting;
    if (!showFormatting) preview = false;
    try {
      localStorage.setItem("kwek.formatBar", showFormatting ? "1" : "0");
    } catch {
      // Private storage unavailable: remembered until restart.
    }
    el?.focus();
  }
  // The preview follows the text, a beat behind typing.
  $effect(() => {
    const t = text;
    if (!preview) return;
    const id = setTimeout(() => {
      api
        .previewMessage(t)
        .then((b) => (previewBlocks = b))
        .catch(() => (previewBlocks = []));
    }, 120);
    return () => clearTimeout(id);
  });

  /** A URL pasted over selected text links it, as in Slack. */
  function pasteLink(e: ClipboardEvent): boolean {
    const url = e.clipboardData?.getData("text/plain") ?? "";
    if (!el || el.selectionStart === el.selectionEnd || !isUrl(url) || isUrl(text.slice(el.selectionStart, el.selectionEnd))) return false;
    e.preventDefault();
    apply(link(text, el.selectionStart, el.selectionEnd, url.trim()));
    return true;
  }

  // ------------------------------------------------------------- emoji button

  let emojiOpen = $state(false);
  /** Where the caret was when the picker took focus. */
  let caretAt = { start: 0, end: 0 };
  /** Picked, to insert once the picker has closed and let go of the focus. */
  let picked: string | null = null;
  function insertEmoji(e: string) {
    picked = e;
    emojiOpen = false;
  }
  /** A popover closing hands the focus back to the text, not to its button. */
  function backToText(ev: Event) {
    ev.preventDefault();
    const e = picked;
    picked = null;
    if (e) apply({ start: caretAt.start, end: caretAt.end, insert: e, selStart: caretAt.start + e.length, selEnd: caretAt.start + e.length });
    else el?.focus();
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

  /** Adds files to the draft and keeps each for sending straight away, so sending is quick. */
  /** Pasted screenshots all arrive as "image.png"; number them so each can be told apart. */
  let pasted = 0;
  function displayName(file: File) {
    return !file.name || /^image\.(png|jpe?g|gif|webp|tiff?)$/i.test(file.name) ? `Pasted image ${++pasted}` : file.name;
  }

  function addFiles(files: Iterable<File>) {
    if (editing) {
      app.say("Attachments can't be added to an edit.");
      return;
    }
    let added = 0;
    for (const file of files) {
      if (file.size > MAX_ATTACHMENT) {
        toast.error(`${file.name || "That file"} is over 50 MB, too big to send from kwek.`);
        continue;
      }
      const item: DraftImage = {
        key: crypto.randomUUID(),
        name: displayName(file),
        url: URL.createObjectURL(file),
        kind: file.type.startsWith("video/") ? "video" : file.type.startsWith("image/") ? "image" : "file",
        id: null,
        error: null,
      };
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
    if (added) app.say(`${added} ${added === 1 ? "attachment" : "attachments"} added, preparing.`);
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
  const blockedBy = $derived(staging ? "Preparing attachments" : broken ? "Remove the attachment that couldn't be added to send" : null);

  function onPaste(e: ClipboardEvent) {
    if (pasteLink(e)) return;
    const files = [...(e.clipboardData?.files ?? [])];
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

  let laterOpen = $state(false);
  /** Keeps the message to send at `atUs`; the draft goes only once that worked. */
  async function scheduleSubmit(atUs: number) {
    laterOpen = false;
    if (!canSend || editing) return;
    const ids = images.map((i) => i.id).filter((id): id is string => !!id);
    const key = draftKey;
    const sent = images;
    try {
      await app.schedule(withMentions(text.trim()), where === "thread" ? thread : null, ids, atUs);
    } catch {
      return;
    }
    for (const i of sent) URL.revokeObjectURL(i.url);
    drafts.delete(key);
    draftImages.delete(key);
    // Switched conversations meanwhile: the composer now holds another draft, which stays.
    if (key !== draftKey) return;
    images = [];
    text = "";
    mentionMap = new Map();
    void tick().then(autosize);
  }
  const scheduledHere = $derived(app.scheduled.filter((s) => s.space === space && (where === "thread" ? s.thread === thread : !s.thread)).length);

  function onKeydown(e: KeyboardEvent) {
    if (formatKey(e)) return;
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
    // On a phone Enter is a new line and Send is the button, as in Slack's app.
    if (e.key === "Enter" && !e.shiftKey && !e.isComposing && !app.phone) {
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

<div class={cn("relative px-6 pt-1 pb-5", app.phone && "px-2 pb-[calc(var(--safe-bottom)+8px)]")}>
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

  {#if scheduledHere && !editing}
    <p class="mb-1 text-[12px] text-muted-foreground">
      <button class="underline underline-offset-2 hover:text-foreground" onclick={() => app.openActivity("scheduled")}
        >{scheduledHere} {scheduledHere === 1 ? "message" : "messages"} scheduled here</button
      >
    </p>
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
        Drop files to attach them
      </div>
    {/if}
    {#if showFormatting}
      <div class="flex items-center gap-0.5 border-b border-border px-2 py-1" role="toolbar" aria-label="Formatting">
        {#snippet tool(f: Format, label: string, keys: string, Icon: typeof Bold)}
          <button type="button" class="tool" aria-label={label} title="{label} ({keys})" onclick={() => format(f)}><Icon aria-hidden="true" /></button>
        {/snippet}
        {@render tool("bold", "Bold", `${MOD}B`, Bold)}
        {@render tool("italic", "Italic", `${MOD}I`, Italic)}
        {@render tool("strike", "Strikethrough", `${MOD_SHIFT}X`, Strikethrough)}
        <span class="mx-1 h-4 w-px bg-border" aria-hidden="true"></span>
        {@render tool("link", "Link", `${MOD_SHIFT}U`, LinkIcon)}
        <span class="mx-1 h-4 w-px bg-border" aria-hidden="true"></span>
        {@render tool("list", "Bulleted list", `${MOD_SHIFT}8`, List)}
        {@render tool("quote", "Quote", `${MOD_SHIFT}9`, Quote)}
        <span class="mx-1 h-4 w-px bg-border" aria-hidden="true"></span>
        {@render tool("code", "Code", `${MOD_SHIFT}C`, Code)}
        {@render tool("block", "Code block", `${isMac ? "⌘⌥⇧" : "Ctrl+Alt+Shift+"}C`, SquareCode)}
        <button
          type="button"
          class={cn("tool ml-auto", preview && "on")}
          aria-pressed={preview}
          aria-label="Preview"
          title="Preview how it will look"
          onclick={() => {
            preview = !preview;
            el?.focus();
          }}><Eye aria-hidden="true" /></button
        >
      </div>
      {#if preview}
        <div class="max-h-48 overflow-y-auto border-b border-border bg-muted/40 px-4 py-2 text-[15px] leading-[1.6] break-words" aria-label="Preview" role="region">
          {#if text.trim()}
            <Blocks blocks={previewBlocks} />
          {:else}
            <p class="m-0 text-[13px] text-muted-foreground">Type a message to see how it will look.</p>
          {/if}
        </div>
      {/if}
    {/if}
    {#if images.length}
      <ul class="m-0 flex list-none flex-wrap gap-2 px-3 pt-3" aria-label="Attachments to send">
        {#each images as img (img.key)}
          {@const label = img.error ? `${img.name}, couldn't be added` : img.id ? img.name : `${img.name}, preparing`}
          <li
            class={cn(
              "relative h-16 overflow-hidden rounded-lg border",
              img.kind === "file" ? "w-44 bg-muted" : "w-16",
              img.error ? "border-destructive" : "border-border",
            )}
          >
            {#if img.kind === "file"}
              <span
                class={cn("flex size-full items-center gap-2 py-2 pr-7 pl-2.5 text-[13px]", (!img.id || img.error) && "opacity-50")}
                aria-label="File {label}"
              >
                <FileIcon class="size-5 shrink-0 text-muted-foreground" aria-hidden="true" />
                <span class="line-clamp-2 break-all" aria-hidden="true">{img.name}</span>
              </span>
            {:else if img.kind === "video"}
              <!-- Its first frame, as the preview. -->
              <video
                src={img.url}
                muted
                preload="metadata"
                aria-label="Video {label}"
                class={cn("size-full bg-black object-cover", (!img.id || img.error) && "opacity-50")}
              ></video>
            {:else}
              <img src={img.url} alt={label} class={cn("size-full object-cover", (!img.id || img.error) && "opacity-50")} draggable="false" />
            {/if}
            {#if !img.id && !img.error}
              <span class="absolute inset-0 grid place-items-center" aria-hidden="true">
                <LoaderCircle class="size-5 animate-spin text-foreground" />
              </span>
            {/if}
            <button
              id="remove-{img.key}"
              type="button"
              class="absolute top-0.5 right-0.5 grid size-5 place-items-center rounded-full bg-foreground/75 text-background hover:bg-foreground"
              aria-label="Remove {img.name}"
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
        aria-label="Attach files"
        title="Attach files (or paste or drop them here)"
        onclick={() => fileInput?.click()}
      >
        <Paperclip class="size-4" aria-hidden="true" />
      </button>
      <input
        bind:this={fileInput}
        type="file"
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
      type="button"
      class={cn(
        "inline-flex size-8 shrink-0 items-center justify-center rounded-lg text-muted-foreground hover:bg-muted hover:text-foreground",
        showFormatting && "bg-muted text-foreground",
      )}
      aria-pressed={showFormatting}
      aria-label="Formatting"
      title="Formatting toolbar"
      onclick={toggleFormatting}
    >
      <Type class="size-4" aria-hidden="true" />
    </button>
    <Popover.Root bind:open={emojiOpen}>
      <Popover.Trigger
        class="inline-flex size-8 shrink-0 items-center justify-center rounded-lg text-muted-foreground hover:bg-muted hover:text-foreground"
        aria-label="Emoji"
        title="Emoji"
        onpointerdown={() => el && (caretAt = { start: el.selectionStart, end: el.selectionEnd })}
        onkeydown={() => el && (caretAt = { start: el.selectionStart, end: el.selectionEnd })}
      >
        <Smile class="size-4" aria-hidden="true" />
      </Popover.Trigger>
      <Popover.Content side="top" align="end" class="w-auto p-2" aria-label="Emoji" onCloseAutoFocus={backToText}>
        <EmojiPicker verb="Insert" onpick={insertEmoji} />
      </Popover.Content>
    </Popover.Root>
    <div class="flex shrink-0">
      <button
        class={cn(
          "inline-flex size-8 shrink-0 items-center justify-center text-muted-foreground enabled:bg-primary enabled:text-primary-foreground enabled:hover:bg-primary/85 disabled:opacity-40",
          editing ? "rounded-lg" : "rounded-l-lg",
        )}
        aria-label={editing ? "Save edit" : "Send"}
        title={editing ? "Save (Enter)" : (blockedBy ?? "Send (Enter)")}
        disabled={!canSend}
        onclick={submit}
      >
        <SendHorizontal class="size-4" />
      </button>
      {#if !editing}
        <Popover.Root bind:open={laterOpen}>
          <Popover.Trigger
            class="inline-flex h-8 w-5 items-center justify-center rounded-r-lg border-l border-primary-foreground/25 text-muted-foreground enabled:bg-primary enabled:text-primary-foreground enabled:hover:bg-primary/85 disabled:opacity-40"
            aria-label="Send later"
            title="Send later"
            disabled={!canSend}
          >
            <ChevronUp class="size-3.5" aria-hidden="true" />
          </Popover.Trigger>
          <Popover.Content side="top" align="end" class="w-72 p-1.5" onCloseAutoFocus={backToText}>
            <WhenPicker title="Send later" onpick={scheduleSubmit}>
              <p class="m-0 px-2 pt-2 text-[12px] text-muted-foreground">kwek sends it then if it's running, otherwise as soon as it starts again.</p>
            </WhenPicker>
          </Popover.Content>
        </Popover.Root>
      {/if}
    </div>
    </div>
  </div>
</div>

<style>
  .tool {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 28px;
    height: 28px;
    border-radius: 6px;
    color: var(--muted-foreground);
  }
  .tool:hover,
  .tool.on {
    background: var(--muted);
    color: var(--foreground);
  }
  .tool:focus-visible {
    outline: 2px solid var(--ring);
    outline-offset: -2px;
  }
  .tool :global(svg) {
    width: 16px;
    height: 16px;
  }
</style>
