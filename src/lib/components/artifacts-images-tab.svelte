<script lang="ts">
  import * as Table from "$lib/components/ui/table/index.js";
  import { Button } from "$lib/components/ui/button/index.js";
  import { RefreshCw } from "@lucide/svelte";
  import StatusBadge from "./status-badge.svelte";
  import ArtifactsStateBanner from "./artifacts-state-banner.svelte";
  import ImageTagDialog from "./image-tag-dialog.svelte";
  import ImageRemoveDialog from "./image-remove-dialog.svelte";
  import ScopePruneControl from "./scope-prune-control.svelte";
  import {
    readEngineImages,
    readEngineImage,
    type EngineImageInventoryResponse,
    type ImageArtifactSummary,
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

  let { engineId, connected }: { engineId: string; connected: boolean } = $props();

  let fetchState = $state(initialScopedFetchState<EngineImageInventoryResponse>());
  let expandedId = $state<string | null>(null);
  let detailCache = $state<Record<string, ScopedDetailEntry<ImageArtifactSummary>>>({});
  let detailController: AbortController | null = null;
  // Plain (non-reactive) counter — see the containers tab for why this must
  // never be read back out of `fetchState` inside the effect below.
  let generation = 0;

  let selectedImage = $state<ImageArtifactSummary | null>(null);
  let tagOpen = $state(false);
  let removeOpen = $state(false);

  function openTag(image: ImageArtifactSummary) {
    selectedImage = image;
    tagOpen = true;
  }

  function openRemove(image: ImageArtifactSummary) {
    selectedImage = image;
    removeOpen = true;
  }

  // Runs only in an async continuation, after the request's first `await` —
  // never synchronously inside the effect.
  async function load(id: string, signal: AbortSignal, requestGeneration: number) {
    try {
      const result = await readEngineImages(id, { signal });
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

  $effect(() => {
    return () => detailController?.abort();
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
      capability: fetchState.data?.capability ?? null,
      itemCount: fetchState.data?.images.length ?? null,
    }),
  );

  async function toggleDetail(id: string, imageId: string) {
    if (expandedId === imageId) {
      expandedId = null;
      return;
    }
    expandedId = imageId;
    const key = scopedDetailKey(id, imageId);
    if (detailCache[key] !== undefined) return;

    detailController?.abort();
    const controller = new AbortController();
    detailController = controller;
    const generation = fetchState.generation;
    try {
      const detail = await readEngineImage(id, imageId, { signal: controller.signal });
      if (controller.signal.aborted || generation !== fetchState.generation) return;
      detailCache = {
        ...detailCache,
        [key]: toDetailEntry({ capability: detail.capability, value: detail.image }),
      };
    } catch (caught) {
      if (controller.signal.aborted || generation !== fetchState.generation) return;
      detailCache = {
        ...detailCache,
        [key]: { kind: "error", message: toArtifactRequestError(caught).message },
      };
    }
  }

  function primaryReference(image: ImageArtifactSummary): string {
    return image.references[0] ?? image.id;
  }
</script>

{#if viewState.kind === "ready" || viewState.kind === "refreshing" || viewState.kind === "stale"}
  <div class="flex flex-col gap-2">
    <div class="flex flex-wrap items-center justify-between gap-2">
      <div class="flex items-center gap-2">
        <p class="text-xs text-muted-foreground">
          {fetchState.data?.images.length ?? 0} image{fetchState.data?.images.length === 1 ? "" : "s"}
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
      <div class="flex items-center gap-2">
        <ScopePruneControl
          {engineId}
          scope="images"
          label="Prune images"
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
        Couldn't refresh ({viewState.error.message}). Showing the last known list.
      </p>
    {/if}

    <Table.Root>
      <Table.Header>
        <Table.Row>
          <Table.Head>Reference</Table.Head>
          <Table.Head>Tags</Table.Head>
          <Table.Head>Created</Table.Head>
          <Table.Head class="text-right">Size</Table.Head>
          <Table.Head class="text-right">Containers</Table.Head>
          <Table.Head class="text-right">Actions</Table.Head>
        </Table.Row>
      </Table.Header>
      <Table.Body>
        {#each fetchState.data?.images ?? [] as image (image.id)}
          <Table.Row class="cursor-pointer" onclick={() => toggleDetail(engineId, image.id)}>
            <Table.Cell class="max-w-64 truncate font-mono text-xs" title={primaryReference(image)}>
              {primaryReference(image)}
            </Table.Cell>
            <Table.Cell class="text-muted-foreground">
              {image.references.length > 1 ? `+${image.references.length - 1} more` : "—"}
            </Table.Cell>
            <Table.Cell class="text-muted-foreground">
              {image.created_at_epoch_seconds
                ? relativeTime(image.created_at_epoch_seconds * 1000)
                : "—"}
            </Table.Cell>
            <Table.Cell class="text-right text-muted-foreground tabular-nums">
              {image.size_bytes !== null ? formatBytes(image.size_bytes) : "—"}
            </Table.Cell>
            <Table.Cell class="text-right text-muted-foreground tabular-nums">
              {image.container_count ?? "—"}
            </Table.Cell>
            <Table.Cell class="text-right">
              <div class="flex justify-end gap-1">
                <Button
                  size="sm"
                  variant="outline"
                  onclick={(event) => {
                    event.stopPropagation();
                    openTag(image);
                  }}
                >
                  Tag
                </Button>
                <Button
                  size="sm"
                  variant="outline"
                  onclick={(event) => {
                    event.stopPropagation();
                    openRemove(image);
                  }}
                >
                  Remove
                </Button>
              </div>
            </Table.Cell>
          </Table.Row>
          {#if expandedId === image.id}
            <Table.Row>
              <Table.Cell colspan={6} class="bg-muted/40 whitespace-normal">
                {@const entry = detailCache[scopedDetailKey(engineId, image.id)]}
                <div class="flex flex-col gap-2 py-1 text-xs">
                  {#if entry === undefined}
                    <p class="text-muted-foreground">Loading detail…</p>
                  {:else if entry.kind === "error"}
                    <p class="text-destructive">{entry.message}</p>
                  {:else if entry.kind === "unsupported"}
                    <div class="flex items-center gap-2">
                      <StatusBadge status={entry.capability} label={capabilityLabel(entry.capability)} />
                      <span class="text-muted-foreground">
                        Detail isn't available for this image on this engine.
                      </span>
                    </div>
                  {:else}
                    <StatusBadge status={entry.capability} label={capabilityLabel(entry.capability)} />
                    <div class="grid grid-cols-1 gap-x-6 gap-y-2 md:grid-cols-2">
                      <div>
                        <span class="text-muted-foreground">Image id</span><br />
                        <span class="font-mono">{entry.value.id}</span>
                      </div>
                      <div>
                        <span class="text-muted-foreground">Digests</span><br />
                        {#if entry.value.digests.length > 0}
                          {#each entry.value.digests as digest (digest)}
                            <div class="truncate font-mono" title={digest}>{digest}</div>
                          {/each}
                        {:else}
                          none
                        {/if}
                      </div>
                      <div>
                        <span class="text-muted-foreground">All references</span><br />
                        {entry.value.references.length > 0 ? entry.value.references.join(", ") : "none"}
                      </div>
                      <div>
                        <span class="text-muted-foreground">Shared size</span><br />
                        {entry.value.shared_size_bytes !== null
                          ? formatBytes(entry.value.shared_size_bytes)
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
  <ArtifactsStateBanner state={viewState} itemNoun="images" onRetry={refresh} />
{/if}

{#if selectedImage}
  <ImageTagDialog {engineId} image={selectedImage} bind:open={tagOpen} oncompleted={refresh} />
  <ImageRemoveDialog {engineId} image={selectedImage} bind:open={removeOpen} oncompleted={refresh} />
{/if}
