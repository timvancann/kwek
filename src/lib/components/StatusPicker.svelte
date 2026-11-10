<script lang="ts">
  import * as Popover from "$lib/components/ui/popover/index.js";
  import { api, errorText, type Choice, type NewCustom } from "$lib/api";
  import { untilText } from "$lib/presence";
  import { app } from "$lib/state.svelte";
  import { cn } from "$lib/utils";
  import { untrack } from "svelte";
  import { toast } from "svelte-sonner";
  import PresenceDot from "./PresenceDot.svelte";
  import X from "@lucide/svelte/icons/x";

  let { anchor }: { anchor: HTMLElement | undefined } = $props();

  /** `keep`: the end the current status or do not disturb already has. */
  type ClearAfter = "keep" | "never" | "30m" | "1h" | "2h" | "4h" | "today" | "week" | "date";
  type Avail = "automatic" | "away" | "dnd";
  type DndFor = "keep" | "30m" | "1h" | "2h" | "tomorrow" | "time";

  const SUGGESTIONS: { emoji: string; text: string; clear: ClearAfter; label: string }[] = [
    { emoji: "📅", text: "In a meeting", clear: "1h", label: "1 hour" },
    { emoji: "🎧", text: "Focusing", clear: "2h", label: "2 hours" },
    { emoji: "🚆", text: "Commuting", clear: "30m", label: "30 min" },
    { emoji: "🤒", text: "Out sick", clear: "today", label: "Today" },
    { emoji: "🌴", text: "On holiday", clear: "date", label: "Pick a date" },
  ];

  let emoji = $state("");
  let text = $state("");
  let clear = $state<ClearAfter>("today");
  let until = $state("");
  let avail = $state<Avail>("automatic");
  let dndFor = $state<DndFor>("1h");
  let dndAt = $state("17:00");
  let saving = $state(false);
  /** The ends already set when it opened, offered as "keep". */
  let customEnd = $state<number | null>(null);
  let dndEnd = $state<number | null>(null);

  // Start from what's set now, each time it opens; not again while it's open, or an update
  // arriving mid-typing would wipe what's being typed.
  $effect(() => {
    if (!app.statusPicker) return;
    untrack(() => {
      const own = app.ownPresence;
      emoji = own.custom?.emoji ?? "";
      text = own.custom?.text ?? "";
      customEnd = own.custom?.untilUs ?? null;
      clear = own.custom ? (customEnd ? "keep" : "never") : "today";
      avail = own.state === "dnd" ? "dnd" : own.manualAway ? "away" : "automatic";
      dndEnd = own.state === "dnd" ? own.dndUntilUs : null;
      dndFor = dndEnd ? "keep" : "1h";
    });
  });

  const needsPermission = $derived(app.ownPresence.access === "needsPermission");

  /** Seconds until the chosen moment, from now. */
  function secondsUntil(ms: number): number {
    return Math.max(60, Math.round((ms - Date.now()) / 1000));
  }
  function endOfDay(d: Date): number {
    return new Date(d.getFullYear(), d.getMonth(), d.getDate() + 1).getTime();
  }

  function ttl(): number | null {
    const now = new Date();
    switch (clear) {
      case "keep":
        return customEnd ? secondsUntil(customEnd / 1000) : null;
      case "never":
        return null;
      case "30m":
        return 30 * 60;
      case "1h":
        return 60 * 60;
      case "2h":
        return 2 * 60 * 60;
      case "4h":
        return 4 * 60 * 60;
      case "today":
        return secondsUntil(endOfDay(now));
      case "week": {
        // Through Sunday.
        const days = (7 - now.getDay()) % 7;
        return secondsUntil(endOfDay(new Date(now.getFullYear(), now.getMonth(), now.getDate() + days)));
      }
      case "date": {
        if (!until) return secondsUntil(endOfDay(now));
        const [y, m, d] = until.split("-").map(Number);
        return secondsUntil(endOfDay(new Date(y, m - 1, d)));
      }
    }
  }

  function choice(): Choice {
    if (avail === "automatic") return { kind: "automatic" };
    if (avail === "away") return { kind: "away" };
    return { kind: "dnd", untilUs: dndUntil() * 1000 };
  }

  /** When do not disturb ends, in ms. A time already past today means tomorrow. */
  function dndUntil(): number {
    const now = new Date();
    switch (dndFor) {
      case "keep":
        return dndEnd ? dndEnd / 1000 : Date.now() + 60 * 60_000;
      case "tomorrow":
        return new Date(now.getFullYear(), now.getMonth(), now.getDate() + 1, 9).getTime();
      case "time": {
        const [h, m] = dndAt.split(":").map(Number);
        const at = new Date(now.getFullYear(), now.getMonth(), now.getDate(), h, m).getTime();
        return at > Date.now() ? at : at + 86_400_000;
      }
      default:
        return Date.now() + { "30m": 30, "1h": 60, "2h": 120 }[dndFor] * 60_000;
    }
  }

  function pick(s: (typeof SUGGESTIONS)[number]) {
    emoji = s.emoji;
    text = s.text;
    clear = s.clear;
  }

  async function save(custom: NewCustom | null) {
    saving = true;
    try {
      app.ownPresence = await api.setStatus(custom, choice());
      app.statusPicker = false;
      app.say(custom ? `Status set: ${custom.text}` : "Status cleared");
    } catch (e) {
      toast.error(errorText(e));
    } finally {
      saving = false;
    }
  }

  function onSave() {
    const t = text.trim();
    if (!t) {
      void save(null);
      return;
    }
    // Google needs an emoji with every status.
    void save({ emoji: emoji.trim() || "💬", text: t, ttlSecs: ttl() });
  }

  async function grant() {
    try {
      await api.grantAccess();
      app.statusPicker = false;
    } catch (e) {
      toast.error(errorText(e));
    }
  }

  const select = "rounded-md border border-border bg-background px-2 py-1 text-[13px]";
  const AVAIL: { id: Avail; label: string; dot: string }[] = [
    { id: "automatic", label: "Automatic", dot: "active" },
    { id: "away", label: "Away", dot: "away" },
    { id: "dnd", label: "Do not disturb", dot: "dnd" },
  ];
