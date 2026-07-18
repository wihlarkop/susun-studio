<script lang="ts">
  import * as Table from "$lib/components/ui/table/index.js";
  import { Button } from "$lib/components/ui/button/index.js";
  import { RefreshCw } from "@lucide/svelte";
  import StatusBadge from "./status-badge.svelte";
  import ArtifactsStateBanner from "./artifacts-state-banner.svelte";
  import {
    readEngineContainers,
    readEngineContainer,
    type ContainerArtifactSummary,
    type EngineContainerInventoryResponse,
    type StudioProject,
  } from "$lib/daemon/client";
  import { resolveArtifactViewState } from "$lib/artifacts/workspace-state";
  import { toArtifactRequestError } from "$lib/artifacts/fetch-error";
  import { formatBytes, relativeTime } from "$lib/utils";
  import type { ArtifactRequestError } from "$lib/artifacts/workspace-state";

  let {
    engineId,
    connected,
    projects,
  }: { engineId: string; connected: boolean; projects: StudioProject[] } = $props();

  function projectName(projectId: string): string {
    return projects.find((project) => project.id === projectId)?.name ?? projectId;
  }

  let response = $state<EngineContainerInventoryResponse | null>(null);
  let loading = $state(false);
  let error = $state<ArtifactRequestError | null>(null);
  let expandedId = $state<string | null>(null);
  let detailCache = $state<Record<string, ContainerArtifactSummary | "unsupported">>({});
  let detailError = $state<string | null>(null);

  async function load(id: string, isConnected: boolean, signal: AbortSignal) {
    if (!isConnected) {
      loading = false;
      return;
    }
    loading = true;
    try {
      const result = await readEngineContainers(id, { signal });
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
      capability: response?.capability ?? null,
      itemCount: response?.containers.length ?? null,
    }),
  );

  async function toggleDetail(containerId: string) {
    if (expandedId === containerId) {
      expandedId = null;
      return;
    }
    expandedId = containerId;
    detailError = null;
    if (detailCache[containerId] !== undefined) return;
    try {
      const detail = await readEngineContainer(engineId, containerId);
      detailCache = {
        ...detailCache,
        [containerId]: detail.container ?? "unsupported",
      };
    } catch (caught) {
      detailError = toArtifactRequestError(caught).message;
    }
  }
</script>

{#if viewState.kind === "ready" || viewState.kind === "refreshing" || viewState.kind === "stale"}
  <div class="flex flex-col gap-2">
    <div class="flex items-center justify-between gap-2">
      <p class="text-xs text-muted-foreground">
        {response?.containers.length ?? 0} container{response?.containers.length === 1 ? "" : "s"}
        {#if response?.observed_at_epoch_seconds}
          · observed {relativeTime(response.observed_at_epoch_seconds * 1000)}
        {/if}
      </p>
      <Button size="sm" variant="outline" disabled={loading} onclick={() => load(engineId, connected, new AbortController().signal)}>
        <RefreshCw class={loading ? "animate-spin" : undefined} />
        Refresh
      </Button>
    </div>

    {#if viewState.kind === "stale"}
      <p class="text-xs text-destructive">
        Couldn't refresh ({viewState.error.message}). Showing the last known list.
      </p>
    {/if}

    <Table.Root>
      <Table.Header>
        <Table.Row>
          <Table.Head>Name</Table.Head>
          <Table.Head>State</Table.Head>
          <Table.Head>Image</Table.Head>
          <Table.Head>Project</Table.Head>
          <Table.Head>Created</Table.Head>
          <Table.Head class="text-right">Size</Table.Head>
        </Table.Row>
      </Table.Header>
      <Table.Body>
        {#each response?.containers ?? [] as container (container.id)}
          <Table.Row class="cursor-pointer" onclick={() => toggleDetail(container.id)}>
            <Table.Cell class="max-w-48 truncate font-medium" title={container.name}>
              {container.name}
            </Table.Cell>
            <Table.Cell>
              <div class="flex items-center gap-1.5">
                <StatusBadge status={container.state} />
                {#if container.health}
                  <StatusBadge status={container.health} />
                {/if}
              </div>
            </Table.Cell>
            <Table.Cell class="max-w-56 truncate text-muted-foreground" title={container.image_reference ?? undefined}>
              {container.image_reference ?? "—"}
            </Table.Cell>
            <Table.Cell class="max-w-40 truncate text-muted-foreground">
              {container.known_project_id ? projectName(container.known_project_id) : "—"}
            </Table.Cell>
            <Table.Cell class="text-muted-foreground">
              {container.created_at_epoch_seconds
                ? relativeTime(container.created_at_epoch_seconds * 1000)
                : "—"}
            </Table.Cell>
            <Table.Cell class="text-right text-muted-foreground tabular-nums">
              {container.writable_size_bytes !== null
                ? formatBytes(container.writable_size_bytes)
                : "—"}
            </Table.Cell>
          </Table.Row>
          {#if expandedId === container.id}
            <Table.Row>
              <Table.Cell colspan={6} class="bg-muted/40 whitespace-normal">
                {@const detail = detailCache[container.id]}
                <div class="flex flex-col gap-2 py-1 text-xs">
                  {#if detailError}
                    <p class="text-destructive">{detailError}</p>
                  {:else if detail === undefined}
                    <p class="text-muted-foreground">Loading detail…</p>
                  {:else if detail === "unsupported"}
                    <p class="text-muted-foreground">
                      Detail isn't available for this container on this engine.
                    </p>
                  {:else}
                    <div class="grid grid-cols-2 gap-x-6 gap-y-1 md:grid-cols-3">
                      <div><span class="text-muted-foreground">Container id</span><br />
                        <span class="font-mono">{detail.id}</span></div>
                      <div><span class="text-muted-foreground">Root filesystem</span><br />
                        {detail.root_filesystem_size_bytes !== null
                          ? formatBytes(detail.root_filesystem_size_bytes)
                          : "—"}</div>
                      <div><span class="text-muted-foreground">Label keys</span><br />
                        {detail.label_keys.length > 0 ? detail.label_keys.join(", ") : "none"}</div>
                    </div>
                  {/if}
                </div>
              </Table.Cell>
            </Table.Row>
          {/if}
        {/each}
      </Table.Body>
    </Table.Root>
  </div>
{:else}
  <ArtifactsStateBanner
    state={viewState}
    itemNoun="containers"
    onRetry={() => load(engineId, connected, new AbortController().signal)}
  />
{/if}
