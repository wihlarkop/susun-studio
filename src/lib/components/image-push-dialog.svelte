<script lang="ts">
  import * as Dialog from "$lib/components/ui/dialog/index.js";
  import { Button } from "$lib/components/ui/button/index.js";
  import { Input } from "$lib/components/ui/input/index.js";
  import {
    cancelJob,
    commitImagePush,
    listRegistryCredentials,
    previewImagePush,
    readJob,
    type ImageArtifactSummary,
    type ImagePushPreview,
    type RegistryCredential,
    type StudioJob,
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
  import {
    canCommitPush,
    pushPreviewBinding,
    pushResultDigest,
    type PushPreviewBinding,
  } from "$lib/artifacts/push-state";
  import {
    deriveRegistryIdentity,
    isArtifactTransferResult,
    isCurrentTransferRequest,
    isTransferJobActive,
    resolvePullAuthSelection,
    transferJobOutcome,
    visibleTransferProgress,
  } from "$lib/jobs/transfer-job";
  import { Square, Upload } from "@lucide/svelte";

  let {
    engineId,
    image,
    open = $bindable(false),
  }: {
    engineId: string;
    image: ImageArtifactSummary;
    open?: boolean;
  } = $props();

  let destination = $state("");
  let credentialId = $state("");
  let credentials = $state<RegistryCredential[]>([]);
  let previewedBinding = $state<PushPreviewBinding | null>(null);
  let mutationState = $state<MutationState<ImagePushPreview, StudioJob>>(resetMutation(0));
  let job = $state<StudioJob | null>(null);
  let cancelling = $state(false);
  let jobError = $state<string | null>(null);
  let generation = 0;
  let controller: AbortController | null = null;

  const registry = $derived(deriveRegistryIdentity(destination));
  const matchingCredentials = $derived(
    credentials.filter((credential) => credential.registry === registry),
  );
  const currentBinding = $derived(
    pushPreviewBinding(engineId, image.id, destination, credentialId || null),
  );
  const commitEnabled = $derived(
    canCommitPush(
      mutationState.phase,
      mutationState.preview,
      previewedBinding,
      currentBinding,
    ),
  );
  const progressWindow = $derived(visibleTransferProgress(job?.transfer_progress ?? [], 8));
  const outcome = $derived(job ? transferJobOutcome(job) : null);

  $effect(() => {
    const isOpen = open;
    const engine = engineId;
    const imageId = image.id;
    const primaryReference = image.references[0] ?? "";
    generation += 1;
    const requestGeneration = generation;
    controller?.abort();
    controller = null;
    destination = primaryReference;
    credentialId = "";
    credentials = [];
    previewedBinding = null;
    mutationState = resetMutation(requestGeneration);
    job = null;
    cancelling = false;
    jobError = null;
    if (isOpen) {
      controller = new AbortController();
      void loadCredentials(controller.signal, requestGeneration);
    }
    void engine;
    void imageId;
    return () => controller?.abort();
  });

  async function loadCredentials(signal: AbortSignal, requestGeneration: number) {
    try {
      const result = await listRegistryCredentials({ signal });
      if (!isCurrentTransferRequest(generation, requestGeneration, signal.aborted)) return;
      credentials = result;
    } catch (caught) {
      if (!isCurrentTransferRequest(generation, requestGeneration, signal.aborted)) return;
      mutationState = applyPreviewError(
        mutationState,
        requestGeneration,
        toArtifactRequestError(caught),
      );
    }
  }

  async function preview() {
    const auth = resolvePullAuthSelection({
      registry,
      credentialId: credentialId || null,
      credentials,
      authSupported: true,
    });
    if (!registry || auth.kind === "blocked") return;
    previewedBinding = currentBinding;
    mutationState = startPreviewing(mutationState);
    const requestGeneration = mutationState.generation;
    controller?.abort();
    controller = new AbortController();
    try {
      const result = await previewImagePush(
        engineId,
        image.id,
        destination.trim(),
        auth.kind === "credential" ? auth.credential.id : null,
        { signal: controller.signal },
      );
      if (!isCurrentTransferRequest(generation, requestGeneration, controller.signal.aborted)) {
        return;
      }
      mutationState = applyPreviewSuccess(mutationState, requestGeneration, result);
    } catch (caught) {
      if (!isCurrentTransferRequest(generation, requestGeneration, controller.signal.aborted)) {
        return;
      }
      mutationState = applyPreviewError(
        mutationState,
        requestGeneration,
        toArtifactRequestError(caught),
      );
    }
  }

  async function commit() {
    const planId = mutationState.preview?.plan_id;
    if (!commitEnabled || !planId) return;
    mutationState = startCommitting(mutationState);
    const requestGeneration = mutationState.generation;
    controller?.abort();
    controller = new AbortController();
    try {
      const started = await commitImagePush(planId, { signal: controller.signal });
      if (!isCurrentTransferRequest(generation, requestGeneration, controller.signal.aborted)) {
        return;
      }
      mutationState = applyCommitSuccess(mutationState, requestGeneration, started);
      job = started;
      void pollJob(started.id, requestGeneration, controller.signal);
    } catch (caught) {
      if (!isCurrentTransferRequest(generation, requestGeneration, controller.signal.aborted)) {
        return;
      }
      mutationState = applyCommitError(
        mutationState,
        requestGeneration,
        toArtifactRequestError(caught),
      );
    }
  }

  async function pollJob(jobId: string, requestGeneration: number, signal: AbortSignal) {
    while (isCurrentTransferRequest(generation, requestGeneration, signal.aborted)) {
      await new Promise((resolve) => setTimeout(resolve, 1500));
      if (!isCurrentTransferRequest(generation, requestGeneration, signal.aborted)) return;
      try {
        const detail = await readJob(jobId, { signal });
        if (!isCurrentTransferRequest(generation, requestGeneration, signal.aborted)) return;
        job = detail;
        if (!isTransferJobActive(detail)) return;
      } catch (caught) {
        if (!isCurrentTransferRequest(generation, requestGeneration, signal.aborted)) return;
        mutationState = applyCommitError(
          mutationState,
          requestGeneration,
          toArtifactRequestError(caught),
        );
        return;
      }
    }
  }

  async function cancelPush() {
    if (!job || !isTransferJobActive(job) || cancelling) return;
    cancelling = true;
    jobError = null;
    try {
      await cancelJob(job.id);
    } catch (caught) {
      jobError = toArtifactRequestError(caught).message;
    } finally {
      cancelling = false;
    }
  }
</script>

<Dialog.Root bind:open>
  <Dialog.Content class="sm:max-w-lg">
    <Dialog.Header>
      <Dialog.Title>Push image</Dialog.Title>
      <Dialog.Description>
        Pushes an existing local reference to a registry. Tag the image first when you need a new
        destination.
      </Dialog.Description>
    </Dialog.Header>

    <div class="grid gap-4">
      <div class="grid gap-1 text-sm">
        <span class="font-medium">Source image</span>
        <span class="font-mono text-muted-foreground break-all">{image.id}</span>
      </div>

      <label class="grid gap-1.5 text-sm font-medium">
        Destination reference
        <Input
          bind:value={destination}
          placeholder="registry.example/team/app:v1"
          disabled={mutationState.phase === "committing" || (job !== null && isTransferJobActive(job))}
        />
      </label>

      <div class="grid gap-1 text-sm">
        <span class="font-medium">Registry</span>
        <span class="font-mono text-muted-foreground">{registry ?? "Enter a valid reference"}</span>
      </div>

      <label class="grid gap-1.5 text-sm font-medium">
        Authentication
        <select
          bind:value={credentialId}
          class="border-input h-9 rounded-md border bg-transparent px-3 text-sm"
          disabled={mutationState.phase === "committing" || (job !== null && isTransferJobActive(job))}
        >
          <option value="">Anonymous</option>
          {#each matchingCredentials as credential (credential.id)}
            <option value={credential.id} disabled={credential.status !== "ready"}>
              Studio-managed auth ({credential.username_label ?? "token"})
              {credential.status === "ready" ? "" : " - sign in again"}
            </option>
          {/each}
        </select>
      </label>

      {#if mutationState.error}
        <p class="text-sm text-destructive">{mutationState.error.message}</p>
      {/if}

      {#if mutationState.preview}
        <div class="grid gap-2 rounded-md border p-3 text-sm">
          <div class="flex flex-wrap justify-between gap-2">
            <span>{mutationState.preview.runtime.display_name ?? engineId}</span>
            <span class="text-xs text-muted-foreground">
              Preview valid for {mutationState.preview.expires_in_seconds ?? 0}s
            </span>
          </div>
          <div class="font-mono text-xs break-all">{mutationState.preview.destination}</div>
          <p class="text-xs text-muted-foreground">{mutationState.preview.warning}</p>
          {#if !mutationState.preview.commit_enabled}
            <p class="text-xs text-destructive">
              Push is unavailable for the current capability or active-work state.
            </p>
          {/if}
        </div>
      {/if}

      {#if mutationState.phase === "previewed" && !commitEnabled}
        <p class="text-sm text-muted-foreground">
          The destination, credential, runtime, or source changed. Preview again before pushing.
        </p>
      {:else if mutationState.phase === "commit_failed"}
        <p class="text-sm text-muted-foreground">
          This plan cannot be retried. Preview again to create a fresh plan.
        </p>
      {/if}

      {#if job}
        <div class="grid gap-2 rounded-md border p-3 text-sm">
          <div class="flex items-center justify-between gap-3">
            <span class="font-medium">{job.status}</span>
            <span class="font-mono text-xs text-muted-foreground">{job.id}</span>
          </div>
          {#if outcome === "uncertain"}
            <p class="text-xs text-muted-foreground">
              Studio stopped waiting. The registry outcome is still being checked.
            </p>
          {/if}
          {#if progressWindow.hiddenCount > 0}
            <p class="text-xs text-muted-foreground">
              {progressWindow.hiddenCount} earlier progress updates hidden.
            </p>
          {/if}
          {#each progressWindow.visible as entry (entry.sequence)}
            <div class="flex items-start justify-between gap-3 text-xs">
              <span class="min-w-0">
                <span class="font-medium">{entry.stage}</span>
                {#if entry.message}<span class="text-muted-foreground">: {entry.message}</span>{/if}
              </span>
              {#if entry.current_units !== null}
                <span class="shrink-0 tabular-nums text-muted-foreground">
                  {entry.current_units}{entry.total_units !== null ? ` / ${entry.total_units}` : ""}
                </span>
              {/if}
            </div>
          {/each}
          {#if job.result && isArtifactTransferResult(job.result) && "digest" in job.result}
            <div class="grid gap-1 text-xs">
              <span class="font-mono break-all">{job.result.image_reference}</span>
              <span class="text-muted-foreground">
                Digest: {pushResultDigest(job.result)}
              </span>
            </div>
          {/if}
          {#if job.error}<p class="text-xs text-destructive">{job.error}</p>{/if}
          {#if jobError}<p class="text-xs text-destructive">{jobError}</p>{/if}
        </div>
      {/if}
    </div>

    <Dialog.Footer>
      <Button type="button" variant="outline" onclick={() => (open = false)}>Close</Button>
      {#if job && isTransferJobActive(job)}
        <Button type="button" variant="outline" disabled={cancelling} onclick={cancelPush}>
          <Square />
          {cancelling ? "Cancelling" : "Cancel"}
        </Button>
      {:else}
        <Button
          type="button"
          variant="outline"
          disabled={mutationState.phase === "previewing" ||
            mutationState.phase === "committing" ||
            !registry}
          onclick={preview}
        >
          {mutationState.phase === "previewing" ? "Checking" : "Preview"}
        </Button>
        <Button type="button" disabled={!commitEnabled} onclick={commit}>
          <Upload />
          {mutationState.phase === "committing" ? "Starting" : "Push"}
        </Button>
      {/if}
    </Dialog.Footer>
  </Dialog.Content>
</Dialog.Root>
