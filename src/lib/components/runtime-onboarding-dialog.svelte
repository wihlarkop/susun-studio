<script lang="ts">
  import * as Dialog from "$lib/components/ui/dialog/index.js";
  import { Badge } from "$lib/components/ui/badge/index.js";
  import { Button } from "$lib/components/ui/button/index.js";
  import RuntimeActionDialog from "$lib/components/runtime-action-dialog.svelte";
  import {
    completeRuntimeOnboarding,
    dismissRuntimeOnboarding,
    prepareRuntimeAction,
    setPreferredRuntime,
    type RuntimeOnboardingState,
    type RuntimeProfile,
    type RuntimeProviderStatus,
    type RuntimeStatus,
  } from "$lib/daemon/client";
  import type { RuntimeActionDialogRequest } from "$lib/components/runtime-action-dialog.svelte";
  import {
    canCompleteBuiltInOnboarding,
    canDismissInitialOnboarding,
    selectableExternalProfiles,
  } from "$lib/runtime/onboarding-state";
  import { ArrowLeft, HardDrive, RefreshCw, Server } from "@lucide/svelte";

  let {
    status,
    onboarding,
    open = $bindable(true),
    reopened = false,
    onchanged,
    onfinished,
  }: {
    status: RuntimeStatus;
    onboarding: RuntimeOnboardingState;
    open?: boolean;
    reopened?: boolean;
    onchanged: () => void | Promise<void>;
    onfinished?: () => void;
  } = $props();

  let screen = $state<"choice" | "existing">("choice");
  let selectingProfileId = $state<string | null>(null);
  let busy = $state(false);
  let message = $state<string | null>(null);
  let waitingForBuiltIn = $state(false);
  let completingBuiltIn = $state(false);
  let runtimeActionRequest = $state<RuntimeActionDialogRequest | null>(null);
  let runtimeActionOpen = $state(false);

  const providers = $derived(status.providers);
  const profiles = $derived(providers.flatMap((provider) => provider.profiles));
  const podmanProvider = $derived(providers.find((provider) => provider.experience.can_create_builtin) ?? null);
  const setupAction = $derived(
    podmanProvider?.actions.find((action) => action.id === "setup" && action.enabled) ??
      podmanProvider?.actions.find((action) => action.id === "install" && action.enabled) ??
      podmanProvider?.actions.find((action) => action.id === "start" && action.enabled) ??
      null,
  );
  const externalProfiles = $derived(
    selectableExternalProfiles(profiles)
      .map((id) => profiles.find((profile) => profile.id === id) ?? null)
      .filter((profile): profile is RuntimeProfile => profile !== null),
  );
  const builtInReadyAndPreferred = $derived(
    canCompleteBuiltInOnboarding({
      onboarding,
      binding: status.policy.binding,
      profiles,
    }),
  );

  $effect(() => {
    if (!waitingForBuiltIn || completingBuiltIn || !builtInReadyAndPreferred) return;
    void completeBuiltIn();
  });

  function setOpen(next: boolean) {
    if (next || !reopened) {
      open = true;
      return;
    }
    open = false;
  }

  function providerFor(profile: RuntimeProfile): RuntimeProviderStatus | null {
    return providers.find((provider) => provider.provider_id === profile.provider_id) ?? null;
  }

  function startBuiltInSetup() {
    message = null;
    waitingForBuiltIn = true;
    if (!podmanProvider || !setupAction) {
      message = "Susun Runtime is not ready to set up yet. Recheck providers and try again.";
      return;
    }
    runtimeActionRequest = {
      identity: { providerId: podmanProvider.provider_id, action: setupAction.id },
      prepare: () => prepareRuntimeAction(podmanProvider.provider_id, setupAction.id),
    };
    runtimeActionOpen = true;
  }

  async function refreshAfterBuiltInAction() {
    await onchanged();
    message = "Checking that Susun Runtime is ready and preferred...";
  }

  async function completeBuiltIn() {
    completingBuiltIn = true;
    try {
      await completeRuntimeOnboarding("built_in");
      await onchanged();
      message = null;
      waitingForBuiltIn = false;
      open = false;
      onfinished?.();
    } catch {
      message = "Susun Runtime is ready, but onboarding could not be completed. Try again.";
    } finally {
      completingBuiltIn = false;
    }
  }

  async function chooseExisting(profile: RuntimeProfile) {
    selectingProfileId = profile.id;
    message = null;
    try {
      await setPreferredRuntime(profile.id);
      await completeRuntimeOnboarding("existing");
      await onchanged();
      open = false;
      onfinished?.();
    } catch {
      message = "This runtime could not be selected. Recheck it and try again.";
    } finally {
      selectingProfileId = null;
    }
  }

  async function dismiss() {
    if (!canDismissInitialOnboarding({ reopened, onboarding })) return;
    busy = true;
    message = null;
    try {
      await dismissRuntimeOnboarding();
      await onchanged();
      open = false;
      onfinished?.();
    } catch {
      message = "Onboarding could not be dismissed right now. Try again.";
    } finally {
      busy = false;
    }
  }
