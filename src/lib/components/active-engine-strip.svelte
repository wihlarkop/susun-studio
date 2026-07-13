<script lang="ts">
  import * as Card from "$lib/components/ui/card/index.js";
  import { Badge } from "$lib/components/ui/badge/index.js";
  import { Button } from "$lib/components/ui/button/index.js";
  import { ChevronDown, RefreshCw, Settings2, Trash2 } from "@lucide/svelte";
  import PruneDialog from "./prune-dialog.svelte";
  import {
    readEngineHealth,
    selectRuntimeProfile,
    type EngineHealth,
    type RuntimeProfile,
  } from "$lib/daemon/client";

  let {
    profiles,
    connected,
    onManageRuntimes,
    onChanged,
  }: {
    profiles: RuntimeProfile[];
    connected: boolean;
    onManageRuntimes: () => void;
    onChanged: () => void;
  } = $props();

  // The legacy engines row only names the prune URL — the daemon connects
  // via the active runtime profile regardless of this id.
  const LEGACY_ENGINE_ID = "engine-docker-local";

  let health = $state<EngineHealth | null>(null);
  let checking = $state(false);
  let switching = $state(false);
  let pruneDialogOpen = $state(false);

  const selected = $derived(profiles.find((profile) => profile.is_selected) ?? null);
  const selectedReady = $derived(selected?.connection.state === "summarized");

  $effect(() => {
    if (!connected) {
      health = null;
      return;
    }
    const controller = new AbortController();
    void recheck(controller.signal);
    return () => controller.abort();
  });

  async function recheck(signal?: AbortSignal) {
    checking = true;
    try {
      health = await readEngineHealth(LEGACY_ENGINE_ID, { signal });
    } catch {
      health = null;
    } finally {
      checking = false;
    }
  }

  async function switchProfile(event: Event) {
    const profileId = (event.currentTarget as HTMLSelectElement).value;
    if (!profileId || profileId === selected?.id) return;
    switching = true;
    try {
      await selectRuntimeProfile(profileId);
      onChanged();
      await recheck();
    } finally {
      switching = false;
    }
  }
</script>

<Card.Root class="gap-3 p-4">
  <div class="flex flex-wrap items-center justify-between gap-3">
    <div class="flex flex-wrap items-center gap-2">
      <h3 class="text-sm font-semibold">Active engine</h3>
      {#if selected}
        <span class="text-sm">{selected.display_name}</span>
        <Badge variant={selectedReady ? "default" : "outline"}>
          {selected.process.state.replace("_", " ")}
        </Badge>
        {#if health}
          <Badge variant={health.reachable ? "default" : "destructive"}>
            {health.reachable ? "Reachable" : "Unreachable"}
          </Badge>
        {/if}
        {#if health?.api_version}
          <span class="text-xs text-muted-foreground">Docker API {health.api_version}</span>
        {/if}
      {:else}
        <Badge variant="outline">None selected</Badge>
        <span class="text-xs text-muted-foreground">
          Projects fall back to the platform-default local engine.
        </span>
      {/if}
    </div>
    <div class="flex min-w-0 flex-wrap items-center justify-end gap-2">
      {#if profiles.length > 0}
        <div class="relative min-w-72 max-w-full flex-1 sm:flex-none">
          <select
            class="h-9 w-full appearance-none rounded-md border bg-background bg-none pr-9 pl-3 text-sm leading-5"
            disabled={switching || !connected}
            value={selected?.id ?? ""}
            onchange={switchProfile}
            aria-label="Switch active engine"
          >
            {#if !selected}
              <option value="">Select an engine…</option>
            {/if}
            {#each profiles as profile (profile.id)}
              <option value={profile.id}>
                {profile.display_name} ({profile.process.state.replace("_", " ")})
              </option>
            {/each}
          </select>
          <ChevronDown
            class="pointer-events-none absolute top-1/2 right-2 size-4 -translate-y-1/2 text-muted-foreground"
          />
        </div>
      {/if}
      <Button
        size="sm"
        variant="outline"
        disabled={checking || !connected}
        onclick={() => recheck()}
      >
        <RefreshCw />
        Recheck
      </Button>
      <Button size="sm" variant="outline" onclick={onManageRuntimes}>
        <Settings2 />
        Manage runtimes
      </Button>
      <Button
        size="sm"
        variant="destructive"
        disabled={!connected}
        onclick={() => (pruneDialogOpen = true)}
      >
        <Trash2 />
        Prune
      </Button>
    </div>
  </div>

  {#if health?.error}
    <p class="text-xs text-destructive">{health.error}</p>
  {/if}
  {#if selected && !selectedReady}
    <p class="text-xs text-muted-foreground">
      The active engine is not ready — open Manage runtimes to start it.
    </p>
  {/if}
</Card.Root>

<PruneDialog
  engineId={LEGACY_ENGINE_ID}
  runtimeName={selected
    ? `${selected.display_name} (${selected.provider_runtime_key})`
    : undefined}
  bind:open={pruneDialogOpen}
  oncompleted={recheck}
/>
