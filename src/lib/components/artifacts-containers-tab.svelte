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
  import { capabilityLabel } from "$lib/artifacts/capability";
  import {
    applyLoadError,
    applyLoadSuccess,
    initialScopedFetchState,
    resetForNewEngine,
    withLoading,
  } from "$lib/artifacts/scoped-fetch";
  import { scopedDetailKey, toDetailEntry, type ScopedDetailEntry } from "$lib/artifacts/scoped-detail";
  import { formatBytes, relativeTime } from "$lib/utils";

  let {
    engineId,
    connected,
    projects,
  }: { engineId: string; connected: boolean; projects: StudioProject[] } = $props();

  function projectName(projectId: string): string {
    return projects.find((project) => project.id === projectId)?.name ?? projectId;
  }

  let fetchState = $state(initialScopedFetchState<EngineContainerInventoryResponse>());
  let expandedId = $state<string | null>(null);
  let detailCache = $state<Record<string, ScopedDetailEntry<ContainerArtifactSummary>>>({});
  let detailController: AbortController | null = null;
  // Plain (non-reactive) counter: bumped synchronously inside the effect
  // below and never read back out of `fetchState` there, so the effect's
  // only tracked dependencies stay `engineId`/`connected`. See
  // `resetForNewEngine`'s doc comment for why reading `fetchState` inside
  // the same effect that assigns it would self-trigger indefinitely.
  let generation = 0;

  // Runs only in an async continuation, after the request's first `await` —
  // never synchronously inside the effect — so its `fetchState` reads/writes
  // are plain reactive updates, not effect dependencies.
  async function load(id: string, signal: AbortSignal, requestGeneration: number) {
    try {
      const result = await readEngineContainers(id, { signal });
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

    // Engine changed (or this is the first run): every piece of state
    // scoped to the previous engine is cleared synchronously, before the
    // new engine's data is requested, so the old engine's containers,
    // errors, expanded row, and cached detail can never render underneath
    // the new engine's header. The next state is built from the plain
    // counter, not read back from `fetchState` — see `generation` above.
    generation += 1;
    const myGeneration = generation;
    detailController?.abort();
    detailController = null;
    detailCache = {};
    expandedId = null;

    const controller = new AbortController();
    fetchState = resetForNewEngine(myGeneration, isConnected);
    if (isConnected) {
      void load(id, controller.signal, myGeneration);
    }
    return () => controller.abort();
  });

  // Detail requests are triggered by row clicks, not the effect above, so
  // they need their own cleanup hook to be aborted on component teardown.
  $effect(() => {
    return () => detailController?.abort();
  });

  // Safe to read/write `fetchState` here: this only ever runs from a
  // button click, never synchronously inside an `$effect`.
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
      capability: fetchState.data?.capability ?? null,
      itemCount: fetchState.data?.containers.length ?? null,
    }),
  );

  async function toggleDetail(id: string, containerId: string) {
    if (expandedId === containerId) {
      expandedId = null;
      return;
    }
    expandedId = containerId;
    const key = scopedDetailKey(id, containerId);
    if (detailCache[key] !== undefined) return;

    detailController?.abort();
    const controller = new AbortController();
    detailController = controller;
    const generation = fetchState.generation;
    try {
      const detail = await readEngineContainer(id, containerId, { signal: controller.signal });
      if (controller.signal.aborted || generation !== fetchState.generation) return;
      detailCache = {
        ...detailCache,
        [key]: toDetailEntry({ capability: detail.capability, value: detail.container }),
      };
    } catch (caught) {
      if (controller.signal.aborted || generation !== fetchState.generation) return;
      detailCache = {
        ...detailCache,
        [key]: { kind: "error", message: toArtifactRequestError(caught).message },
      };
    }
  }
</script>

{#if viewState.kind === "ready" || viewState.kind === "refreshing" || viewState.kind === "stale"}
  <div class="flex flex-col gap-2">
    <div class="flex flex-wrap items-center justify-between gap-2">
      <div class="flex items-center gap-2">
        <p class="text-xs text-muted-foreground">
          {fetchState.data?.containers.length ?? 0} container{fetchState.data?.containers.length === 1
            ? ""
            : "s"}
          {#if fetchState.data?.observed_at_epoch_seconds}
            · observed {relativeTime(fetchState.data.observed_at_epoch_seconds * 1000)}
          {/if}
        </p>
        {#if fetchState.data}
          <StatusBadge
            status={fetchState.data.capability}
            label={capabilityLabel(fetchState.data.capability)}
          />
        {/if}
      </div>
      <Button size="sm" variant="outline" disabled={fetchState.loading} onclick={refresh}>
        <RefreshCw class={fetchState.loading ? "animate-spin" : undefined} />
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
        {#each fetchState.data?.containers ?? [] as container (container.id)}
          <Table.Row class="cursor-pointer" onclick={() => toggleDetail(engineId, container.id)}>
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
            <Table.Cell
              class="max-w-56 truncate text-muted-foreground"
              title={container.image_reference ?? undefined}
            >
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
                {@const entry = detailCache[scopedDetailKey(engineId, container.id)]}
                <div class="flex flex-col gap-2 py-1 text-xs">
                  {#if entry === undefined}
                    <p class="text-muted-foreground">Loading detail…</p>
                  {:else if entry.kind === "error"}
                    <p class="text-destructive">{entry.message}</p>
                  {:else if entry.kind === "unsupported"}
                    <div class="flex items-center gap-2">
                      <StatusBadge status={entry.capability} label={capabilityLabel(entry.capability)} />
                      <span class="text-muted-foreground">
                        Detail isn't available for this container on this engine.
                      </span>
                    </div>
                  {:else}
                    <StatusBadge status={entry.capability} label={capabilityLabel(entry.capability)} />
                    <div class="grid grid-cols-2 gap-x-6 gap-y-1 md:grid-cols-3">
                      <div>
                        <span class="text-muted-foreground">Container id</span><br />
                        <span class="font-mono">{entry.value.id}</span>
                      </div>
                      <div>
                        <span class="text-muted-foreground">Root filesystem</span><br />
                        {entry.value.root_filesystem_size_bytes !== null
                          ? formatBytes(entry.value.root_filesystem_size_bytes)
                          : "—"}
                      </div>
                      <div>
                        <span class="text-muted-foreground">Label keys</span><br />
                        {entry.value.label_keys.length > 0 ? entry.value.label_keys.join(", ") : "none"}
                      </div>
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
  <ArtifactsStateBanner state={viewState} itemNoun="containers" onRetry={refresh} />
{/if}
