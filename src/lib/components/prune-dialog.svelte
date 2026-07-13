<script lang="ts">
  import * as Dialog from "$lib/components/ui/dialog/index.js";
  import { Button } from "$lib/components/ui/button/index.js";
  import { Input } from "$lib/components/ui/input/index.js";
  import {
    commitEnginePrune,
    previewEnginePrune,
    type PrunePreview,
    type PruneReport,
    type PruneScope,
  } from "$lib/daemon/client";

  let {
    engineId,
    open = $bindable(false),
  }: {
    engineId: string;
    open?: boolean;
  } = $props();

  let includeContainers = $state(true);
  let includeNetworks = $state(true);
  let includeImages = $state(true);
  let includeAllImages = $state(false);
  let includeVolumes = $state(false);
  let confirmText = $state("");
  let previewing = $state(false);
  let pruning = $state(false);
  let errorMessage = $state<string | null>(null);
  let preview = $state<PrunePreview | null>(null);
  let report = $state<PruneReport | null>(null);

  const scopes = $derived.by(() => {
    const selected: PruneScope[] = [];
    if (includeContainers) selected.push("containers");
    if (includeNetworks) selected.push("networks");
    if (includeVolumes) selected.push("volumes");
    if (includeImages) selected.push("images");
    return selected;
  });

  async function buildPreview() {
    previewing = true;
    errorMessage = null;
    report = null;
    preview = null;
    try {
      // The server derives a non-destructive inventory and, when safe, mints a
      // single-use commit plan bound to the engine identity and that inventory.
      preview = await previewEnginePrune(engineId, scopes, includeImages && includeAllImages);
    } catch (error) {
      errorMessage = error instanceof Error ? error.message : String(error);
    } finally {
      previewing = false;
    }
  }

  async function confirmPrune() {
    if (!preview?.commit_enabled || !preview.plan_id) return;
    pruning = true;
    errorMessage = null;
    try {
      report = await commitEnginePrune(preview.plan_id);
      confirmText = "";
      preview = null;
    } catch (error) {
      errorMessage = error instanceof Error ? error.message : String(error);
    } finally {
      pruning = false;
    }
  }

  function formatBytesMaybe(value: number | null): string {
    return value === null ? "unknown" : formatBytes(value);
  }

  // A preview is bound to a specific policy; drop it when the policy changes so a
  // stale plan is never committed.
  $effect(() => {
    void scopes;
    void includeAllImages;
    preview = null;
  });

  function totalRemoved(value: PruneReport): number {
    return (
      value.containers_removed.length +
      value.networks_removed.length +
      value.volumes_removed.length +
      value.images_removed.length
    );
  }

  function formatBytes(bytes: number): string {
    if (bytes < 1024) return `${bytes} B`;
    const units = ["KB", "MB", "GB", "TB"];
    let value = bytes / 1024;
    let unitIndex = 0;
    while (value >= 1024 && unitIndex < units.length - 1) {
      value /= 1024;
      unitIndex += 1;
    }
    return `${value.toFixed(1)} ${units[unitIndex]}`;
  }
</script>

<Dialog.Root bind:open>
  <Dialog.Content class="sm:max-w-lg">
    <Dialog.Header>
      <Dialog.Title>Prune Docker system</Dialog.Title>
      <Dialog.Description>
        Removes unused resources across the <b>whole Docker engine</b> — not just projects
        tracked by Studio. This can affect other tools and projects. Volumes may contain data
        (e.g. database contents) and are excluded by default.
      </Dialog.Description>
    </Dialog.Header>

    <div class="flex flex-col gap-2 text-sm">
      <label class="flex items-center gap-2">
        <input type="checkbox" bind:checked={includeContainers} />
        Stopped containers
      </label>
      <label class="flex items-center gap-2">
        <input type="checkbox" bind:checked={includeNetworks} />
        Unused networks
      </label>
      <label class="flex items-center gap-2">
        <input type="checkbox" bind:checked={includeImages} />
        Dangling images
      </label>
      <label class="ml-6 flex items-center gap-2 {includeImages ? '' : 'text-muted-foreground'}">
        <input type="checkbox" bind:checked={includeAllImages} disabled={!includeImages} />
        Also remove unused tagged images
      </label>
      <label class="flex items-center gap-2">
        <input type="checkbox" bind:checked={includeVolumes} />
        <span>Unused volumes <b class="text-destructive">(deletes data)</b></span>
      </label>
    </div>

    <label class="flex flex-col gap-1 text-sm">
      <span>Type <b class="font-mono">prune</b> to confirm</span>
      <Input bind:value={confirmText} placeholder="prune" />
    </label>

    {#if errorMessage}
      <p class="text-destructive text-sm">{errorMessage}</p>
    {/if}

    {#if preview}
      <div class="grid gap-2 rounded-md border p-3 text-sm">
        {#if preview.active_jobs > 0 || preview.active_watch_sessions > 0}
          <p class="text-destructive">
            {preview.active_jobs} running job(s) and {preview.active_watch_sessions} watch session(s).
            Stop them before pruning.
          </p>
        {/if}
        {#if !preview.inventory_supported}
          <p class="text-muted-foreground">
            This engine can't report a prune inventory, so prune is disabled here.
          </p>
        {:else}
          <div class="grid gap-1">
            {#each preview.inventory as item (item.scope)}
              <div class="flex flex-wrap items-center justify-between gap-2">
                <span>{item.scope.replaceAll("_", " ")}</span>
                <span class="text-muted-foreground">
                  {item.candidate_count ?? "?"} candidate(s) · {formatBytesMaybe(
                    item.reclaimable_bytes,
                  )}
                  {item.estimate_kind === "exact" ? "" : `(${item.estimate_kind.replaceAll("_", " ")})`}
                </span>
              </div>
            {/each}
          </div>
          <p class="text-muted-foreground">
            Estimated reclaim: {formatBytesMaybe(preview.estimated_reclaim_bytes)}.
          </p>
        {/if}
      </div>
    {/if}

    {#if report}
      <div class="bg-muted/40 rounded-md border p-2 text-sm">
        <p>Removed {totalRemoved(report)} resources.</p>
        <p class="text-muted-foreground">
          Reclaimed {formatBytes(report.space_reclaimed_bytes)}.
        </p>
      </div>
    {/if}

    <Dialog.Footer>
      <Button type="button" variant="outline" onclick={() => (open = false)}>Close</Button>
      <Button
        type="button"
        variant="outline"
        disabled={previewing || scopes.length === 0}
        onclick={buildPreview}
      >
        {previewing ? "Checking…" : "Preview"}
      </Button>
      <Button
        type="button"
        variant="destructive"
        disabled={pruning || !preview?.commit_enabled || confirmText !== "prune"}
        onclick={confirmPrune}
      >
        {pruning ? "Pruning…" : "Prune"}
      </Button>
    </Dialog.Footer>
  </Dialog.Content>
</Dialog.Root>
