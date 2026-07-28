<script lang="ts">
  import * as Dialog from "$lib/components/ui/dialog/index.js";
  import { Button } from "$lib/components/ui/button/index.js";
  import { Input } from "$lib/components/ui/input/index.js";
  import {
    cancelJob,
    listRegistryCredentials,
    readEngineRegistryCapability,
    readJob,
    startImagePull,
    type RegistryCapabilityResponse,
    type RegistryCredential,
    type StudioJob,
  } from "$lib/daemon/client";
  import { isCapabilityUsable } from "$lib/artifacts/capability";
  import { toArtifactRequestError } from "$lib/artifacts/fetch-error";
  import {
    deriveRegistryIdentity,
    isArtifactTransferResult,
    isCurrentTransferRequest,
    isTransferJobActive,
    resolvePullAuthSelection,
    transferJobOutcome,
    visibleTransferProgress,
  } from "$lib/jobs/transfer-job";
  import { Download, Square } from "@lucide/svelte";

  let {
    engineId,
    connected,
    open = $bindable(false),
    oncompleted,
  }: {
    engineId: string;
    connected: boolean;
    open?: boolean;
    oncompleted?: () => void | Promise<void>;
  } = $props();

  let imageReference = $state("");
  let credentialId = $state("");
  let capability = $state<RegistryCapabilityResponse | null>(null);
  let credentials = $state<RegistryCredential[]>([]);
  let loadingOptions = $state(false);
  let submitting = $state(false);
  let cancelling = $state(false);
  let operationError = $state<string | null>(null);
  let job = $state<StudioJob | null>(null);
  let generation = 0;
  let controller: AbortController | null = null;

  const registry = $derived(deriveRegistryIdentity(imageReference));
  const matchingCredentials = $derived(
    credentials.filter((credential) => credential.registry === registry),
  );
  const pullSupported = $derived(
    capability !== null && isCapabilityUsable(capability.supports_pull),
  );
  const authSupported = $derived(
    capability !== null && isCapabilityUsable(capability.supports_auth),
  );
  const authSelection = $derived(
    resolvePullAuthSelection({
      registry,
      credentialId: credentialId || null,
      credentials,
      authSupported,
    }),
  );
  const progressWindow = $derived(visibleTransferProgress(job?.transfer_progress ?? [], 8));
  const outcome = $derived(job ? transferJobOutcome(job) : null);
  const canSubmit = $derived(
    connected &&
      pullSupported &&
      registry !== null &&
      authSelection.kind !== "blocked" &&
      !submitting &&
      !(job && isTransferJobActive(job)),
  );

  async function loadOptions(
    id: string,
    signal: AbortSignal,
    requestGeneration: number,
  ) {
    try {
      const [nextCapability, nextCredentials] = await Promise.all([
        readEngineRegistryCapability(id, { signal }),
        listRegistryCredentials({ signal }),
      ]);
      if (!isCurrentTransferRequest(generation, requestGeneration, signal.aborted)) return;
      capability = nextCapability;
      credentials = nextCredentials;
    } catch (caught) {
      if (!isCurrentTransferRequest(generation, requestGeneration, signal.aborted)) return;
      operationError = toArtifactRequestError(caught).message;
    } finally {
      if (isCurrentTransferRequest(generation, requestGeneration, signal.aborted)) {
        loadingOptions = false;
      }
    }
  }

  $effect(() => {
    const isOpen = open;
    const id = engineId;
    const isConnected = connected;
    generation += 1;
    const requestGeneration = generation;
    controller?.abort();
    controller = null;
    imageReference = "";
    credentialId = "";
    capability = null;
    credentials = [];
    operationError = null;
    job = null;
    submitting = false;
    cancelling = false;
    loadingOptions = isOpen && isConnected;
    if (isOpen && isConnected) {
      controller = new AbortController();
      void loadOptions(id, controller.signal, requestGeneration);
    }
    return () => controller?.abort();
  });

  async function pollJob(jobId: string, requestGeneration: number, signal: AbortSignal) {
    while (isCurrentTransferRequest(generation, requestGeneration, signal.aborted)) {
      await new Promise((resolve) => setTimeout(resolve, 1500));
      if (!isCurrentTransferRequest(generation, requestGeneration, signal.aborted)) return;
      try {
        const detail = await readJob(jobId, { signal });
        if (!isCurrentTransferRequest(generation, requestGeneration, signal.aborted)) return;
        job = detail;
        if (!isTransferJobActive(detail)) {
          if (detail.status === "succeeded") await oncompleted?.();
          return;
        }
      } catch (caught) {
        if (!isCurrentTransferRequest(generation, requestGeneration, signal.aborted)) return;
        operationError = toArtifactRequestError(caught).message;
        return;
      }
    }
  }

  async function startPull() {
    if (!canSubmit) return;
    const requestGeneration = generation;
    controller?.abort();
    controller = new AbortController();
    submitting = true;
    operationError = null;
    try {
      const started = await startImagePull(
        engineId,
        {
          image: imageReference.trim(),
          credential_id:
            authSelection.kind === "credential" ? authSelection.credential.id : null,
        },
        { signal: controller.signal },
      );
      if (!isCurrentTransferRequest(generation, requestGeneration, controller.signal.aborted)) {
        return;
      }
      job = started;
      void pollJob(started.id, requestGeneration, controller.signal);
    } catch (caught) {
      if (!isCurrentTransferRequest(generation, requestGeneration, controller.signal.aborted)) {
        return;
      }
      operationError = toArtifactRequestError(caught).message;
    } finally {
      if (isCurrentTransferRequest(generation, requestGeneration, controller.signal.aborted)) {
        submitting = false;
      }
    }
  }

  async function cancelPull() {
    if (!job || !isTransferJobActive(job) || cancelling) return;
    cancelling = true;
    operationError = null;
    try {
      await cancelJob(job.id);
    } catch (caught) {
      operationError = toArtifactRequestError(caught).message;
    } finally {
      cancelling = false;
    }
  }
