<script lang="ts">
  import * as Card from "$lib/components/ui/card/index.js";
  import { Badge } from "$lib/components/ui/badge/index.js";
  import { Button } from "$lib/components/ui/button/index.js";
  import { CheckCircle2, CircleAlert, FolderPlus, RefreshCw, Server } from "@lucide/svelte";
  import {
    listEngines,
    readEngineHealth,
    type StudioEngine,
  } from "$lib/daemon/client";
  import type { HealthState } from "$lib/daemon/daemon-state.svelte";

  let {
    healthState,
    projectCount,
    onImportClick,
    onRetry,
  }: {
    healthState: HealthState;
    projectCount: number;
    onImportClick: () => void;
    onRetry: () => void;
  } = $props();

  let engine = $state<StudioEngine | null>(null);
  let engineError = $state<string | null>(null);
  let checkingEngine = $state(false);

  const connected = $derived(healthState.kind === "connected");
  const hasProjects = $derived(projectCount > 0);
  const engineReachable = $derived(engine?.last_health?.reachable === true);
  const showPanel = $derived(!connected || !hasProjects || !engineReachable);

  $effect(() => {
    if (!connected) {
      engine = null;
      engineError = null;
      return;
    }

    const controller = new AbortController();
    void refreshEngine(controller.signal);
    return () => controller.abort();
  });

  async function refreshEngine(signal?: AbortSignal) {
    checkingEngine = true;
    engineError = null;
    try {
      const engines = await listEngines({ signal });
      const selected = engines.find((item) => item.is_default) ?? engines[0] ?? null;
      if (!selected) {
        engine = null;
        engineError = "No engine provider is registered.";
        return;
      }
      const health = await readEngineHealth(selected.id, { signal });
      engine = { ...selected, last_health: health };
      engineError = health.error;
    } catch (error) {
      if (!signal?.aborted) {
        engine = null;
        engineError = error instanceof Error ? error.message : "Engine check failed.";
      }
    } finally {
      if (!signal?.aborted) {
        checkingEngine = false;
      }
    }
  }

  function stepVariant(ready: boolean): "default" | "outline" | "destructive" {
    return ready ? "default" : "outline";
  }
</script>

{#if showPanel}
  <Card.Root class="gap-4 p-4">
    <div class="flex flex-col gap-1">
      <div class="flex flex-wrap items-center gap-2">
        <h2 class="text-base font-semibold">Beta setup</h2>
        <Badge variant={connected && engineReachable && hasProjects ? "default" : "secondary"}>
          {connected && engineReachable && hasProjects ? "Ready" : "Needs attention"}
        </Badge>
      </div>
      <p class="max-w-2xl text-sm text-muted-foreground">
        Get the local workspace to a usable state before running project actions.
      </p>
    </div>

    <div class="grid gap-3 md:grid-cols-3">
      <div class="flex min-w-0 gap-3 rounded-md border p-3">
        {#if connected}
          <CheckCircle2 class="mt-0.5 size-4 shrink-0 text-primary" />
        {:else}
          <CircleAlert class="mt-0.5 size-4 shrink-0 text-destructive" />
        {/if}
        <div class="min-w-0 space-y-2">
          <div class="flex items-center gap-2">
            <p class="text-sm font-medium">Daemon</p>
            <Badge variant={stepVariant(connected)} class="text-xs">{healthState.label}</Badge>
          </div>
          <p class="text-xs text-muted-foreground">{healthState.detail}</p>
          {#if !connected}
            <Button size="sm" variant="outline" onclick={onRetry}>
              <RefreshCw />
              Retry
            </Button>
          {/if}
        </div>
      </div>

      <div class="flex min-w-0 gap-3 rounded-md border p-3">
        {#if engineReachable}
          <CheckCircle2 class="mt-0.5 size-4 shrink-0 text-primary" />
        {:else}
          <Server class="mt-0.5 size-4 shrink-0 text-muted-foreground" />
        {/if}
        <div class="min-w-0 space-y-2">
          <div class="flex items-center gap-2">
            <p class="text-sm font-medium">Engine</p>
            <Badge variant={engineReachable ? "default" : "outline"} class="text-xs">
              {engineReachable ? "Reachable" : "Check needed"}
            </Badge>
          </div>
          <p class="text-xs text-muted-foreground">
            {engine?.display_name ?? "Local Docker-compatible engine"}
            {#if engine?.last_health?.api_version}
              , API {engine.last_health.api_version}
            {/if}
          </p>
          {#if engineError}
            <p class="text-xs text-destructive">{engineError}</p>
          {/if}
          <Button
            size="sm"
            variant="outline"
            disabled={!connected || checkingEngine}
            onclick={() => refreshEngine()}
          >
            <RefreshCw />
            {checkingEngine ? "Checking" : "Recheck"}
          </Button>
        </div>
      </div>

      <div class="flex min-w-0 gap-3 rounded-md border p-3">
        {#if hasProjects}
          <CheckCircle2 class="mt-0.5 size-4 shrink-0 text-primary" />
        {:else}
          <FolderPlus class="mt-0.5 size-4 shrink-0 text-muted-foreground" />
        {/if}
        <div class="min-w-0 space-y-2">
          <div class="flex items-center gap-2">
            <p class="text-sm font-medium">Project</p>
            <Badge variant={stepVariant(hasProjects)} class="text-xs">
              {hasProjects ? `${projectCount} imported` : "None imported"}
            </Badge>
          </div>
          <p class="text-xs text-muted-foreground">
            Import a Compose file to inspect services, diagnostics, plans, and actions.
          </p>
          <Button size="sm" disabled={!connected} onclick={onImportClick}>
            <FolderPlus />
            Import
          </Button>
        </div>
      </div>
    </div>
  </Card.Root>
{/if}
