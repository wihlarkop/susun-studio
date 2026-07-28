<script lang="ts">
  import { Badge } from "$lib/components/ui/badge/index.js";
  import { Button } from "$lib/components/ui/button/index.js";
  import { Skeleton } from "$lib/components/ui/skeleton/index.js";
  import {
    listRegistryCredentials,
    readEngineRegistryCapability,
    type RegistryCapabilityResponse,
    type RegistryCredential,
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
  import { toCredentialOperationError } from "$lib/registry/credential-state";
  import {
    KeyRound,
    LogIn,
    LogOut,
    RefreshCw,
    RotateCcw,
    ShieldCheck,
    ShieldAlert,
  } from "@lucide/svelte";
  import StatusBadge from "./status-badge.svelte";
  import ArtifactsStateBanner from "./artifacts-state-banner.svelte";
  import RegistryCredentialDialog from "./registry-credential-dialog.svelte";

  let { engineId, connected }: { engineId: string; connected: boolean } = $props();

  let capabilityState = $state(initialScopedFetchState<RegistryCapabilityResponse>());
  let credentialState = $state(initialScopedFetchState<RegistryCredential[]>());
  let generation = 0;
  let dialogOpen = $state(false);
  let dialogMode = $state<"create" | "rotate" | "delete">("create");
  let selectedCredential = $state<RegistryCredential | null>(null);

  async function loadCapabilities(
    id: string,
    signal: AbortSignal,
    requestGeneration: number,
  ) {
    try {
      const result = await readEngineRegistryCapability(id, { signal });
      if (signal.aborted) return;
      capabilityState = applyLoadSuccess(capabilityState, requestGeneration, result);
    } catch (caught) {
      if (signal.aborted) return;
      capabilityState = applyLoadError(
        capabilityState,
        requestGeneration,
        toArtifactRequestError(caught),
      );
    }
  }

  async function loadCredentials(signal: AbortSignal, requestGeneration: number) {
    try {
      const result = await listRegistryCredentials({ signal });
      if (signal.aborted) return;
      credentialState = applyLoadSuccess(credentialState, requestGeneration, result);
    } catch (caught) {
      if (signal.aborted) return;
      credentialState = applyLoadError(
        credentialState,
        requestGeneration,
        toCredentialOperationError(caught),
      );
    }
  }

  $effect(() => {
    const id = engineId;
    const isConnected = connected;
    generation += 1;
    const requestGeneration = generation;
    const controller = new AbortController();

    capabilityState = resetForNewEngine(requestGeneration, isConnected);
    credentialState = resetForNewEngine(requestGeneration, true);
    if (isConnected) {
      void loadCapabilities(id, controller.signal, requestGeneration);
    }
    void loadCredentials(controller.signal, requestGeneration);
    return () => controller.abort();
  });

  const capabilityView = $derived(
    resolveArtifactViewState({
      connected,
      loading: capabilityState.loading,
      hasData: capabilityState.data !== null,
      error: capabilityState.error,
      capability: null,
      itemCount: null,
    }),
  );

  const capabilityFlags = $derived(
    capabilityState.data
      ? [
          { label: "Pull", support: capabilityState.data.supports_pull },
          { label: "Push", support: capabilityState.data.supports_push },
          { label: "Authentication", support: capabilityState.data.supports_auth },
        ]
      : [],
  );

  function refresh() {
    const controller = new AbortController();
    capabilityState = withLoading(capabilityState);
    credentialState = withLoading(credentialState);
    if (connected) {
      void loadCapabilities(engineId, controller.signal, capabilityState.generation);
    }
    void loadCredentials(controller.signal, credentialState.generation);
  }

  async function refreshCredentials() {
    const controller = new AbortController();
    credentialState = withLoading(credentialState);
    await loadCredentials(controller.signal, credentialState.generation);
  }

  function openCreate() {
    selectedCredential = null;
    dialogMode = "create";
    dialogOpen = true;
  }

  function openRotate(credential: RegistryCredential) {
    selectedCredential = credential;
    dialogMode = "rotate";
    dialogOpen = true;
  }

  function openDelete(credential: RegistryCredential) {
    selectedCredential = credential;
    dialogMode = "delete";
    dialogOpen = true;
  }

  function statusLabel(credential: RegistryCredential): string {
    if (credential.status === "ready") return "Ready";
    if (credential.status === "reauthentication_required") return "Sign in again";
    return "Unavailable";
  }

  function statusVariant(
    credential: RegistryCredential,
  ): "secondary" | "destructive" | "outline" {
    if (credential.status === "ready") return "secondary";
    if (credential.status === "reauthentication_required") return "destructive";
    return "outline";
  }
</script>

<div class="flex flex-col gap-6">
  <div class="flex flex-wrap items-center justify-between gap-3">
    <div>
      <h3 class="text-sm font-semibold">Registry access</h3>
      <p class="mt-1 text-xs text-muted-foreground">
        Studio-owned credentials stay in this device's native credential manager.
      </p>
    </div>
    <div class="flex items-center gap-2">
      <Button size="sm" variant="outline" disabled={credentialState.loading} onclick={refresh}>
        <RefreshCw class={credentialState.loading ? "animate-spin" : undefined} />
        Refresh
      </Button>
      <Button size="sm" onclick={openCreate}>
        <LogIn />
        Sign in
      </Button>
    </div>
  </div>

  <section aria-labelledby="registry-capabilities-heading" class="grid gap-2">
    <div class="flex items-center justify-between gap-3">
      <h4 id="registry-capabilities-heading" class="text-sm font-medium">Engine capabilities</h4>
      {#if capabilityState.data}
        <span class="text-xs text-muted-foreground">{capabilityState.data.runtime.display_name}</span>
      {/if}
    </div>

    {#if capabilityView.kind === "ready" ||
    capabilityView.kind === "refreshing" ||
    capabilityView.kind === "stale"}
      <div class="flex flex-wrap gap-x-5 gap-y-2 border-y py-3">
        {#each capabilityFlags as flag (flag.label)}
          <div class="flex items-center gap-2">
            <span class="text-xs font-medium text-muted-foreground">{flag.label}</span>
            <StatusBadge status={flag.support} label={capabilityLabel(flag.support)} />
          </div>
        {/each}
      </div>
      {#if capabilityView.kind === "stale"}
        <p class="text-xs text-destructive">
          Couldn't refresh capabilities. Showing the last known values.
        </p>
      {/if}
    {:else}
      <ArtifactsStateBanner
        state={capabilityView}
        itemNoun="registry capabilities"
        onRetry={refresh}
      />
    {/if}
  </section>

  <section aria-labelledby="saved-credentials-heading" class="grid gap-2">
    <div class="flex items-center justify-between gap-3">
      <h4 id="saved-credentials-heading" class="text-sm font-medium">Saved credentials</h4>
      {#if credentialState.data}
        <span class="text-xs text-muted-foreground">
          {credentialState.data.length} {credentialState.data.length === 1 ? "registry" : "registries"}
        </span>
      {/if}
    </div>

    {#if credentialState.loading && credentialState.data === null}
      <div class="grid gap-2">
        <Skeleton class="h-14 w-full" />
        <Skeleton class="h-14 w-full" />
      </div>
    {:else if credentialState.error && credentialState.data === null}
      <div class="flex items-center justify-between gap-3 rounded-md border p-3">
        <p class="text-sm text-destructive">{credentialState.error.message}</p>
        <Button size="sm" variant="outline" onclick={refreshCredentials}>Retry</Button>
      </div>
    {:else if credentialState.data?.length === 0}
      <div class="flex items-start gap-3 rounded-md border border-dashed p-4">
        <KeyRound class="mt-0.5 size-4 text-muted-foreground" />
        <div class="min-w-0">
          <p class="text-sm font-medium">No Studio-owned credentials</p>
          <p class="mt-1 text-xs text-muted-foreground">
            Sign in when a private registry needs authentication. Existing Docker or Podman
            credentials remain external.
          </p>
        </div>
      </div>
    {:else if credentialState.data}
      {#if credentialState.error}
        <p class="text-xs text-destructive">
          Couldn't refresh credentials. Showing the last known metadata.
        </p>
      {/if}
      <div class="divide-y rounded-md border">
        {#each credentialState.data as credential (credential.id)}
          <div class="flex flex-wrap items-center gap-3 p-3">
            <div class="grid size-8 shrink-0 place-items-center rounded-md bg-muted">
              {#if credential.status === "ready"}
                <ShieldCheck class="size-4 text-muted-foreground" />
              {:else}
                <ShieldAlert class="size-4 text-destructive" />
              {/if}
            </div>
            <div class="min-w-[12rem] flex-1">
              <div class="truncate text-sm font-medium">{credential.registry}</div>
              <div class="truncate text-xs text-muted-foreground">
                {credential.username_label ?? "Token credential"}
              </div>
            </div>
            <Badge variant={statusVariant(credential)}>{statusLabel(credential)}</Badge>
            <div class="ml-auto flex items-center gap-1">
              <Button size="sm" variant="ghost" onclick={() => openRotate(credential)}>
                <RotateCcw />
                {credential.status === "ready" ? "Rotate" : "Sign in again"}
              </Button>
              <Button size="sm" variant="ghost" onclick={() => openDelete(credential)}>
                <LogOut />
                Sign out
              </Button>
            </div>
          </div>
        {/each}
      </div>
    {/if}
  </section>
</div>

<RegistryCredentialDialog
  mode={dialogMode}
  credential={selectedCredential}
  {engineId}
  bind:open={dialogOpen}
  oncompleted={refreshCredentials}
/>
