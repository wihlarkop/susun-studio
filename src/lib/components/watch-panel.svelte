<script lang="ts">
  import * as Card from "$lib/components/ui/card/index.js";
  import { Button } from "$lib/components/ui/button/index.js";
  import StatusBadge from "./status-badge.svelte";
  import { Eye, Plus, X } from "@lucide/svelte";
  import {
    listProjectWatchSessions,
    readWatchSession,
    startWatch,
    stopWatchSession,
    subscribeWatchEvents,
    type StudioProject,
    type StudioWatchSession,
    type SyncSpec,
    type WatchAction,
  } from "$lib/daemon/client";

  let { project }: { project: StudioProject | null } = $props();

  const projectId = $derived(project?.id ?? null);

  let session = $state<StudioWatchSession | null>(null);
  let starting = $state(false);
  let errorMessage = $state<string | null>(null);
  let events = $state<string[]>([]);
  let source: EventSource | null = null;

  let action = $state<WatchAction>("restart");
  let watchPathsInput = $state("");
  let servicesInput = $state("");
  let syncSpecs = $state<SyncSpec[]>([]);
  let trackRestartAsJob = $state(false);

  async function refresh() {
    if (!projectId) return;
    try {
      const sessions = await listProjectWatchSessions(projectId);
      session = sessions.find((candidate) => candidate.status === "running") ?? null;
      if (session) attach(session.id);
    } catch {
      // best-effort
    }
  }

  $effect(() => {
    void projectId;
    events = [];
    refresh();
    return () => source?.close();
  });

  function addSyncSpec() {
    syncSpecs = [...syncSpecs, { service: "", host_path: "", container_path: "" }];
  }

  function removeSyncSpec(index: number) {
    syncSpecs = syncSpecs.filter((_, i) => i !== index);
  }

  function attach(watchId: string) {
    source?.close();
    subscribeWatchEvents(watchId)
      .then((eventSource) => {
        source = eventSource;
        source.onmessage = (message) => {
          try {
            const event = JSON.parse(message.data) as { type: string; [key: string]: unknown };
            events = [...events.slice(-99), describeEvent(event)];
            if (event.type === "session_failed") {
              readWatchSession(watchId).then((updated) => (session = updated));
            } else if (event.type === "action_succeeded" || event.type === "action_failed") {
              readWatchSession(watchId).then((updated) => (session = updated));
            }
          } catch {
            // ignore malformed frames
          }
        };
      })
      .catch(() => {
        // best-effort — the session row itself still reflects real state
      });
  }

  function describeEvent(event: { type: string; [key: string]: unknown }): string {
    switch (event.type) {
      case "file_event":
        return `${event.kind} ${event.path}`;
      case "action_started":
        return `→ ${event.action} started`;
      case "action_succeeded":
        return `→ ${event.action} succeeded`;
      case "action_failed":
        return `→ ${event.action} failed: ${event.error}`;
      case "session_failed":
        return `watcher stopped: ${event.error}`;
      default:
        return JSON.stringify(event);
    }
  }

  async function start() {
    if (!projectId) return;
    starting = true;
    errorMessage = null;
    events = [];
    try {
      const watchPaths = watchPathsInput
        .split(",")
        .map((path) => path.trim())
        .filter(Boolean);
      const services = servicesInput
        .split(",")
        .map((name) => name.trim())
        .filter(Boolean);
      const started = await startWatch(projectId, {
        action,
        services,
        sync: syncSpecs.filter((spec) => spec.service && spec.host_path && spec.container_path),
        watch_paths: watchPaths,
        track_restart_as_job: trackRestartAsJob,
      });
      session = started;
      attach(started.id);
    } catch (error) {
      errorMessage = error instanceof Error ? error.message : "Failed to start watching";
    } finally {
      starting = false;
    }
  }

  async function stop() {
    if (!session) return;
    await stopWatchSession(session.id);
    source?.close();
    session = { ...session, status: "stopped" };
  }
</script>

