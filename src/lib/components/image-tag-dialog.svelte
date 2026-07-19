<script lang="ts">
  import * as Dialog from "$lib/components/ui/dialog/index.js";
  import { Button } from "$lib/components/ui/button/index.js";
  import { Input } from "$lib/components/ui/input/index.js";
  import StatusBadge from "./status-badge.svelte";
  import {
    previewTagImage,
    commitTagImage,
    type ImageArtifactSummary,
    type ImageTagPreview,
    type ImageTagResult,
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

  let targetReference = $state("");
  // The exact reference string that produced the current preview/plan — not
  // necessarily `targetReference`'s live value. Committing must be gated on
  // these still matching: otherwise the user could preview `app:v1`, edit
  // the input to `app:v2` without re-previewing, and Tag would still commit
  // the server-held `app:v1` plan while the input shows `app:v2`.
  let previewedTargetReference = $state<string | null>(null);
  let mutationState = $state<MutationState<ImageTagPreview, ImageTagResult>>(
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
    targetReference = "";
    previewedTargetReference = null;
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
  const inputDiverged = $derived(
    mutationState.phase === "previewed" && targetReference.trim() !== previewedTargetReference,
  );

  async function preview() {
    const reference = targetReference.trim();
    if (!reference) return;
    previewedTargetReference = reference;
    mutationState = startPreviewing(mutationState);
    const requestGeneration = mutationState.generation;
    try {
      const result = await previewTagImage(engineId, image.id, reference);
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
      inputDiverged
    ) {
      return;
    }
    mutationState = startCommitting(mutationState);
    const requestGeneration = mutationState.generation;
    try {
      const result = await commitTagImage(planId);
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
      <Dialog.Title>Tag image</Dialog.Title>
      <Dialog.Description>
        Adds a new reference to <span class="font-mono">{primaryReference}</span> on this engine.
        The image and its other references are left untouched.
      </Dialog.Description>
    </Dialog.Header>

    <label class="flex flex-col gap-1 text-sm">
      <span>New reference (repository:tag)</span>
      <Input
        bind:value={targetReference}
        placeholder="myapp:latest"
        disabled={mutationState.phase === "committing" || mutationState.phase === "succeeded"}
      />
    </label>

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
          <p class="text-muted-foreground">Image tagging isn't supported on this engine.</p>
        {:else if blocker.kind === "active_work"}
          <p class="text-muted-foreground">
            {blocker.jobs} running job(s) and {blocker.watchSessions} watch session(s). Stop them
            and preview again.
          </p>
        {:else}
          <p>
            <span class="font-mono">{mutationState.preview.source_image_id}</span> →
            <span class="font-mono">{mutationState.preview.target_reference}</span>
          </p>
        {/if}
      </div>
    {/if}

    {#if inputDiverged}
      <p class="text-muted-foreground text-sm">
        The reference changed since preview. Preview again before tagging.
      </p>
    {/if}

    {#if mutationState.result}
      <div class="bg-muted/40 rounded-md border p-2 text-sm">
        Tagged as <span class="font-mono">{mutationState.result.target}</span>.
      </div>
    {/if}

    <Dialog.Footer>
      <Button type="button" variant="outline" onclick={() => (open = false)}>Close</Button>
      <Button
        type="button"
        variant="outline"
        disabled={mutationState.phase === "previewing" ||
          mutationState.phase === "committing" ||
          !targetReference.trim()}
        onclick={preview}
      >
        {mutationState.phase === "previewing" ? "Checking…" : "Preview"}
      </Button>
      <Button
        type="button"
        disabled={mutationState.phase !== "previewed" ||
          !mutationState.preview?.commit_enabled ||
          inputDiverged}
        onclick={confirm}
      >
        {mutationState.phase === "committing" ? "Tagging…" : "Tag"}
      </Button>
    </Dialog.Footer>
  </Dialog.Content>
</Dialog.Root>
