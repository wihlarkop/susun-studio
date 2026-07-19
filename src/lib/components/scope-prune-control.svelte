<script lang="ts">
  import * as Dialog from "$lib/components/ui/dialog/index.js";
  import { Button } from "$lib/components/ui/button/index.js";
  import { Input } from "$lib/components/ui/input/index.js";
  import {
    previewEnginePrune,
    commitEnginePrune,
    type PruneReport,
    type PrunePreview,
    type PruneScope,
  } from "$lib/daemon/client";
  import { toArtifactRequestError } from "$lib/artifacts/fetch-error";
  import {
    applyCommitError,
    applyCommitSuccess,
    applyPreviewError,
    applyPreviewSuccess,
    resetMutation,
    startCommitting,
    startPreviewing,
    type MutationState,
  } from "$lib/artifacts/mutation-state";
  import { formatBytes } from "$lib/utils";

  let {
    engineId,
    scope,
    label,
    disabled = false,
    oncompleted,
  }: {
    engineId: string;
    /** Reuses the same trusted prune backend as the Runtime page's
     * multi-scope prune dialog, scoped down to exactly one resource class so
     * it fits the tab it's embedded in. */
    scope: PruneScope;
    label: string;
    disabled?: boolean;
    oncompleted?: () => void | Promise<void>;
  } = $props();

  let open = $state(false);
  let confirmText = $state("");
  let mutationState = $state<MutationState<PrunePreview, PruneReport>>(resetMutation(0));
  // Plain (non-reactive) counter — see the artifacts tabs for why this must
  // never be read back out of `mutationState` inside the effect below.
  let generation = 0;

  $effect(() => {
    const isOpen = open;
    generation += 1;
    mutationState = resetMutation(generation);
    confirmText = "";
    void isOpen;
  });

  async function preview() {
    mutationState = startPreviewing(mutationState);
    const requestGeneration = mutationState.generation;
    try {
      const result = await previewEnginePrune(engineId, [scope], false);
      mutationState = applyPreviewSuccess(mutationState, requestGeneration, result);
    } catch (caught) {
      mutationState = applyPreviewError(
        mutationState,
        requestGeneration,
        toArtifactRequestError(caught),
      );
    }
  }

  async function confirm() {
    const planId = mutationState.preview?.plan_id;
    if (!mutationState.preview?.commit_enabled || !planId || confirmText !== "prune") return;
    mutationState = startCommitting(mutationState);
    const requestGeneration = mutationState.generation;
    try {
      const result = await commitEnginePrune(planId);
      mutationState = applyCommitSuccess(mutationState, requestGeneration, result);
      await oncompleted?.();
    } catch (caught) {
      mutationState = applyCommitError(
        mutationState,
        requestGeneration,
        toArtifactRequestError(caught),
      );
    }
  }

  function totalRemoved(report: PruneReport): number {
    return (
      report.containers_removed.length +
      report.networks_removed.length +
      report.volumes_removed.length +
      report.images_removed.length
    );
  }

  const scopeNoun = $derived(scope === "build_cache" ? "build cache" : scope);
</script>

<Button size="sm" variant="outline" {disabled} onclick={() => (open = true)}>
  {label}
</Button>

<Dialog.Root bind:open>
  <Dialog.Content class="sm:max-w-lg">
    <Dialog.Header>
      <Dialog.Title>{label}</Dialog.Title>
      <Dialog.Description>
        Removes unused {scopeNoun} from this engine, across all tools and projects sharing it —
        not only ones tracked by Studio.
      </Dialog.Description>
    </Dialog.Header>

    {#if mutationState.error}
      <p class="text-destructive text-sm">{mutationState.error.message}</p>
    {/if}

    {#if mutationState.preview}
      <div class="grid gap-2 rounded-md border p-3 text-sm">
        {#if mutationState.preview.active_jobs > 0 || mutationState.preview.active_watch_sessions > 0}
          <p class="text-destructive">
            {mutationState.preview.active_jobs} running job(s) and {mutationState.preview
              .active_watch_sessions} watch session(s). Stop them before pruning.
          </p>
        {/if}
        {#if !mutationState.preview.inventory_supported}
          <p class="text-muted-foreground">
            This engine can't report a prune inventory, so prune is disabled here.
          </p>
        {:else}
          <div class="grid gap-1">
            {#each mutationState.preview.inventory as item (item.scope)}
              <div class="flex flex-wrap items-center justify-between gap-2">
                <span>{item.scope.replaceAll("_", " ")}</span>
                <span class="text-muted-foreground">
                  {item.candidate_count ?? "?"} candidate(s) · {item.reclaimable_bytes !== null
                    ? formatBytes(item.reclaimable_bytes)
                    : "unknown"}
                </span>
              </div>
            {/each}
          </div>
        {/if}
      </div>
    {/if}

    {#if mutationState.result}
      <div class="bg-muted/40 rounded-md border p-2 text-sm">
        <p>Removed {totalRemoved(mutationState.result)} resource(s).</p>
        <p class="text-muted-foreground">
          Reclaimed {formatBytes(mutationState.result.space_reclaimed_bytes)}.
        </p>
      </div>
    {:else}
      <label class="flex flex-col gap-1 text-sm">
        <span>Type <b class="font-mono">prune</b> to confirm</span>
        <Input
          bind:value={confirmText}
          placeholder="prune"
          disabled={mutationState.phase === "committing"}
        />
      </label>
    {/if}

    <Dialog.Footer>
      <Button type="button" variant="outline" onclick={() => (open = false)}>Close</Button>
      <Button
        type="button"
        variant="outline"
        disabled={mutationState.phase === "previewing" || mutationState.phase === "committing"}
        onclick={preview}
      >
        {mutationState.phase === "previewing" ? "Checking…" : "Preview"}
      </Button>
      <Button
        type="button"
        variant="destructive"
        disabled={mutationState.phase !== "previewed" ||
          !mutationState.preview?.commit_enabled ||
          confirmText !== "prune"}
        onclick={confirm}
      >
        {mutationState.phase === "committing" ? "Pruning…" : "Prune"}
      </Button>
    </Dialog.Footer>
  </Dialog.Content>
</Dialog.Root>
