<script lang="ts">
  import { Button } from "$lib/components/ui/button/index.js";
  import { RefreshCw } from "@lucide/svelte";
  import StatusBadge from "./status-badge.svelte";
  import ArtifactsStateBanner from "./artifacts-state-banner.svelte";
  import { readEngineBuildCacheStatus, type BuildCacheStatusResponse } from "$lib/daemon/client";
  import { resolveArtifactViewState } from "$lib/artifacts/workspace-state";
  import { toArtifactRequestError } from "$lib/artifacts/fetch-error";
  import { capabilityLabel } from "$lib/artifacts/capability";
  import { formatBytes } from "$lib/utils";
  import type { ArtifactRequestError } from "$lib/artifacts/workspace-state";

  let { engineId, connected }: { engineId: string; connected: boolean } = $props();

  let response = $state<BuildCacheStatusResponse | null>(null);
  let loading = $state(false);
  let error = $state<ArtifactRequestError | null>(null);

  async function load(id: string, isConnected: boolean, signal: AbortSignal) {
    if (!isConnected) {
      loading = false;
      return;
    }
    loading = true;
    try {
      const result = await readEngineBuildCacheStatus(id, { signal });
      if (signal.aborted) return;
      response = result;
      error = null;
    } catch (caught) {
      if (signal.aborted) return;
      error = toArtifactRequestError(caught);
    } finally {
      if (!signal.aborted) loading = false;
    }
  }

  $effect(() => {
    const id = engineId;
    const isConnected = connected;
    const controller = new AbortController();
    void load(id, isConnected, controller.signal);
    return () => controller.abort();
  });

  const viewState = $derived(
    resolveArtifactViewState({
      connected,
      loading,
      hasData: response !== null,
      error,
      capability: response?.support ?? null,
      itemCount: null,
    }),
  );
</script>

{#if viewState.kind === "ready" || viewState.kind === "refreshing" || viewState.kind === "stale"}
  <div class="flex flex-col gap-3">
    <div class="flex items-center justify-between gap-2">
      <div class="flex items-center gap-2">
        <span class="text-sm font-medium">Build cache</span>
        {#if response}
          <StatusBadge status={response.support} label={capabilityLabel(response.support)} />
        {/if}
      </div>
      <Button
        size="sm"
        variant="outline"
        disabled={loading}
        onclick={() => load(engineId, connected, new AbortController().signal)}
      >
        <RefreshCw class={loading ? "animate-spin" : undefined} />
        Refresh
      </Button>
    </div>

    {#if viewState.kind === "stale"}
      <p class="text-xs text-destructive">
        Couldn't refresh ({viewState.error.message}). Showing the last known status.
      </p>
    {/if}

    {#if response?.usage}
      <div class="grid grid-cols-2 gap-2 md:grid-cols-4">
        <div class="rounded-md border p-3">
          <div class="text-xs font-medium text-muted-foreground">Candidates</div>
          <div class="mt-1 font-semibold tabular-nums">
            {response.usage.candidate_count ?? "unknown"}
          </div>
        </div>
        <div class="rounded-md border p-3">
          <div class="text-xs font-medium text-muted-foreground">Reclaimable</div>
          <div class="mt-1 font-semibold tabular-nums">
            {response.usage.reclaimable_bytes !== null
              ? formatBytes(response.usage.reclaimable_bytes)
              : "unknown"}
          </div>
        </div>
        <div class="rounded-md border p-3">
          <div class="text-xs font-medium text-muted-foreground">Estimate</div>
          <div class="mt-1 font-semibold">{response.usage.estimate_kind.replaceAll("_", " ")}</div>
        </div>
        <div class="rounded-md border p-3">
          <div class="text-xs font-medium text-muted-foreground">Scope support</div>
          <div class="mt-1 font-semibold">{capabilityLabel(response.usage.support)}</div>
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
    onRetry={() => load(engineId, connected, new AbortController().signal)}
  />
{/if}
