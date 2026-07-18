<script lang="ts">
  import { Button } from "$lib/components/ui/button/index.js";
  import { RefreshCw } from "@lucide/svelte";
  import StatusBadge from "./status-badge.svelte";
  import ArtifactsStateBanner from "./artifacts-state-banner.svelte";
  import { readEngineBuildCacheStatus, type BuildCacheStatusResponse } from "$lib/daemon/client";
  import { resolveArtifactViewState } from "$lib/artifacts/workspace-state";
  import { toArtifactRequestError } from "$lib/artifacts/fetch-error";
  import { capabilityLabel } from "$lib/artifacts/capability";
  import {
    applyLoadError,
    applyLoadSuccess,
    applyNotConnected,
    initialScopedFetchState,
    resetForNewEngine,
    withLoading,
  } from "$lib/artifacts/scoped-fetch";
  import { formatBytes } from "$lib/utils";

  let { engineId, connected }: { engineId: string; connected: boolean } = $props();

  let fetchState = $state(initialScopedFetchState<BuildCacheStatusResponse>());

  async function load(id: string, isConnected: boolean, signal: AbortSignal, generation: number) {
    if (!isConnected) {
      fetchState = applyNotConnected(fetchState, generation);
      return;
    }
    fetchState = withLoading(fetchState);
    try {
      const result = await readEngineBuildCacheStatus(id, { signal });
      if (signal.aborted) return;
      fetchState = applyLoadSuccess(fetchState, generation, result);
    } catch (caught) {
      if (signal.aborted) return;
      fetchState = applyLoadError(fetchState, generation, toArtifactRequestError(caught));
    }
  }

  $effect(() => {
    const id = engineId;
    const isConnected = connected;

    // Engine changed (or first run): clear the previous engine's status
    // before requesting the new one's, so it can never render underneath
    // the new engine's header.
    fetchState = resetForNewEngine(fetchState);
    const generation = fetchState.generation;

    const controller = new AbortController();
    void load(id, isConnected, controller.signal, generation);
    return () => controller.abort();
  });

  const viewState = $derived(
    resolveArtifactViewState({
      connected,
      loading: fetchState.loading,
      hasData: fetchState.data !== null,
      error: fetchState.error,
      capability: fetchState.data?.support ?? null,
      itemCount: null,
    }),
  );
</script>

{#if viewState.kind === "ready" || viewState.kind === "refreshing" || viewState.kind === "stale"}
  <div class="flex flex-col gap-3">
    <div class="flex items-center justify-between gap-2">
      <div class="flex items-center gap-2">
        <span class="text-sm font-medium">Build cache</span>
        {#if fetchState.data}
          <StatusBadge
            status={fetchState.data.support}
            label={capabilityLabel(fetchState.data.support)}
          />
        {/if}
      </div>
      <Button
        size="sm"
        variant="outline"
        disabled={fetchState.loading}
        onclick={() => load(engineId, connected, new AbortController().signal, fetchState.generation)}
      >
        <RefreshCw class={fetchState.loading ? "animate-spin" : undefined} />
        Refresh
      </Button>
    </div>

    {#if viewState.kind === "stale"}
      <p class="text-xs text-destructive">
        Couldn't refresh ({viewState.error.message}). Showing the last known status.
      </p>
    {/if}

    {#if fetchState.data?.usage}
      <div class="grid grid-cols-2 gap-2 md:grid-cols-4">
        <div class="rounded-md border p-3">
          <div class="text-xs font-medium text-muted-foreground">Candidates</div>
          <div class="mt-1 font-semibold tabular-nums">
            {fetchState.data.usage.candidate_count ?? "unknown"}
          </div>
        </div>
        <div class="rounded-md border p-3">
          <div class="text-xs font-medium text-muted-foreground">Reclaimable</div>
          <div class="mt-1 font-semibold tabular-nums">
            {fetchState.data.usage.reclaimable_bytes !== null
              ? formatBytes(fetchState.data.usage.reclaimable_bytes)
              : "unknown"}
          </div>
        </div>
        <div class="rounded-md border p-3">
          <div class="text-xs font-medium text-muted-foreground">Estimate</div>
          <div class="mt-1 font-semibold">
            {fetchState.data.usage.estimate_kind.replaceAll("_", " ")}
          </div>
        </div>
        <div class="rounded-md border p-3">
          <div class="text-xs font-medium text-muted-foreground">Scope support</div>
          <div class="mt-1 font-semibold">{capabilityLabel(fetchState.data.usage.support)}</div>
        </div>
      </div>
    {:else}
      <p class="rounded-md border p-3 text-sm text-muted-foreground">
        No usage estimate is available from this engine right now.
      </p>
    {/if}

    <p class="text-xs text-muted-foreground">
      This is a read-only view. Reclaiming build cache isn't available from Studio yet.
    </p>
  </div>
{:else}
  <ArtifactsStateBanner
    state={viewState}
    itemNoun="build-cache status"
    onRetry={() => load(engineId, connected, new AbortController().signal, fetchState.generation)}
  />
{/if}
