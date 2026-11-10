<script lang="ts">
  import * as Popover from "$lib/components/ui/popover/index.js";
  import { api, errorText, type ProfileView } from "$lib/api";
  import { clock, meetingWhen } from "$lib/format";
  import { endOfToday, presenceWords } from "$lib/presence";
  import { app } from "$lib/state.svelte";
  import { toast } from "svelte-sonner";
  import Avatar from "./Avatar.svelte";
  import CalendarIcon from "@lucide/svelte/icons/calendar";
  import Hash from "@lucide/svelte/icons/hash";
  import Mail from "@lucide/svelte/icons/mail";

  let open = $state(false);
  let profile = $state<ProfileView | null>(null);
  let loading = $state(false);
  /** What opened the card: it anchors there and focus goes back to it. */
  let from: HTMLElement | undefined = $state();

  $effect(() => {
    const p = app.profileOf;
    open = p !== null;
    if (!p) return;
    from = p.from;
    // What kwek already knows shows at once; the rest fills in.
    profile = null;
    loading = true;
    const user = p.user;
    api
      .profile(user, endOfToday())
      .then((v) => {
        if (app.profileOf?.user === user) profile = v;
      })
      .catch((e) => toast.error(errorText(e)))
      .finally(() => (loading = false));
  });
  $effect(() => {
    if (!open && app.profileOf) app.profileOf = null;
  });

  const user = $derived(app.profileOf?.user ?? "");
  const presence = $derived(app.presence[user]);
  const words = $derived(presenceWords(presence));
  const custom = $derived(presence?.custom ?? null);
  /** A name before the card has loaded: from a DM in the sidebar, if there is one. */
  const known = $derived(app.spaces.find((s) => s.peer === user));
  const name = $derived(profile?.user.displayName ?? known?.displayName ?? "");
  const avatar = $derived(profile?.user.avatarUrl ?? known?.avatarUrl ?? null);

  const busy = $derived.by(() => {
    const b = profile?.busy;
    if (!b) return null;
    switch (b.kind) {
      case "busyUntil":
        return `Busy until ${clock(b.untilUs!)}, then free`;
      case "busyRestOfDay":
        return "Busy for the rest of the day";
      case "freeUntil":
        return `Free until ${clock(b.untilUs!)}`;
      default:
        return "Free for the rest of the day";
    }
  });
  const next = $derived(
    profile?.nextMeeting
      ? `Next with you: ${profile.nextMeeting.title}, ${meetingWhen(profile.nextMeeting.startUs, profile.nextMeeting.startUs, Date.now() * 1000)}`
      : null,
  );
  const role = $derived([profile?.title, profile?.department].filter(Boolean).join(", "));
  const spaces = $derived(profile?.sharedSpaces ?? []);

  function close() {
    app.profileOf = null;
  }

  function message() {
    const dm = profile?.dmSpace ?? known?.name;
    close();
    // No DM yet: kwek can't start one (spec D9); Google Chat can.
    if (dm) void app.open(dm);
    else void api.openUrl("https://chat.google.com/");
  }

  function openSpace(name: string) {
    close();
    void app.open(name);
  }

  async function copyEmail(email: string) {
    try {
      await navigator.clipboard.writeText(email);
      toast.success("Email copied");
    } catch {
      toast.error("Couldn't copy the email.");
    }
  }

  async function grant() {
    try {
      await api.grantAccess();
    } catch (e) {
      toast.error(errorText(e));
    }
  }

  function onCloseAutoFocus(e: Event) {
    if (from?.isConnected) {
      e.preventDefault();
      from.focus();
    }
  }

  const action =
    "flex-1 rounded-md border border-border py-1.5 text-center text-[13px] font-medium hover:bg-muted disabled:opacity-40";
</script>

