<script lang="ts">
  import { Button } from "$lib/components/ui/button/index.js";
  import { RefreshCw } from "@lucide/svelte";
  import StatusBadge from "./status-badge.svelte";
  import ArtifactsStateBanner from "./artifacts-state-banner.svelte";
  import { readEngineRegistryCapability, type RegistryCapabilityResponse } from "$lib/daemon/client";
  import { resolveArtifactViewState } from "$lib/artifacts/workspace-state";
  import { toArtifactRequestError } from "$lib/artifacts/fetch-error";
  import { capabilityLabel } from "$lib/artifacts/capability";
  import type { ArtifactRequestError } from "$lib/artifacts/workspace-state";

  let { engineId, connected }: { engineId: string; connected: boolean } = $props();

  let response = $state<RegistryCapabilityResponse | null>(null);
  let loading = $state(false);
  let error = $state<ArtifactRequestError | null>(null);

  async function load(id: string, isConnected: boolean, signal: AbortSignal) {
    if (!isConnected) {
      loading = false;
      return;
    }
    loading = true;
    try {
      const result = await readEngineRegistryCapability(id, { signal });
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

  // Registry has no single top-level capability — pull/push/auth are three
  // independent flags, each rendered on its own. `capability: null` means
  // the shared resolver never reports a workspace-level "unsupported" here;
  // each flag speaks for itself once the response is ready.
  const viewState = $derived(
    resolveArtifactViewState({
      connected,
      loading,
      hasData: response !== null,
      error,
      capability: null,
      itemCount: null,
    }),
  );

  const flags = $derived(
    response
      ? [
          { label: "Pull", support: response.supports_pull },
          { label: "Push", support: response.supports_push },
          { label: "Auth", support: response.supports_auth },
        ]
      : [],
  );
</script>

{#if viewState.kind === "ready" || viewState.kind === "refreshing" || viewState.kind === "stale"}
  <div class="flex flex-col gap-3">
    <div class="flex items-center justify-between gap-2">
      <span class="text-sm font-medium">Registry capabilities</span>
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
  <ArtifactsStateBanner
    state={viewState}
    itemNoun="registry capabilities"
    onRetry={() => load(engineId, connected, new AbortController().signal)}
  />
{/if}