</script>

<Popover.Root bind:open={app.statusPicker}>
  <Popover.Content customAnchor={anchor} side="top" align="start" sideOffset={8} class="w-[330px] gap-0 p-3" aria-label="Your status">
    {#if needsPermission}
      <p class="m-0 text-[13px]">kwek needs permission to read and set your status. Signing in again asks for it once.</p>
      <div class="mt-3 flex justify-end">
        <button class="rounded-md bg-accent px-3 py-1.5 text-[13px] font-medium text-accent-foreground" onclick={grant}>Allow</button>
      </div>
    {:else}
      <div class="flex items-center gap-2 rounded-md border border-input px-2 py-1 focus-within:border-accent focus-within:ring-2 focus-within:ring-accent/20">
        <input
          bind:value={emoji}
          class="w-7 bg-transparent text-center text-[15px] outline-none"
          aria-label="Emoji (Control Command Space for the emoji picker)"
          title="Emoji: press Control Command Space for the picker"
          placeholder="💬"
        />
        <input bind:value={text} maxlength={64} class="min-w-0 flex-1 bg-transparent py-0.5 text-[14px] outline-none" placeholder="What's your status?" />
        {#if text || emoji}
          <button
            class="grid size-5 place-items-center rounded-full text-muted-foreground hover:bg-muted hover:text-foreground"
            aria-label="Empty the status"
            onclick={() => {
              text = "";
              emoji = "";
            }}><X class="size-3.5" aria-hidden="true" /></button
          >
        {/if}
      </div>

      <p class="m-0 mt-3 mb-1 px-0.5 text-[11.5px] font-semibold text-muted-foreground">Suggestions</p>
      <ul class="m-0 flex list-none flex-col p-0">
        {#each SUGGESTIONS as s (s.text)}
          <li>
            <button class="flex w-full items-center gap-2 rounded-md px-1.5 py-1 text-left text-[13px] hover:bg-muted" onclick={() => pick(s)}>
              <span aria-hidden="true">{s.emoji}</span>{s.text}<span class="ml-auto text-[12px] text-muted-foreground">{s.label}</span>
            </button>
          </li>
        {/each}
      </ul>

      <label class="mt-2.5 flex items-center gap-2 px-0.5 text-[13px]">
        <span class="text-muted-foreground">Clear after</span>
        <select bind:value={clear} class={select}>
          {#if customEnd}<option value="keep">{untilText(customEnd)}, as set</option>{/if}
          <option value="never">Don't clear</option>
          <option value="30m">30 minutes</option>
          <option value="1h">1 hour</option>
          <option value="2h">2 hours</option>
          <option value="4h">4 hours</option>
          <option value="today">Today</option>
          <option value="week">This week</option>
          <option value="date">Pick a date</option>
        </select>
        {#if clear === "date"}<input type="date" bind:value={until} class={select} aria-label="Clear after this day" />{/if}
      </label>

      <fieldset class="m-0 mt-3 border-0 p-0">
        <legend class="mb-1 px-0.5 text-[11.5px] font-semibold text-muted-foreground">Availability</legend>
        <div class="flex flex-col gap-0.5">
          {#each AVAIL as a (a.id)}
            <label
              class={cn(
                "flex cursor-pointer items-center gap-2.5 rounded-md px-1.5 py-1 text-[13px]",
                avail === a.id ? "bg-accent-soft font-semibold text-accent" : "hover:bg-muted",
              )}
            >
              <input type="radio" name="avail" value={a.id} bind:group={avail} class="sr-only" />
              <PresenceDot state={a.dot} size={9} ring="transparent" />
              {a.label}
              {#if a.id === "automatic"}<span class="ml-auto text-[12px] font-normal opacity-80">active while you're around</span>{/if}
              {#if a.id === "dnd" && avail === "dnd"}
                <span class="ml-auto flex items-center gap-1">
                  <select bind:value={dndFor} class="{select} font-normal text-foreground" aria-label="Do not disturb for">
                    {#if dndEnd}<option value="keep">Until {untilText(dndEnd)}, as set</option>{/if}
                    <option value="30m">30 minutes</option>
                    <option value="1h">1 hour</option>
                    <option value="2h">2 hours</option>
                    <option value="tomorrow">Until tomorrow 09:00</option>
                    <option value="time">Until a time</option>
                  </select>
                  {#if dndFor === "time"}
                    <input type="time" bind:value={dndAt} class="{select} font-normal text-foreground" aria-label="Do not disturb until" />
                  {/if}
                </span>
              {/if}
            </label>
          {/each}
        </div>
      </fieldset>

      <div class="mt-3 flex justify-end gap-1.5">
        <button class="rounded-md border border-border px-3 py-1.5 text-[13px] font-medium hover:bg-muted" disabled={saving} onclick={() => save(null)}>
          Clear status
        </button>
        <button class="rounded-md bg-accent px-3 py-1.5 text-[13px] font-medium text-accent-foreground disabled:opacity-60" disabled={saving} onclick={onSave}>
          Save
        </button>
      </div>
    {/if}
  </Popover.Content>
</Popover.Root>
