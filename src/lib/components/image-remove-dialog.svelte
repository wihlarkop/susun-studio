<script lang="ts">
  import * as Dialog from "$lib/components/ui/dialog/index.js";
  import { Button } from "$lib/components/ui/button/index.js";
  import { Input } from "$lib/components/ui/input/index.js";
  import StatusBadge from "./status-badge.svelte";
  import {
    previewRemoveImage,
    commitRemoveImage,
    type ImageArtifactSummary,
    type ImageRemovePreview,
    type ImageRemoveResult,
  } from "$lib/daemon/client";
  import { toArtifactRequestError } from "$lib/artifacts/fetch-error";
  import { capabilityLabel, isCapabilityUsable } from "$lib/artifacts/capability";
  import {
    applyCommitError,
    applyCommitSuccess,
    applyPreviewError,
    applyPreviewSuccess,
    describeMutationBlocker,
    resetMutation,
    startCommitting,
    startPreviewing,
    type MutationState,
  } from "$lib/artifacts/mutation-state";
  import { formatBytes } from "$lib/utils";

  let {
    engineId,
    image,
    open = $bindable(false),
    oncompleted,
  }: {
    engineId: string;
    image: ImageArtifactSummary;
    open?: boolean;
    oncompleted?: () => void | Promise<void>;
  } = $props();

  let confirmText = $state("");
  let mutationState = $state<MutationState<ImageRemovePreview, ImageRemoveResult>>(
    resetMutation(0),
  );
  // Plain (non-reactive) counter — see the artifacts tabs for why this must
  // never be read back out of `mutationState` inside the effect below.
  let generation = 0;

  // Re-arms the dialog whenever it opens, the target image changes, or the
  // engine changes, so a preview/commit response for a row (or engine) the
  // user has since navigated away from can never be applied to this one.
  $effect(() => {
    const isOpen = open;
    const id = image.id;
    const engine = engineId;
    generation += 1;
    mutationState = resetMutation(generation);
    confirmText = "";
    void isOpen;
    void id;
    void engine;
  });

  const primaryReference = $derived(image.references[0] ?? image.id);
  const blocker = $derived(
    mutationState.preview
      ? describeMutationBlocker({
          supported: isCapabilityUsable(mutationState.preview.capability),
          commitEnabled: mutationState.preview.commit_enabled,
          activeJobs: mutationState.preview.active_jobs,
          activeWatchSessions: mutationState.preview.active_watch_sessions,
        })
      : null,
  );

  async function preview() {
    mutationState = startPreviewing(mutationState);
    const requestGeneration = mutationState.generation;
    try {
      const result = await previewRemoveImage(engineId, image.id);
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
    if (
      mutationState.phase !== "previewed" ||
      !mutationState.preview?.commit_enabled ||
      !planId ||
      confirmText !== "remove"
    ) {
      return;
    }
    mutationState = startCommitting(mutationState);
    const requestGeneration = mutationState.generation;
    try {
      const result = await commitRemoveImage(planId);
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
</script>

<Dialog.Root bind:open>
  <Dialog.Content class="sm:max-w-lg">
    <Dialog.Header>
      <Dialog.Title>Remove image</Dialog.Title>
      <Dialog.Description>
        Removes <span class="font-mono">{primaryReference}</span> from this engine. This can affect
        other tools and projects sharing it, and cannot be undone.
      </Dialog.Description>
    </Dialog.Header>

    {#if mutationState.error}
      <p class="text-destructive text-sm">{mutationState.error.message}</p>
    {/if}

    {#if mutationState.preview && blocker}
      <div class="grid gap-2 rounded-md border p-3 text-sm">
        <StatusBadge
          status={mutationState.preview.capability}
          label={capabilityLabel(mutationState.preview.capability)}
        />
        {#if blocker.kind === "unsupported"}
          <p class="text-muted-foreground">Image removal isn't supported on this engine.</p>
        {:else if blocker.kind === "active_work"}
          <p class="text-muted-foreground">
            {blocker.jobs} running job(s) and {blocker.watchSessions} watch session(s). Stop them
            and preview again.
          </p>
        {:else}
          <div>
            <span class="text-muted-foreground">References</span><br />
            {mutationState.preview.references.length > 0
              ? mutationState.preview.references.join(", ")
              : "none"}
          </div>
          <p class="text-muted-foreground">
            Estimated reclaim: {mutationState.preview.estimated_reclaim_bytes !== null
              ? formatBytes(mutationState.preview.estimated_reclaim_bytes)
              : "unknown"}.
          </p>
        {/if}
      </div>
    {/if}

    {#if mutationState.result}
      <div class="bg-muted/40 rounded-md border p-2 text-sm">
        Removed {mutationState.result.deleted.length} image(s).
      </div>
    {:else}
      <label class="flex flex-col gap-1 text-sm">
        <span>Type <b class="font-mono">remove</b> to confirm</span>
        <Input
          bind:value={confirmText}
          placeholder="remove"
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
          confirmText !== "remove"}
        onclick={confirm}
      >
        {mutationState.phase === "committing" ? "Removing…" : "Remove"}
      </Button>
    </Dialog.Footer>
  </Dialog.Content>
</Dialog.Root>
