<script lang="ts">
  import { Button } from "$lib/components/ui/button/index.js";
  import { RefreshCw } from "@lucide/svelte";
  import StatusBadge from "./status-badge.svelte";
  import ArtifactsStateBanner from "./artifacts-state-banner.svelte";
  import { readEngineRegistryCapability, type RegistryCapabilityResponse } from "$lib/daemon/client";
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

  let { engineId, connected }: { engineId: string; connected: boolean } = $props();

  let fetchState = $state(initialScopedFetchState<RegistryCapabilityResponse>());
  // Plain (non-reactive) counter — see the containers tab for why this must
  // never be read back out of `fetchState` inside the effect below.
  let generation = 0;

  // Runs only in an async continuation, after the request's first `await` —
  // never synchronously inside the effect.
  async function load(id: string, signal: AbortSignal, requestGeneration: number) {
    try {
      const result = await readEngineRegistryCapability(id, { signal });
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

    // Engine changed (or first run): clear the previous engine's
    // capabilities before requesting the new one's, so they can never
    // render underneath the new engine's header. Built from the plain
    // counter, never read back from `fetchState`.
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

  // Registry has no single top-level capability — pull/push/auth are three
  // independent flags, each rendered on its own. `capability: null` means
  // the shared resolver never reports a workspace-level "unsupported" here;
  // each flag speaks for itself once the response is ready.
  const viewState = $derived(
    resolveArtifactViewState({
      connected,
      loading: fetchState.loading,
      hasData: fetchState.data !== null,
      error: fetchState.error,
      capability: null,
      itemCount: null,
    }),
  );

  const flags = $derived(
    fetchState.data
      ? [
          { label: "Pull", support: fetchState.data.supports_pull },
          { label: "Push", support: fetchState.data.supports_push },
          { label: "Auth", support: fetchState.data.supports_auth },
        ]
      : [],
  );
</script>

{#if viewState.kind === "ready" || viewState.kind === "refreshing" || viewState.kind === "stale"}
  <div class="flex flex-col gap-3">
    <div class="flex items-center justify-between gap-2">
      <span class="text-sm font-medium">Registry capabilities</span>
      <Button size="sm" variant="outline" disabled={fetchState.loading} onclick={refresh}>
        <RefreshCw class={fetchState.loading ? "animate-spin" : undefined} />
        Refresh
      </Button>
    </div>

    {#if viewState.kind === "stale"}
      <p class="text-xs text-destructive">
        Couldn't refresh ({viewState.error.message}). Showing the last known capabilities.
      </p>
    {/if}

    <div class="grid grid-cols-1 gap-2 sm:grid-cols-3">
      {#each flags as flag (flag.label)}
        <div class="flex items-center justify-between rounded-md border p-3">
          <span class="text-sm font-medium">{flag.label}</span>
          <StatusBadge status={flag.support} label={capabilityLabel(flag.support)} />
        </div>
      {/each}
    </div>

    <p class="text-xs text-muted-foreground">
      Capability flags only. No live registry session, sign-in state, or credentials are shown or
      stored here.
    </p>
  </div>
{:else}
  <ArtifactsStateBanner state={viewState} itemNoun="registry capabilities" onRetry={refresh} />
{/if}