</script>

<Dialog.Root bind:open>
  <Dialog.Content class="sm:max-w-lg">
    <Dialog.Header>
      <Dialog.Title>Pull image</Dialog.Title>
      <Dialog.Description>
        Download an image into the selected runtime. Public images can remain anonymous.
      </Dialog.Description>
    </Dialog.Header>

    <div class="grid gap-4">
      <label class="grid gap-1.5 text-sm font-medium">
        Image reference
        <Input
          bind:value={imageReference}
          placeholder="alpine:latest or ghcr.io/team/app:v1"
          disabled={submitting || (job !== null && isTransferJobActive(job))}
        />
      </label>

      <div class="grid gap-1 text-sm">
        <span class="font-medium">Runtime</span>
        <span class="text-muted-foreground">
          {capability?.runtime.display_name ?? (loadingOptions ? "Checking runtime" : engineId)}
        </span>
      </div>

      <div class="grid gap-1 text-sm">
        <span class="font-medium">Registry</span>
        <span class="font-mono text-muted-foreground">{registry ?? "Enter a valid image reference"}</span>
      </div>

      <label class="grid gap-1.5 text-sm font-medium">
        Authentication
        <select
          bind:value={credentialId}
          class="border-input h-9 rounded-md border bg-transparent px-3 text-sm"
          disabled={loadingOptions || submitting || (job !== null && isTransferJobActive(job))}
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

      {#if credentialId && !authSupported}
        <p class="text-sm text-destructive">
          This runtime does not support authenticated registry pulls.
        </p>
      {:else if !pullSupported && !loadingOptions}
        <p class="text-sm text-destructive">Image pull is not supported by this runtime.</p>
      {:else if registry && matchingCredentials.length === 0}
        <p class="text-xs text-muted-foreground">
          No Studio-managed credential is saved for {registry}. Anonymous pull remains available.
        </p>
      {/if}

      {#if operationError}
        <p class="text-sm text-destructive">{operationError}</p>
      {/if}

      {#if job}
        <div class="grid gap-2 rounded-md border p-3 text-sm">
          <div class="flex items-center justify-between gap-3">
            <span class="font-medium">{job.status}</span>
            <span class="font-mono text-xs text-muted-foreground">{job.id}</span>
          </div>
          {#if outcome === "uncertain"}
            <p class="text-xs text-muted-foreground">
              Studio stopped waiting. The runtime is still being checked for the final outcome.
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
          {#if job.result && isArtifactTransferResult(job.result)}
            <p class="font-mono text-xs break-all">{job.result.image_reference}</p>
          {/if}
          {#if job.error}
            <p class="text-xs text-destructive">{job.error}</p>
          {/if}
        </div>
      {/if}
    </div>

    <Dialog.Footer>
      <Button type="button" variant="outline" onclick={() => (open = false)}>Close</Button>
      {#if job && isTransferJobActive(job)}
        <Button type="button" variant="outline" disabled={cancelling} onclick={cancelPull}>
          <Square />
          {cancelling ? "Cancelling" : "Cancel"}
        </Button>
      {:else}
        <Button type="button" disabled={!canSubmit} onclick={startPull}>
          <Download />
          {submitting ? "Starting" : "Pull"}
        </Button>
      {/if}
    </Dialog.Footer>
  </Dialog.Content>
</Dialog.Root>