</script>

<Dialog.Root bind:open={() => open, setOpen}>
  <Dialog.Content class="sm:max-w-2xl">
    <Dialog.Header>
      <Dialog.Title>{screen === "choice" ? "Choose a runtime" : "Use an existing runtime"}</Dialog.Title>
      <Dialog.Description>
        Choose how Susun Studio should run local container projects. This stays on your device.
      </Dialog.Description>
    </Dialog.Header>

    {#if message}
      <p class="rounded-md border p-3 text-sm text-muted-foreground">{message}</p>
    {/if}

    {#if screen === "choice"}
      <div class="grid gap-3 md:grid-cols-2">
        <button
          type="button"
          class="grid gap-3 rounded-md border border-primary bg-primary/5 p-4 text-left"
          onclick={startBuiltInSetup}
        >
          <div class="flex flex-wrap items-center gap-2">
            <HardDrive class="size-5 text-primary" />
            <span class="font-semibold">Set up Susun Runtime</span>
            <Badge>Recommended</Badge>
          </div>
          <p class="text-sm text-muted-foreground">
            Powered by Podman. Studio manages a dedicated machine for your projects.
          </p>
          <span class="text-sm font-medium">Set up managed runtime</span>
        </button>

        <button
          type="button"
          class="grid gap-3 rounded-md border p-4 text-left hover:bg-muted/30"
          onclick={() => {
            screen = "existing";
            message = null;
          }}
        >
          <div class="flex items-center gap-2">
            <Server class="size-5 text-muted-foreground" />
            <span class="font-semibold">Use an existing runtime</span>
          </div>
          <p class="text-sm text-muted-foreground">
            Select an available external runtime that Studio has already discovered.
          </p>
          <span class="text-sm font-medium">Choose external runtime</span>
        </button>
      </div>
    {:else}
      <div class="grid gap-3">
        {#if externalProfiles.length === 0}
          <div class="rounded-md border p-4 text-sm text-muted-foreground">
            No selectable external runtime is available. Start or install one separately, then recheck.
          </div>
        {:else}
          {#each externalProfiles as profile (profile.id)}
            {@const provider = providerFor(profile)}
            <div class="grid gap-3 rounded-md border p-4 md:grid-cols-[minmax(0,1fr)_auto] md:items-center">
              <div class="min-w-0">
                <div class="flex flex-wrap items-center gap-2">
                  <span class="font-medium">{profile.display_name}</span>
                  <Badge variant="secondary">External</Badge>
                  <Badge variant="outline">{profile.runtime_class.replaceAll("_", " ")}</Badge>
                </div>
                <p class="mt-1 text-sm text-muted-foreground">
                  {provider?.display_name ?? "External runtime"} · setup {provider?.experience.can_create_builtin
                    ? "available"
                    : "external"} · lifecycle {provider?.experience.can_manage_external_lifecycle
                    ? "available"
                    : "managed outside Studio"} · resources {provider?.experience.can_manage_resources
                    ? "available"
                    : "provider-managed"} · {provider?.experience.requires_external_desktop_app
                    ? "requires external desktop app"
                    : "does not require a desktop app"}
                </p>
              </div>
              <Button
                size="sm"
                disabled={selectingProfileId !== null}
                onclick={() => chooseExisting(profile)}
              >
                {selectingProfileId === profile.id ? "Selecting..." : "Use this runtime"}
              </Button>
            </div>
          {/each}
        {/if}
        <div class="flex flex-wrap justify-between gap-2">
          <Button size="sm" variant="outline" onclick={() => (screen = "choice")}>
            <ArrowLeft />
            Back
          </Button>
          <Button size="sm" variant="outline" disabled={busy} onclick={onchanged}>
            <RefreshCw />
            Recheck
          </Button>
        </div>
      </div>
    {/if}

    {#if canDismissInitialOnboarding({ reopened, onboarding })}
      <Dialog.Footer>
        <Button type="button" variant="ghost" disabled={busy} onclick={dismiss}>Not now</Button>
      </Dialog.Footer>
    {/if}
  </Dialog.Content>
</Dialog.Root>

<RuntimeActionDialog
  request={runtimeActionRequest}
  bind:open={runtimeActionOpen}
  oncompleted={refreshAfterBuiltInAction}
/>
