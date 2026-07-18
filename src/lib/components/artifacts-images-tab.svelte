<script lang="ts">
  import * as Table from "$lib/components/ui/table/index.js";
  import { Button } from "$lib/components/ui/button/index.js";
  import { RefreshCw } from "@lucide/svelte";
  import ArtifactsStateBanner from "./artifacts-state-banner.svelte";
  import {
    readEngineImages,
    readEngineImage,
    type ImageArtifactSummary,
    type EngineImageInventoryResponse,
  } from "$lib/daemon/client";
  import { resolveArtifactViewState } from "$lib/artifacts/workspace-state";
  import { toArtifactRequestError } from "$lib/artifacts/fetch-error";
  import { formatBytes, relativeTime } from "$lib/utils";
  import type { ArtifactRequestError } from "$lib/artifacts/workspace-state";

  let { engineId, connected }: { engineId: string; connected: boolean } = $props();

  let response = $state<EngineImageInventoryResponse | null>(null);
  let loading = $state(false);
  let error = $state<ArtifactRequestError | null>(null);
  let expandedId = $state<string | null>(null);
  let detailCache = $state<Record<string, ImageArtifactSummary | "unsupported">>({});
  let detailError = $state<string | null>(null);

  async function load(id: string, isConnected: boolean, signal: AbortSignal) {
    if (!isConnected) {
      loading = false;
      return;
    }
    loading = true;
    try {
      const result = await readEngineImages(id, { signal });
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
      itemCount: response?.images.length ?? null,
    }),
  );

  async function toggleDetail(imageId: string) {
    if (expandedId === imageId) {
      expandedId = null;
      return;
    }
    expandedId = imageId;
    detailError = null;
    if (detailCache[imageId] !== undefined) return;
    try {
      const detail = await readEngineImage(engineId, imageId);
      detailCache = {
        ...detailCache,
        [imageId]: detail.image ?? "unsupported",
      };
    } catch (caught) {
      detailError = toArtifactRequestError(caught).message;
    }
  }

  function primaryReference(image: ImageArtifactSummary): string {
    return image.references[0] ?? image.id;
  }
</script>

{#if viewState.kind === "ready" || viewState.kind === "refreshing" || viewState.kind === "stale"}
  <div class="flex flex-col gap-2">
    <div class="flex items-center justify-between gap-2">
      <p class="text-xs text-muted-foreground">
        {response?.images.length ?? 0} image{response?.images.length === 1 ? "" : "s"}
        {#if response?.observed_at_epoch_seconds}
          · observed {relativeTime(response.observed_at_epoch_seconds * 1000)}
        {/if}
      </p>
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
        </Table.Row>
      </Table.Header>
      <Table.Body>
        {#each response?.images ?? [] as image (image.id)}
          <Table.Row class="cursor-pointer" onclick={() => toggleDetail(image.id)}>
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
          </Table.Row>
          {#if expandedId === image.id}
            <Table.Row>
              <Table.Cell colspan={5} class="bg-muted/40 whitespace-normal">
                {@const detail = detailCache[image.id]}
                <div class="flex flex-col gap-2 py-1 text-xs">
                  {#if detailError}
                    <p class="text-destructive">{detailError}</p>
                  {:else if detail === undefined}
                    <p class="text-muted-foreground">Loading detail…</p>
                  {:else if detail === "unsupported"}
                    <p class="text-muted-foreground">
                      Detail isn't available for this image on this engine.
                    </p>
                  {:else}
                    <div class="grid grid-cols-1 gap-x-6 gap-y-2 md:grid-cols-2">
                      <div>
                        <span class="text-muted-foreground">Image id</span><br />
                        <span class="font-mono">{detail.id}</span>
                      </div>
                      <div>
                        <span class="text-muted-foreground">Digests</span><br />
                        {#if detail.digests.length > 0}
                          {#each detail.digests as digest (digest)}
                            <div class="truncate font-mono" title={digest}>{digest}</div>
                          {/each}
                        {:else}
                          none
                        {/if}
                      </div>
                      <div>
                        <span class="text-muted-foreground">All references</span><br />
                        {detail.references.length > 0 ? detail.references.join(", ") : "none"}
                      </div>
                      <div>
                        <span class="text-muted-foreground">Shared size</span><br />
                        {detail.shared_size_bytes !== null
                          ? formatBytes(detail.shared_size_bytes)
                          : "—"}
                      </div>
                      <div>
                        <span class="text-muted-foreground">Label keys</span><br />
                        {detail.label_keys.length > 0 ? detail.label_keys.join(", ") : "none"}
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
  <ArtifactsStateBanner
    state={viewState}
    itemNoun="images"
    onRetry={() => load(engineId, connected, new AbortController().signal)}
  />
{/if}
