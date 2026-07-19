<script lang="ts">
  import { Button } from "$lib/components/ui/button/index.js";
  import { RefreshCw } from "@lucide/svelte";
  import StatusBadge from "./status-badge.svelte";
  import ArtifactsStateBanner from "./artifacts-state-banner.svelte";
  import ScopePruneControl from "./scope-prune-control.svelte";
  import { readEngineBuildCacheStatus, type BuildCacheStatusResponse } from "$lib/daemon/client";
  import { resolveArtifactViewState } from "$lib/artifacts/workspace-state";
  import { toArtifactRequestError } from "$lib/artifacts/fetch-error";
  import { capabilityLabel } from "$lib/artifacts/capability";
  import {
    applyLoadError,
    applyLoadSuccess,
    initialScopedFetchState,
    resetForNewEngine,
    withLoading,
  } from "$lib/artifacts/scoped-fetch";
  import { formatBytes } from "$lib/utils";

  let { engineId, connected }: { engineId: string; connected: boolean } = $props();

  let fetchState = $state(initialScopedFetchState<BuildCacheStatusResponse>());
  // Plain (non-reactive) counter — see the containers tab for why this must
  // never be read back out of `fetchState` inside the effect below.
  let generation = 0;

  // Runs only in an async continuation, after the request's first `await` —
  // never synchronously inside the effect.
  async function load(id: string, signal: AbortSignal, requestGeneration: number) {
    try {
      const result = await readEngineBuildCacheStatus(id, { signal });
      if (signal.aborted) return;
      fetchState = applyLoadSuccess(fetchState, requestGeneration, result);
    } catch (caught) {
      if (signal.aborted) return;
      fetchState = applyLoadError(fetchState, requestGeneration, toArtifactRequestError(caught));
    }
  }

  $effect(() => {
    const id = engineId;
    const isConnected = connected;

    // Engine changed (or first run): clear the previous engine's status
    // before requesting the new one's, so it can never render underneath
    // the new engine's header. Built from the plain counter, never read
    // back from `fetchState`.
    generation += 1;
    const myGeneration = generation;

    const controller = new AbortController();
    fetchState = resetForNewEngine(myGeneration, isConnected);
    if (isConnected) {
      void load(id, controller.signal, myGeneration);
    }
    return () => controller.abort();
  });

  function refresh() {
    fetchState = withLoading(fetchState);
    const controller = new AbortController();
    void load(engineId, controller.signal, fetchState.generation);
  }

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
      <div class="flex items-center gap-2">
        <ScopePruneControl
          {engineId}
          scope="build_cache"
          label="Prune build cache"
          disabled={fetchState.loading}
          oncompleted={refresh}
        />
        <Button size="sm" variant="outline" disabled={fetchState.loading} onclick={refresh}>
          <RefreshCw class={fetchState.loading ? "animate-spin" : undefined} />
          Refresh
        </Button>
      </div>
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
  </div>
{:else}
  <ArtifactsStateBanner state={viewState} itemNoun="build-cache status" onRetry={refresh} />
{/if}
