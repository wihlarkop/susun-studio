<script lang="ts">
  import * as Dialog from "$lib/components/ui/dialog/index.js";
  import { Button } from "$lib/components/ui/button/index.js";
  import { Input } from "$lib/components/ui/input/index.js";
  import {
    commitEnginePrune,
    previewEnginePrune,
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
  let pruning = $state(false);
  let errorMessage = $state<string | null>(null);
  let report = $state<PruneReport | null>(null);

  const scopes = $derived.by(() => {
    const selected: PruneScope[] = [];
    if (includeContainers) selected.push("containers");
    if (includeNetworks) selected.push("networks");
    if (includeVolumes) selected.push("volumes");
    if (includeImages) selected.push("images");
    return selected;
  });

  async function confirmPrune() {
    pruning = true;
    errorMessage = null;
    report = null;
    try {
      // Two-step: the server mints a single-use plan for this policy, then the
      // engine derives and removes the exact resources at commit.
      const plan = await previewEnginePrune(engineId, scopes, includeImages && includeAllImages);
      report = await commitEnginePrune(plan.plan_id);
      confirmText = "";
    } catch (error) {
      errorMessage = error instanceof Error ? error.message : String(error);
    } finally {
      pruning = false;
    }
  }

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
        variant="destructive"
        disabled={pruning || scopes.length === 0 || confirmText !== "prune"}
        onclick={confirmPrune}
      >
        {pruning ? "Pruning…" : "Prune"}
      </Button>
    </Dialog.Footer>
  </Dialog.Content>
</Dialog.Root>
