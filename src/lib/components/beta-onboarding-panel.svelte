<script lang="ts">
  import * as Card from "$lib/components/ui/card/index.js";
  import { Badge } from "$lib/components/ui/badge/index.js";
  import { Button } from "$lib/components/ui/button/index.js";
  import { CheckCircle2, CircleAlert, FolderPlus, RefreshCw, Server } from "@lucide/svelte";
  import { readRuntimeStatus, type RuntimeProfile } from "$lib/daemon/client";
  import type { HealthState } from "$lib/daemon/daemon-state.svelte";

  let {
    healthState,
    projectCount,
    runtimeProfiles,
    onImportClick,
    onRetry,
    onSetupRuntime,
  }: {
    healthState: HealthState;
    projectCount: number;
    runtimeProfiles: RuntimeProfile[];
    onImportClick: () => void;
    onRetry: () => void;
    onSetupRuntime: () => void;
  } = $props();

  let checkingEngine = $state(false);

  const connected = $derived(healthState.kind === "connected");
  const hasProjects = $derived(projectCount > 0);
  const selectedProfile = $derived(
    runtimeProfiles.find((profile) => profile.is_selected) ?? null,
  );
  const engineReady = $derived(selectedProfile?.connection.state === "summarized");
  const showPanel = $derived(!connected || !hasProjects || !engineReady);

  // Re-runs provider detection daemon-side (repersisting profiles), then
  // refreshes shared state so the new observations land everywhere.
  async function recheckEngine() {
    checkingEngine = true;
    try {
      await readRuntimeStatus();
    } catch {
      // detection failure surfaces through the profiles list staying stale
    } finally {
      checkingEngine = false;
      onRetry();
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
        <Badge variant={connected && engineReady && hasProjects ? "default" : "secondary"}>
          {connected && engineReady && hasProjects ? "Ready" : "Needs attention"}
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
        {#if engineReady}
          <CheckCircle2 class="mt-0.5 size-4 shrink-0 text-primary" />
        {:else}
          <Server class="mt-0.5 size-4 shrink-0 text-muted-foreground" />
        {/if}
        <div class="min-w-0 space-y-2">
          <div class="flex items-center gap-2">
            <p class="text-sm font-medium">Engine</p>
            <Badge variant={engineReady ? "default" : "outline"} class="text-xs">
              {engineReady ? "Ready" : "Setup needed"}
            </Badge>
          </div>
          <p class="text-xs text-muted-foreground">
            {#if selectedProfile}
              {selectedProfile.display_name} — {selectedProfile.process.state.replace("_", " ")}
            {:else}
              No runtime selected yet. Set one up to run project actions.
            {/if}
          </p>
          <div class="flex gap-2">
            <Button
              size="sm"
              variant="outline"
              disabled={!connected || checkingEngine}
              onclick={recheckEngine}
            >
              <RefreshCw />
              {checkingEngine ? "Checking" : "Recheck"}
            </Button>
            {#if !engineReady}
              <Button size="sm" disabled={!connected} onclick={onSetupRuntime}>Set up</Button>
            {/if}
          </div>
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