<Popover.Root bind:open>
  <Popover.Content
    customAnchor={from}
    side="bottom"
    align="start"
    sideOffset={6}
    collisionPadding={12}
    class="w-[300px] gap-0 overflow-hidden p-0"
    aria-label={name ? `Profile of ${name}` : "Profile"}
    {onCloseAutoFocus}
  >
    <div class="flex items-start gap-3 px-4 pt-4 pb-3">
      <Avatar user={{ displayName: name || "?", avatarUrl: avatar }} size={56} presence={presence?.state ?? null} ring="var(--popover)" />
      <div class="min-w-0 flex-1">
        <h2 class="m-0 truncate text-[16px] leading-snug font-semibold">{name}</h2>
        {#if role}<p class="m-0 truncate text-[12.5px] text-muted-foreground">{role}</p>{/if}
        {#if words}
          <p class="m-0 mt-1 text-[12.5px]" class:text-destructive={presence?.state === "dnd"} class:text-muted-foreground={!presence?.state}>
            {words}
          </p>
        {/if}
      </div>
    </div>

    {#if custom}
      <div class="mx-4 flex items-center gap-2 rounded-md bg-muted px-2.5 py-1.5 text-[13px]">
        {#if custom.emoji}<span aria-hidden="true">{custom.emoji}</span>{/if}
        <span class="min-w-0 flex-1">{custom.text}</span>
        {#if custom.untilUs}<span class="shrink-0 text-[12px] text-muted-foreground">until {clock(custom.untilUs)}</span>{/if}
      </div>
    {/if}

    <div class="flex gap-1.5 px-4 py-3">
      <button class="{action} border-accent bg-accent text-accent-foreground hover:bg-accent/90" onclick={message}>Message</button>
      <button class={action} onclick={() => void api.openUrl("https://meet.google.com/new")}>Meet</button>
      <button
        class={action}
        disabled={!profile?.user.email}
        onclick={() => profile?.user.email && void api.openUrl(`mailto:${profile.user.email}`)}>Email</button
      >
    </div>

    <div class="flex flex-col gap-2 border-t border-border px-4 pt-2.5 pb-3 text-[13px]">
      {#if loading && !profile}
        <p class="m-0 text-muted-foreground">Loading…</p>
      {/if}
      {#if busy || next || profile?.calendar === "needsPermission"}
        <div class="flex items-start gap-2.5">
          <CalendarIcon class="mt-0.5 size-4 shrink-0 text-muted-foreground" aria-hidden="true" />
          <div class="min-w-0">
            {#if busy}
              <p class="m-0">{busy}</p>
            {:else if profile?.calendar === "needsPermission"}
              <p class="m-0 text-muted-foreground">
                Needs permission to show when they're free.
                <button class="text-accent underline underline-offset-2" onclick={grant}>Allow</button>
              </p>
            {/if}
            {#if next}<p class="m-0 text-[12.5px] text-muted-foreground">{next}</p>{/if}
          </div>
        </div>
      {/if}
      {#if spaces.length}
        <div class="flex items-start gap-2.5">
          <Hash class="mt-0.5 size-4 shrink-0 text-muted-foreground" aria-hidden="true" />
          <div class="min-w-0">
            <p class="m-0">In {spaces.length} {spaces.length === 1 ? "space" : "spaces"} with you</p>
            <div class="mt-1 flex flex-wrap gap-1">
              {#each spaces.slice(0, 3) as s (s.name)}
                <button class="rounded bg-muted px-1.5 text-[12px] hover:bg-sidebar-active" onclick={() => openSpace(s.name)}>{s.displayName}</button>
              {/each}
              {#if spaces.length > 3}<span class="px-1 text-[12px] text-muted-foreground">+{spaces.length - 3}</span>{/if}
            </div>
          </div>
        </div>
      {/if}
      {#if profile?.user.email}
        <div class="flex items-center gap-2.5">
          <Mail class="size-4 shrink-0 text-muted-foreground" aria-hidden="true" />
          <button
            class="min-w-0 truncate text-left hover:underline hover:underline-offset-2"
            title="Copy"
            aria-label="{profile.user.email}, copy"
            onclick={() => copyEmail(profile!.user.email!)}>{profile.user.email}</button
          >
        </div>
      {/if}
    </div>
  </Popover.Content>
</Popover.Root>
