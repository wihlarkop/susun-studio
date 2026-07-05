<script lang="ts">
  import { Button } from "$lib/components/ui/button/index.js";
  import { Input } from "$lib/components/ui/input/index.js";
  import StatusBadge from "./status-badge.svelte";
  import { openEventStream, type EngineEventPayload, type StudioProject } from "$lib/daemon/client";

  let { project }: { project: StudioProject | null } = $props();

  const MAX_EVENTS = 500;
  const DESTRUCTIVE_ACTIONS = new Set(["die", "kill", "destroy", "stop"]);
  const POSITIVE_ACTIONS = new Set(["start", "create"]);

  let watching = $state(false);
  let events = $state<(EngineEventPayload & { receivedAtMs: number })[]>([]);
  let filter = $state("");
  let errorMessage = $state<string | null>(null);
  let source: EventSource | null = null;

  /** Maps an engine action to a status recognized by StatusBadge's tone table. */
  function toneFor(action: string): string {
    if (DESTRUCTIVE_ACTIONS.has(action)) return "destructive";
    if (POSITIVE_ACTIONS.has(action)) return "safe";
    return "cancelled";
  }

  function relativeTime(ms: number): string {
    const deltaSeconds = Math.max(0, Math.round((Date.now() - ms) / 1000));
    if (deltaSeconds < 5) return "just now";
    if (deltaSeconds < 60) return `${deltaSeconds}s ago`;
    const minutes = Math.round(deltaSeconds / 60);
    if (minutes < 60) return `${minutes}m ago`;
    return `${Math.round(minutes / 60)}h ago`;
  }

  async function start() {
    if (!project || watching) return;
    stop();
    events = [];
    errorMessage = null;
    watching = true;
    try {
      source = await openEventStream(project.id);
      source.onmessage = (message) => {
        try {
          const event = JSON.parse(message.data) as EngineEventPayload;
          const receivedAtMs = event.time ? event.time * 1000 : Date.now();
          events = [...events.slice(-(MAX_EVENTS - 1)), { ...event, receivedAtMs }];
        } catch {
          // ignore malformed frames
        }
      };
      source.onerror = () => {
        errorMessage = "Event stream disconnected.";
        stop();
      };
    } catch (error) {
      errorMessage = error instanceof Error ? error.message : String(error);
      watching = false;
    }
  }

  function stop() {
    source?.close();
    source = null;
    watching = false;
  }

  const visible = $derived(
    filter.trim() === ""
      ? events
      : events.filter((event) => {
          const haystack = [
            event.kind,
            event.action,
            event.resource_id ?? "",
            ...Object.values(event.attributes ?? {}),
          ]
            .join(" ")
            .toLowerCase();
          return haystack.includes(filter.toLowerCase());
        }),
  );

  $effect(() => {
    return () => stop();
  });
</script>

{#if !project}
  <p class="text-muted-foreground text-sm">Select a project to watch events.</p>
{:else}
  <div class="flex flex-col gap-3">
    <div class="flex flex-wrap items-center gap-2">
      {#if watching}
        <Button size="sm" variant="outline" onclick={stop}>Stop</Button>
      {:else}
        <Button size="sm" onclick={start}>Watch</Button>
      {/if}
      <Input bind:value={filter} placeholder="Filter…" class="w-40" />
      <span class="text-muted-foreground ml-auto text-xs">{visible.length} / {events.length}</span>
    </div>

    {#if errorMessage}
      <p class="text-destructive text-sm">{errorMessage}</p>
    {/if}

    <div class="bg-muted/30 h-96 overflow-y-auto rounded-md border p-3 text-sm">
      {#if visible.length === 0}
        <p class="text-muted-foreground">
          Watch engine events for this project — container starts, stops, health changes.
        </p>
      {:else}
        <div class="flex flex-col gap-1">
          {#each visible as event, index (index)}
            <div class="flex items-center gap-2 border-b py-1 last:border-0">
              <span class="text-muted-foreground w-16 shrink-0 text-xs">
                {relativeTime(event.receivedAtMs)}
              </span>
              <StatusBadge status={toneFor(event.action)} label={`${event.kind}:${event.action}`} />
              {#if event.resource_id}
                <span class="text-muted-foreground font-mono text-xs">
                  {event.resource_id.slice(0, 12)}
                </span>
              {/if}
              {#if event.attributes?.name}
                <span class="text-xs">{event.attributes.name}</span>
              {/if}
              {#if event.attributes?.image}
                <span class="text-muted-foreground font-mono text-xs">
                  {event.attributes.image}
                </span>
              {/if}
            </div>
          {/each}
        </div>
      {/if}
    </div>
  </div>
{/if}
