<script lang="ts">
  import * as Dialog from "$lib/components/ui/dialog/index.js";
  import { api, errorText, type MeetingView } from "$lib/api";
  import { agendaDay, clock, dayKey } from "$lib/format";
  import { app } from "$lib/state.svelte";
  import { cn } from "$lib/utils";
  import { toast } from "svelte-sonner";
  import External from "@lucide/svelte/icons/external-link";

  let open = $state(false);
  $effect(() => {
    open = app.overlay === "agenda";
  });
  $effect(() => {
    if (!open && app.overlay === "agenda") app.overlay = null;
  });

  /** Ticks while open, so "now" moves along. */
  let now = $state(Date.now() * 1000);
  $effect(() => {
    if (!open) return;
    now = Date.now() * 1000;
    const t = setInterval(() => (now = Date.now() * 1000), 20_000);
    return () => clearInterval(t);
  });

  /** The coming week's meetings, by day. */
  const days = $derived.by(() => {
    const out: { key: number; label: string; items: MeetingView[] }[] = [];
    for (const m of app.calendar.meetings.filter((x) => x.endUs > now)) {
      const key = dayKey(m.startUs);
      let day = out.find((d) => d.key === key);
      if (!day) {
        day = { key, label: agendaDay(m.startUs, now), items: [] };
        out.push(day);
      }
      day.items.push(m);
    }
    return out;
  });

  let connecting = $state(false);
  async function connect() {
    connecting = true;
    try {
      await api.connectCalendar();
    } catch (e) {
      toast.error(errorText(e));
    } finally {
      connecting = false;
    }
  }
</script>

<Dialog.Root bind:open>
  <Dialog.Content class="top-[10%] max-w-lg translate-y-0 gap-0 p-0 sm:max-w-lg">
    <div class="border-b border-border px-5 py-4">
      <Dialog.Title>Upcoming meetings</Dialog.Title>
      <Dialog.Description class="mt-0.5 text-[13px] text-muted-foreground">The coming week from your Google Calendar.</Dialog.Description>
    </div>
    <div class="max-h-[65vh] overflow-y-auto px-5 py-3">
      {#if app.calendar.state === "needsPermission"}
        <p class="my-3 text-[14px] text-muted-foreground">kwek doesn't have access to your calendar yet.</p>
        <button class="mb-3 rounded-md bg-primary px-3 py-1.5 text-[13px] font-medium text-primary-foreground hover:bg-primary/85" onclick={connect} disabled={connecting}>
          {connecting ? "Finish in the browser, then come back" : "Connect your calendar"}
        </button>
      {:else if app.calendar.state !== "ok"}
        <p class="my-3 text-[14px] text-muted-foreground">{app.calendar.detail ?? "Your calendar isn't loaded yet."}</p>
      {:else if days.length === 0}
        <p class="my-3 text-[14px] text-muted-foreground">Nothing planned for the coming week.</p>
      {:else}
        {#each days as day (day.key)}
          <h3 class="mt-3 mb-1 text-[12px] font-semibold tracking-wide text-muted-foreground uppercase first:mt-1">{day.label}</h3>
          <ul class="m-0 list-none p-0">
            {#each day.items as m (m.id + m.startUs)}
              {@const live = m.startUs <= now && now < m.endUs}
              <li class={cn("flex items-center gap-3 rounded-lg px-2 py-1.5", live && "bg-accent-soft")}>
                <span class={cn("w-[7.25rem] shrink-0 text-[13px] whitespace-nowrap tabular-nums", live ? "font-medium text-accent" : "text-muted-foreground")}>
                  {live ? "Now" : clock(m.startUs)}{" "}<span class="text-muted-foreground">to {clock(m.endUs)}</span>
                </span>
                <span class="min-w-0 flex-1 truncate text-[14px]" title={m.title}>{m.title}</span>
                {#if m.openUrl}
                  <button
                    class="grid size-7 shrink-0 place-items-center rounded-md text-muted-foreground hover:bg-muted hover:text-foreground"
                    aria-label="Open {m.title} in Google Calendar"
                    title="Open in Google Calendar"
                    onclick={() => m.openUrl && void api.openUrl(m.openUrl)}
                  >
                    <External class="size-3.5" aria-hidden="true" />
                  </button>
                {/if}
                {#if m.joinUrl}
                  <button
                    class="shrink-0 rounded-md bg-primary px-2.5 py-1 text-[12px] font-medium text-primary-foreground hover:bg-primary/85"
                    aria-label="Join {m.title}"
                    onclick={() => m.joinUrl && void api.openUrl(m.joinUrl)}>Join</button
                  >
                {:else}
                  <span class="w-[2.9rem] shrink-0" aria-hidden="true"></span>
                {/if}
              </li>
            {/each}
          </ul>
        {/each}
      {/if}
    </div>
  </Dialog.Content>
</Dialog.Root>