<Card.Root class="gap-3 p-4">
  <div class="flex items-center justify-between gap-2">
    <div class="flex items-center gap-2">
      <Eye class="size-4" />
      <h3 class="text-lg font-semibold">Watch</h3>
      {#if session}
        <StatusBadge
          status={session.status}
          label={session.status === "running" ? "watching" : session.status}
        />
      {/if}
    </div>
    {#if session?.status === "running"}
      <Button size="sm" variant="destructive" onclick={stop}>Stop</Button>
    {/if}
  </div>

  {#if !project}
    <p class="text-muted-foreground text-sm">Select a project to watch its files.</p>
  {:else if session?.status === "running"}
    <div class="flex flex-wrap gap-1">
      {#each session.watch_paths.length ? session.watch_paths : ["(project root)"] as path (path)}
        <span class="bg-muted text-muted-foreground rounded px-2 py-0.5 font-mono text-xs"
          >{path}</span
        >
      {/each}
    </div>
    <p class="text-muted-foreground text-xs">
      On file change: <span class="text-foreground font-medium"
        >{session.action.replace("_", " + ")}</span
      >
    </p>
    {#if session.last_action_status}
      <p class="text-xs">
        Last action:
        <StatusBadge
          status={session.last_action_status}
          label={session.last_action_status === "succeeded" ? "succeeded" : "failed"}
        />
        {#if session.last_action_error}
          <span class="text-destructive block">{session.last_action_error}</span>
        {/if}
      </p>
    {/if}
    <div class="flex max-h-44 flex-col gap-1 overflow-y-auto text-xs">
      {#each events as line, index (index)}
        <div class="bg-secondary rounded px-2 py-1">{line}</div>
      {/each}
    </div>
  {:else}
    {#if errorMessage}
      <p class="text-destructive text-sm">{errorMessage}</p>
    {/if}
    <div class="flex flex-col gap-2 text-sm">
      <label class="flex flex-col gap-1">
        <span class="text-muted-foreground text-xs"
          >Watched paths (comma-separated, blank = project root)</span
        >
        <input
          bind:value={watchPathsInput}
          placeholder="src, docker"
          class="border-input rounded-md border bg-transparent px-2 py-1 text-sm"
        />
      </label>
      <label class="flex flex-col gap-1">
        <span class="text-muted-foreground text-xs">On file change</span>
        <select
          bind:value={action}
          class="border-input rounded-md border bg-transparent bg-none px-2 py-1 text-sm"
        >
          <option value="rebuild">Rebuild</option>
          <option value="restart">Restart</option>
          <option value="sync">Sync</option>
          <option value="sync_restart">Sync + Restart</option>
        </select>
      </label>
      {#if action === "restart" || action === "sync_restart" || action === "rebuild"}
        <label class="flex flex-col gap-1">
          <span class="text-muted-foreground text-xs">Services (comma-separated, blank = all)</span
          >
          <input
            bind:value={servicesInput}
            placeholder="web"
            class="border-input rounded-md border bg-transparent px-2 py-1 text-sm"
          />
        </label>
      {/if}
      {#if action === "sync" || action === "sync_restart"}
        <div class="flex flex-col gap-1">
          <span class="text-muted-foreground text-xs">Sync mappings</span>
          {#each syncSpecs as spec, index (index)}
            <div class="flex items-center gap-1">
              <input
                bind:value={spec.service}
                placeholder="service"
                class="border-input w-24 rounded-md border bg-transparent px-2 py-1 text-xs"
              />
              <input
                bind:value={spec.host_path}
                placeholder="host path"
                class="border-input flex-1 rounded-md border bg-transparent px-2 py-1 text-xs"
              />
              <span class="text-muted-foreground">→</span>
              <input
                bind:value={spec.container_path}
                placeholder="/app/path"
                class="border-input flex-1 rounded-md border bg-transparent px-2 py-1 text-xs"
              />
              <Button size="icon" variant="ghost" class="size-6" onclick={() => removeSyncSpec(index)}>
                <X class="size-3" />
              </Button>
            </div>
          {/each}
          <Button size="sm" variant="outline" class="w-fit" onclick={addSyncSpec}>
            <Plus class="size-3" />
            Add mapping
          </Button>
        </div>
      {/if}
      {#if action === "restart" || action === "sync_restart"}
        <label class="flex items-start gap-2 text-xs">
          <input type="checkbox" bind:checked={trackRestartAsJob} class="mt-0.5" />
          <span>
            Also track restart actions as Jobs
            <span class="text-muted-foreground block">
              Off by default — restarts stay instant. Turn on to see every watch-triggered restart in
              the Jobs page too.
            </span>
          </span>
        </label>
      {/if}
      <Button size="sm" disabled={starting} onclick={start} class="w-fit">
        {starting ? "Starting…" : "Start watching"}
      </Button>
    </div>
  {/if}
</Card.Root>
