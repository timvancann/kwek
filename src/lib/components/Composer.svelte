<script lang="ts" module>
  // Unsent drafts per conversation.
  const drafts = new Map<string, string>();
</script>

<script lang="ts">
  import { api, type UserRef } from "$lib/api";
  import { searchEmoji } from "$lib/emoji";
  import { app } from "$lib/state.svelte";
  import { cn } from "$lib/utils";
  import { tick } from "svelte";
  import SendHorizontal from "@lucide/svelte/icons/send-horizontal";

  let { where, space, thread = null, label }: { where: "channel" | "thread"; space: string; thread?: string | null; label: string } =
    $props();

  const draftKey = $derived(thread ?? space);
  const zone = $derived(where === "thread" ? "threadComposer" : "composer");
  const editing = $derived(app.editing && app.editing.where === where ? app.editing : null);

  let el: HTMLTextAreaElement | undefined = $state();
  let text = $state("");
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
      if (loadedKey) drafts.set(loadedKey, text);
      loadedKey = k;
      text = drafts.get(k) ?? "";
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
    if (app.zone === zone && el && document.activeElement !== el && !app.overlay) el.focus();
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

  async function submit() {
    const value = text.trim();
    if (!value) return;
    if (editing) {
      text = "";
      await app.saveEdit(withMentions(value));
      return;
    }
    const body = withMentions(value);
    text = "";
    drafts.delete(draftKey);
    mentionMap = new Map();
    void tick().then(autosize);
    try {
      await app.send(body, where === "thread" ? thread : null);
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
      void submit();
      return;
    }
    if (e.key === "ArrowUp" && text === "" && !editing) {
      e.preventDefault();
      void app.editLast(where);
    }
  }
</script>

<div class="relative px-4 pb-3">
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

  <div
    class={cn(
      "flex items-end gap-2 rounded-lg border bg-background px-3 py-2 focus-within:border-accent focus-within:ring-2 focus-within:ring-accent/20",
      editing ? "border-highlight-strong" : "border-input",
    )}
  >
    <textarea
      bind:this={el}
      bind:value={text}
      oninput={() => {
        autosize();
        updateSuggestions();
      }}
      onkeydown={onKeydown}
      onclick={updateSuggestions}
      onfocus={() => (app.zone = zone)}
      onblur={() => drafts.set(draftKey, text)}
      rows="1"
      aria-label={label}
      aria-describedby={editing ? `edit-hint-${where}` : undefined}
      role="combobox"
      aria-expanded={suggestions.length > 0}
      aria-controls={listId}
      aria-autocomplete="list"
      aria-activedescendant={suggestions.length ? `${listId}-${active}` : undefined}
      placeholder={label}
      spellcheck="true"
      class="selectable max-h-[40vh] min-h-5 flex-1 resize-none bg-transparent leading-5 outline-none placeholder:text-muted-foreground"
    ></textarea>
    <button
      class="inline-flex size-7 shrink-0 items-center justify-center rounded-md text-muted-foreground enabled:hover:bg-accent enabled:hover:text-accent-foreground disabled:opacity-40"
      aria-label={editing ? "Save edit" : "Send"}
      title={editing ? "Save (Enter)" : "Send (Enter)"}
      disabled={!text.trim()}
      onclick={submit}
    >
      <SendHorizontal class="size-4" />
    </button>
  </div>
</div>
