<script lang="ts">
  import * as Card from "$lib/components/ui/card/index.js";
  import * as Dialog from "$lib/components/ui/dialog/index.js";
  import { Badge } from "$lib/components/ui/badge/index.js";
  import { Button } from "$lib/components/ui/button/index.js";
  import RuntimeMigrationDialog from "$lib/components/runtime-migration-dialog.svelte";
  import RuntimeDataScopeDialog from "$lib/components/runtime-data-scope-dialog.svelte";
  import RuntimeActionDialog from "$lib/components/runtime-action-dialog.svelte";
  import RuntimeActionAudit from "$lib/components/runtime-action-audit.svelte";
  import RuntimeResourcePanel from "$lib/components/runtime-resource-panel.svelte";
  import RuntimeIdentity from "$lib/components/runtime-identity.svelte";
  import RuntimeCompatibilityPanel from "$lib/components/runtime-compatibility-panel.svelte";
  import PruneDialog from "$lib/components/prune-dialog.svelte";
  import {
    forgetRuntimeProfile,
    readRuntimeLogs,
    readRuntimeProfileResources,
    prepareRuntimeAction,
    prepareRuntimeResourceUpdate,
    type RuntimeAction,
    type RuntimeDimension,
    type RuntimeEndpointSummary,
    type RuntimeLogLine,
    type RuntimeProfile,
    type RuntimeResourceSnapshot,
    type RuntimeProviderStatus,
    type RuntimeStatus,
  } from "$lib/daemon/client";
  import { resolveActiveEngineId } from "$lib/engine-identity";
  import { presentRuntimeBinding, presentRuntimeProfile } from "$lib/runtime/presentation";
  import type { RuntimeContextTarget } from "$lib/runtime/context-change";
  import type { RuntimeActionDialogRequest } from "$lib/components/runtime-action-dialog.svelte";
  import {
    AlertCircle,
    ArrowRightLeft,
    CheckCircle2,
    ChevronRight,
    HardDrive,
    Play,
    RefreshCw,
    RotateCw,
    Server,
    SlidersHorizontal,
    Square,
    TerminalSquare,
    Trash2,
    Wrench,
  } from "@lucide/svelte";

  let {
    runtimeStatus,
    refreshing,
    onRecheck,
    onChooseRuntime,
    onContextChange,
    trayRuntimeAction = null,
    onTrayRuntimeActionHandled,
  }: {
    runtimeStatus: RuntimeStatus | undefined;
    refreshing: boolean;
    onRecheck: () => Promise<void>;
    onChooseRuntime: () => void;
    onContextChange: (target: RuntimeContextTarget) => void;
    trayRuntimeAction?: { action: "start" | "stop"; requestId: number } | null;
    onTrayRuntimeActionHandled?: () => void;
  } = $props();

  let logs = $state<RuntimeLogLine[]>([]);
  let logsLoading = $state(false);
  let errorMessage = $state<string | null>(null);
  let expandedProviders = $state<Set<string>>(new Set());
  let ownershipDialogOpen = $state(false);
  let ownershipDialogBusy = $state(false);
  let pendingOwnershipProfile = $state<RuntimeProfile | null>(null);
  let pendingOwnershipAction = $state<"forget" | null>(null);
  let runtimeActionRequest = $state<RuntimeActionDialogRequest | null>(null);
  let runtimeActionDialogOpen = $state(false);
  let migrationDialogOpen = $state(false);
  let dataScopeDialogOpen = $state(false);
  let dataScopeProfile = $state<RuntimeProfile | null>(null);
  let pruneDialogOpen = $state(false);
  let pruneProfile = $state<RuntimeProfile | null>(null);
  let resourceSnapshots = $state<Record<string, RuntimeResourceSnapshot>>({});
  let resourceLoading = $state<Record<string, boolean>>({});
  let resourceErrors = $state<Record<string, string>>({});
  let builtInExpanded = $state(false);
  let existingExpanded = $state(false);

  const actionIcons = {
    install: Wrench,
    setup: HardDrive,
    start: Play,
    stop: Square,
    restart: RotateCw,
  } as const;

  const status = $derived(runtimeStatus ?? null);
  const providers = $derived(status?.providers ?? []);
  const runtimePreference = $derived(status?.policy ?? null);
  const pruneEngineId = $derived(
    runtimePreference ? resolveActiveEngineId(runtimePreference.binding) : null,
  );
  const readyProviders = $derived(
    providers.filter((provider) => provider.connection.state === "summarized"),
  );
  const enabledActions = $derived(
    providers.reduce(
      (total, provider) => total + provider.actions.filter((action) => action.enabled).length,
      0,
    ),
  );
  const managedBuiltIn = $derived(
    providers
      .flatMap((provider) => provider.profiles)
      .find(
        (profile) =>
          profile.runtime_class === "built_in" && profile.ownership_state === "studio_managed",
      ),
  );
  const podmanProvider = $derived(
    providers.find((provider) => provider.provider_id === "windows-podman"),
  );
  const setupAction = $derived(podmanProvider?.actions.find((action) => action.id === "setup"));
  const preferredPresentation = $derived(
    runtimePreference ? presentRuntimeBinding(runtimePreference.binding) : null,
  );
  const profileEntries = $derived(
    providers.flatMap((provider) => provider.profiles.map((profile) => ({ profile, provider }))),
  );
  const builtInEntries = $derived(
    profileEntries.filter((entry) => entry.profile.runtime_class === "built_in"),
  );
  const externalEntries = $derived(
    profileEntries.filter((entry) => entry.profile.runtime_class !== "built_in"),
  );
  const builtInNeedsAttention = $derived(
    !managedBuiltIn ||
      builtInEntries.some(
        (entry) =>
          entry.profile.management.requires_recovery || entry.profile.availability_state !== "available",
      ),
  );

  $effect(() => {
    if (builtInNeedsAttention) {
      builtInExpanded = true;
    }
  });

  $effect(() => {
    const controller = new AbortController();
    if (status) {
      void refreshLogs(controller.signal);
    } else {
      logs = [];
    }
    return () => controller.abort();
  });

  async function refreshLogs(signal?: AbortSignal) {
    logsLoading = true;
    try {
      logs = await readRuntimeLogs({ signal });
      errorMessage = null;
    } catch (error) {
      if (!signal?.aborted) {
        errorMessage = error instanceof Error ? error.message : String(error);
      }
    } finally {
      logsLoading = false;
    }
  }

  async function refreshRuntime() {
    await onRecheck();
    for (const profile of providers
      .filter((provider) => expandedProviders.has(provider.provider_id))
      .flatMap((provider) => provider.profiles)
      .filter((profile) => profile.runtime_class === "built_in")) {
      void loadResources(profile);
    }
  }

  async function loadResources(profile: RuntimeProfile, signal?: AbortSignal) {
    resourceLoading = { ...resourceLoading, [profile.id]: true };
    try {
      const snapshot = await readRuntimeProfileResources(profile.id, { signal });
      resourceSnapshots = { ...resourceSnapshots, [profile.id]: snapshot };
      const { [profile.id]: _removed, ...remainingErrors } = resourceErrors;
      resourceErrors = remainingErrors;
    } catch (error) {
      if (!signal?.aborted) {
        resourceErrors = {
          ...resourceErrors,
          [profile.id]: error instanceof Error ? error.message : String(error),
        };
      }
    } finally {
      resourceLoading = { ...resourceLoading, [profile.id]: false };
    }
  }

  function handleAction(providerId: string, action: RuntimeAction) {
    runtimeActionRequest = {
      identity: { providerId, action: action.id },
      prepare: () => prepareRuntimeAction(providerId, action.id),
    };
    runtimeActionDialogOpen = true;
  }

  function openTrayRuntimeAction(actionId: "start" | "stop") {
    const selectedProfileId = runtimePreference?.binding.profile_id;
    const profile = selectedProfileId
      ? profileEntries.find((entry) => entry.profile.id === selectedProfileId)?.profile
      : undefined;
    const provider = profile
      ? providers.find((candidate) => candidate.provider_id === profile.provider_id)
      : undefined;
    const action = provider?.actions.find(
      (candidate) => candidate.id === actionId && candidate.enabled,
    );
    const selectedBuiltIn =
      profile?.runtime_class === "built_in" &&
      profile.ownership_state === "studio_managed" &&
      runtimePreference?.binding.runtime_class === "built_in";
    if (!selectedBuiltIn || !profile || !provider || !action) {
      errorMessage = "The requested built-in runtime action is no longer available.";
      return;
    }
    handleAction(provider.provider_id, action);
  }

  $effect(() => {
    const request = trayRuntimeAction;
    if (!request) return;
    openTrayRuntimeAction(request.action);
    onTrayRuntimeActionHandled?.();
  });

  function handleResourceUpdate(
    profile: RuntimeProfile,
    networkMode: "wsl" | "user_mode",
  ) {
    runtimeActionRequest = {
      identity: {
        providerId: profile.provider_id,
        action: `resource_network_${networkMode}:${profile.id}`,
      },
      prepare: () => prepareRuntimeResourceUpdate(profile.id, networkMode),
    };
    runtimeActionDialogOpen = true;
  }

  function handleSelect(profile: RuntimeProfile) {
    onContextChange({ kind: "preference", profileId: profile.id });
  }

  function isPreferred(profile: RuntimeProfile): boolean {
    return runtimePreference?.preferred_profile_id === profile.id;
  }

  async function handleForget(profile: RuntimeProfile) {
    try {
      await forgetRuntimeProfile(profile.id);
      errorMessage = null;
    } catch (error) {
      errorMessage = error instanceof Error ? error.message : String(error);
    }
    await refreshRuntime();
  }

  function requestOwnershipAction(profile: RuntimeProfile, action: "forget") {
    pendingOwnershipProfile = profile;
    pendingOwnershipAction = action;
    ownershipDialogOpen = true;
  }

  async function confirmOwnershipAction() {
    if (!pendingOwnershipProfile || !pendingOwnershipAction) return;
    ownershipDialogBusy = true;
    try {
      await handleForget(pendingOwnershipProfile);
      ownershipDialogOpen = false;
    } finally {
      ownershipDialogBusy = false;
    }
  }

  // Identity/ownership/availability are surfaced as their own concepts, kept
  // separate from the install/process/connection health chips.
  function ownershipBadges(
    profile: RuntimeProfile,
  ): { label: string; variant: "default" | "secondary" | "outline" | "destructive" }[] {
    const badges: {
      label: string;
      variant: "default" | "secondary" | "outline" | "destructive";
    }[] = [];
    badges.push(
      profile.runtime_class === "built_in"
        ? { label: "Built-in", variant: "default" }
        : { label: "External", variant: "secondary" },
    );
    if (profile.ownership_state === "ownership_conflict") {
      badges.push({ label: "Ownership conflict", variant: "destructive" });
    } else if (profile.ownership_state === "studio_managed") {
      badges.push({ label: "Studio-managed", variant: "outline" });
    }
    if (profile.availability_state === "missing") {
      badges.push({ label: "Missing", variant: "destructive" });
    }
    return badges;
  }

  function stateLabel(value: string): string {
    return value.replaceAll("_", " ");
  }

  function dimensionVariant(
    dimension: RuntimeDimension,
  ): "default" | "secondary" | "outline" | "destructive" {
    if (["installed", "running", "reachable", "summarized"].includes(dimension.state)) {
      return "default";
    }
    if (["failed", "not_installed", "unreachable"].includes(dimension.state)) {
      return "destructive";
    }
    if (["unknown", "not_applicable"].includes(dimension.state)) return "secondary";
    return "outline";
  }

  function providerVariant(
    provider: RuntimeProviderStatus,
  ): "default" | "secondary" | "outline" | "destructive" {
    if (!provider.supported) return "secondary";
    if (provider.connection.state === "summarized") return "default";
    if (provider.installation.state === "not_installed") return "outline";
    if (provider.process.state === "failed" || provider.connection.state === "failed") {
      return "destructive";
    }
    return "outline";
  }

  function providerStatusLabel(provider: RuntimeProviderStatus): string {
    if (!provider.supported) return "Unsupported";
    if (provider.connection.state === "summarized") return "Ready";
    if (provider.process.state === "running") return "Starting";
    if (provider.installation.state === "installed") return "Installed";
    if (provider.installation.state === "not_installed") return "Not installed";
    return stateLabel(provider.connection.state);
  }

  function endpointLabel(summary: RuntimeProfile["endpoint_summary"]): string | null {
    if (!summary) return null;
    if (typeof summary !== "string") return summary.redacted;
    try {
      return (JSON.parse(summary) as RuntimeEndpointSummary).redacted;
    } catch {
      return summary;
    }
  }

  function providerDimensions(provider: RuntimeProviderStatus) {
    return [
      { label: "Install", value: provider.installation },
      { label: "Process", value: provider.process },
      { label: "Endpoint", value: provider.connection },
    ];
  }

  function enabledActionSummary(provider: RuntimeProviderStatus): string {
    const enabled = provider.actions.filter((action) => action.enabled);
    if (enabled.length === 0) return "No action is currently available.";
    return enabled.map((action) => action.label).join(", ");
  }

  function toggleProvider(provider: RuntimeProviderStatus) {
    const next = new Set(expandedProviders);
    if (next.has(provider.provider_id)) {
      next.delete(provider.provider_id);
    } else {
      next.add(provider.provider_id);
      for (const profile of provider.profiles.filter(
        (profile) => profile.runtime_class === "built_in",
      )) {
        void loadResources(profile);
      }
    }
    expandedProviders = next;
  }

  function providerOpen(providerId: string): boolean {
    return expandedProviders.has(providerId);
  }

  function reviewDataScope(profile: RuntimeProfile) {
    dataScopeProfile = profile;
    dataScopeDialogOpen = true;
  }

  function reviewPrune(profile: RuntimeProfile) {
    pruneProfile = profile;
    pruneDialogOpen = true;
  }

  function entryPresentation(profile: RuntimeProfile, provider: RuntimeProviderStatus) {
    return presentRuntimeProfile(profile, provider);
  }
</script>

<div class="flex flex-col gap-4">
  <div class="flex flex-wrap items-start justify-between gap-3">
    <div class="max-w-3xl">
      <h3 class="text-lg font-semibold">Runtime choices</h3>
      <p class="text-sm text-muted-foreground">
        Choose the runtime Studio prefers for new project actions. Existing runtimes remain
        external; unavailable choices block actions instead of falling back silently.
      </p>
    </div>
    <Button size="sm" variant="outline" disabled={refreshing} onclick={refreshRuntime}>
      <RefreshCw />
      {refreshing ? "Checking" : "Recheck"}
    </Button>
    <Button size="sm" variant="outline" onclick={() => (migrationDialogOpen = true)}>
      <ArrowRightLeft />
      Migrate projects
    </Button>
    <Button size="sm" variant="outline" onclick={onChooseRuntime}>
      <Server />
      Choose runtime
    </Button>
  </div>

  <div class="grid gap-2 md:grid-cols-3">
    <div class="rounded-md border p-3">
      <div class="text-xs font-medium text-muted-foreground">Ready providers</div>
      <div class="mt-2 flex items-center gap-2">
        <Badge variant={readyProviders.length > 0 ? "default" : "outline"}>
          {readyProviders.length} / {providers.length}
        </Badge>
        <span class="text-sm text-muted-foreground">
          {readyProviders.length > 0 ? "runtime endpoint available" : "no endpoint ready"}
        </span>
      </div>
    </div>
    <div class="rounded-md border p-3">
      <div class="text-xs font-medium text-muted-foreground">Preferred runtime</div>
      <div class="mt-2 min-w-0">
        {#if preferredPresentation}
          <RuntimeIdentity presentation={preferredPresentation} compact />
        {:else}
          <span class="text-sm text-muted-foreground">Loading runtime policy</span>
        {/if}
      </div>
    </div>
    <div class="rounded-md border p-3">
      <div class="text-xs font-medium text-muted-foreground">Available actions</div>
      <div class="mt-2 flex items-center gap-2">
        <Badge variant={enabledActions > 0 ? "default" : "secondary"}>{enabledActions}</Badge>
        <span class="text-sm text-muted-foreground">
          {enabledActions === 1 ? "provider action" : "provider actions"}
        </span>
      </div>
    </div>
  </div>

  {#if errorMessage}
    <div class="flex gap-2 rounded-md border border-destructive/30 bg-destructive/5 p-3 text-sm text-destructive">
      <AlertCircle class="mt-0.5 size-4 shrink-0" />
      <span>{errorMessage}</span>
    </div>
  {/if}

  <Card.Root class="gap-0 overflow-hidden p-0">
    <button
      type="button"
      class="flex w-full items-center justify-between gap-3 border-b bg-primary/5 p-4 text-left"
      aria-expanded={builtInExpanded}
      onclick={() => (builtInExpanded = !builtInExpanded)}
    >
      <div class="min-w-0">
        <div class="flex flex-wrap items-center gap-2">
          <ChevronRight class="size-4 transition-transform {builtInExpanded ? 'rotate-90' : ''}" />
          <h4 class="text-sm font-semibold">Susun Runtime</h4>
          <Badge>Built-in</Badge>
          <Badge variant="secondary">Recommended</Badge>
        </div>
        <p class="mt-1 text-sm text-muted-foreground">Powered by Podman and managed by Studio.</p>
      </div>
      <Badge variant={builtInNeedsAttention ? "outline" : "secondary"}>
        {builtInNeedsAttention ? "Needs attention" : "Ready"}
      </Badge>
    </button>
    {#if builtInExpanded}
      <div class="space-y-3 p-4">
        {#if !managedBuiltIn && podmanProvider}
          <div class="grid gap-3 rounded-md border p-3 md:grid-cols-[minmax(0,1fr)_auto] md:items-center">
            <p class="text-sm text-muted-foreground">
              Set up a dedicated local runtime without installing Docker Desktop.
            </p>
            {#if setupAction}
              <Button
                size="sm"
                disabled={!setupAction.enabled}
                title={setupAction.reason}
                onclick={() => handleAction(podmanProvider.provider_id, setupAction)}
              >
                <HardDrive />
                Set up Susun Runtime
              </Button>
            {/if}
          </div>
        {/if}
        {#if builtInEntries.length === 0}
          <p class="text-sm text-muted-foreground">No managed Susun Runtime has been observed yet.</p>
        {:else}
          <div class="space-y-2">
            {#each builtInEntries as entry (entry.profile.id)}
              <RuntimeIdentity presentation={entryPresentation(entry.profile, entry.provider)} />
            {/each}
          </div>
        {/if}
      </div>
    {/if}
  </Card.Root>

  <Card.Root class="gap-0 overflow-hidden p-0">
    <button
      type="button"
      class="flex w-full items-center justify-between gap-3 border-b bg-muted/20 p-4 text-left"
      aria-expanded={existingExpanded}
      onclick={() => (existingExpanded = !existingExpanded)}
    >
      <div>
        <div class="flex items-center gap-2">
          <ChevronRight class="size-4 transition-transform {existingExpanded ? 'rotate-90' : ''}" />
          <h4 class="text-sm font-semibold">Existing runtimes</h4>
        </div>
        <p class="mt-1 text-sm text-muted-foreground">External Podman, Docker Desktop, and remote runtimes.</p>
      </div>
      <Badge variant="outline">{externalEntries.length}</Badge>
    </button>
    {#if existingExpanded}
      <div class="space-y-3 p-4">
        {#if externalEntries.length === 0}
          <p class="text-sm text-muted-foreground">No selectable external runtime has been observed.</p>
        {:else}
          {#each externalEntries as entry (entry.profile.id)}
            <div class="flex min-w-0 flex-wrap items-center justify-between gap-3 rounded-md border p-3">
              <div class="min-w-0 flex-1">
                <RuntimeIdentity presentation={entryPresentation(entry.profile, entry.provider)} />
                <RuntimeCompatibilityPanel profile={entry.profile} />
              </div>
              <Button
                size="sm"
                variant={isPreferred(entry.profile) ? "secondary" : "outline"}
                disabled={isPreferred(entry.profile) || !entry.profile.management.can_select}
                onclick={() => handleSelect(entry.profile)}
              >
                {isPreferred(entry.profile) ? "Preferred" : "Make preferred"}
              </Button>
            </div>
          {/each}
        {/if}
      </div>
    {/if}
  </Card.Root>

  {#if !status && !errorMessage}
    <div class="rounded-md border p-4 text-sm text-muted-foreground">
      Checking runtime providers...
    </div>
  {/if}

  <div class="flex flex-wrap items-end justify-between gap-2">
    <div>
      <h4 class="text-sm font-semibold">Advanced provider diagnostics</h4>
      <p class="text-xs text-muted-foreground">
        Inspect provider health and the safe actions currently supported by the daemon.
      </p>
    </div>
    <Badge variant="outline">{providers.length}</Badge>
  </div>

  {#each providers as provider (provider.provider_id)}
    {@const open = providerOpen(provider.provider_id)}
    <Card.Root class="gap-0 overflow-hidden p-0">
      <button
        type="button"
        class="w-full border-b bg-muted/20 p-4 text-left transition-colors hover:bg-muted/35"
        aria-expanded={open}
        onclick={() => toggleProvider(provider)}
      >
        <div class="grid gap-3 md:grid-cols-[minmax(0,1fr)_auto] md:items-center">
          <div class="min-w-0">
            <div class="flex min-w-0 flex-wrap items-center gap-2">
              <ChevronRight
                class="size-4 shrink-0 text-muted-foreground transition-transform {open
                  ? 'rotate-90'
                  : ''}"
              />
              <span class="min-w-0 truncate text-base font-semibold">{provider.display_name}</span>
              <Badge variant={providerVariant(provider)}>{providerStatusLabel(provider)}</Badge>
              <Badge variant="secondary">{provider.platform}</Badge>
            </div>
            <p class="mt-2 max-w-3xl text-sm text-muted-foreground">{provider.summary}</p>
          </div>
          <div class="flex flex-wrap items-center gap-2 md:justify-end">
            <Badge variant={provider.profiles.length > 0 ? "secondary" : "outline"}>
              {provider.profiles.length}
              {provider.profiles.length === 1 ? "profile" : "profiles"}
            </Badge>
            <span class="text-xs text-muted-foreground">{provider.freshness}</span>
          </div>
        </div>
      </button>

      {#if open}
        <div class="grid gap-4 p-4 lg:grid-cols-[minmax(0,1fr)_minmax(18rem,24rem)]">
          <div class="space-y-4">
            <div class="grid gap-2 md:grid-cols-3">
              {#each providerDimensions(provider) as item (item.label)}
                <div class="min-w-0 rounded-md border p-3">
                  <div class="flex min-w-0 flex-wrap items-center justify-between gap-2">
                    <div class="shrink-0 text-xs font-medium text-muted-foreground">
                      {item.label}
                    </div>
                    <Badge
                      variant={dimensionVariant(item.value)}
                      class="max-w-full whitespace-normal text-center leading-4 [overflow-wrap:anywhere]"
                    >
                      {stateLabel(item.value.state)}
                    </Badge>
                  </div>
                  {#if item.value.detail}
                    <p class="mt-2 text-xs leading-5 text-muted-foreground">{item.value.detail}</p>
                  {/if}
                </div>
              {/each}
            </div>

            {#if provider.remediation.length > 0}
              <div class="rounded-md border bg-muted/20 p-3">
                <div class="text-xs font-medium text-muted-foreground">Next steps</div>
                <ul class="mt-2 space-y-1 text-sm">
                  {#each provider.remediation as step}
                    <li class="leading-5">{step}</li>
                  {/each}
                </ul>
              </div>
            {/if}
          </div>

          <div class="rounded-md border p-3">
            <div class="flex items-center justify-between gap-2">
              <div>
                <div class="text-sm font-semibold">Provider actions</div>
                <p class="text-xs text-muted-foreground">{enabledActionSummary(provider)}</p>
              </div>
            </div>
            <div class="mt-3 grid gap-2">
              {#each provider.actions as action (action.id)}
                {@const Icon = actionIcons[action.id]}
                <Button
                  size="sm"
                  variant={action.destructive ? "destructive" : action.enabled ? "default" : "outline"}
                  disabled={!action.enabled}
                  title={action.reason}
                  class="w-full justify-start"
                  onclick={() => handleAction(provider.provider_id, action)}
                >
                  <Icon />
                  {action.label}
                </Button>
              {/each}
            </div>
          </div>
        </div>

        <div class="border-t p-4">
          <div class="mb-3 flex flex-wrap items-center justify-between gap-2">
            <div>
              <h4 class="text-sm font-semibold">Runtime profiles</h4>
              <p class="text-xs text-muted-foreground">
                Profiles are persisted observations. Projects can pin one or use the preferred runtime.
              </p>
            </div>
            <Badge variant="outline">{provider.profiles.length}</Badge>
          </div>

          {#if provider.profiles.length === 0}
            <p class="rounded-md border p-3 text-sm text-muted-foreground">
              No profiles have been observed yet.
            </p>
          {:else}
            <ul class="divide-y rounded-md border">
              {#each provider.profiles as profile (profile.id)}
                {@const endpoint = endpointLabel(profile.endpoint_summary)}
                <li class="grid gap-3 p-3 md:grid-cols-[minmax(0,1fr)_auto] md:items-start">
                  <div class="min-w-0">
                    <div class="flex flex-wrap items-center gap-2">
                      <div class="min-w-0 flex-1">
                        <RuntimeIdentity presentation={entryPresentation(profile, provider)} compact />
                      </div>
                      {#if isPreferred(profile)}
                        <Badge variant="default" class="text-xs">
                          <CheckCircle2 />
                          Preferred
                        </Badge>
                      {/if}
                      {#each ownershipBadges(profile) as badge (badge.label)}
                        <Badge variant={badge.variant} class="text-xs">{badge.label}</Badge>
                      {/each}
                    </div>
                    <div class="mt-1 flex flex-wrap gap-x-4 gap-y-1 text-xs text-muted-foreground [overflow-wrap:anywhere]">
                      <span>{profile.provider_runtime_key}</span>
                      <span>{profile.freshness}</span>
                      {#if endpoint}
                        <span>{endpoint}</span>
                      {/if}
                    </div>
                    {#if profile.management.requires_recovery}
                      <p class="mt-2 text-xs text-destructive">
                        Studio can't prove it manages this built-in runtime. Lifecycle actions are
                        blocked. Remove the conflicting susun-runtime-default machine before using
                        Set up Susun Runtime.
                      </p>
                    {/if}
                    {#if profile.runtime_class === "built_in"}
                      <RuntimeResourcePanel
                        snapshot={resourceSnapshots[profile.id] ?? null}
                        loading={resourceLoading[profile.id] ?? false}
                        error={resourceErrors[profile.id] ?? null}
                        onrefresh={() => loadResources(profile)}
                        onnetworkchange={(mode) => handleResourceUpdate(profile, mode)}
                      />
                    {/if}
                    <RuntimeCompatibilityPanel {profile} />
                  </div>
                  <div class="flex flex-wrap items-center gap-2 md:justify-end">
                    {#if profile.runtime_class === "built_in"}
                      <Button size="sm" variant="outline" onclick={() => reviewDataScope(profile)}>
                        <SlidersHorizontal />
                        Recovery
                      </Button>
                      <Button
                        size="sm"
                        variant="outline"
                        disabled={!isPreferred(profile) || profile.connection.state !== "summarized"}
                        title={isPreferred(profile)
                          ? "Preview unused resources on this runtime."
                          : "Set this runtime as preferred before pruning it."}
                        onclick={() => reviewPrune(profile)}
                      >
                        <Trash2 />
                        Prune
                      </Button>
                    {/if}
                    {#if profile.management.can_forget}
                      <Button
                        size="sm"
                        variant="outline"
                        onclick={() => requestOwnershipAction(profile, "forget")}
                      >
                        Forget
                      </Button>
                    {/if}
                    <Button
                      size="sm"
                      variant={isPreferred(profile) ? "secondary" : "outline"}
                      disabled={isPreferred(profile) || !profile.management.can_select}
                      title={profile.management.can_select
                        ? undefined
                        : "This runtime is missing, so it can't be preferred."}
                      onclick={() => handleSelect(profile)}
                    >
                      {isPreferred(profile) ? "Preferred" : "Set preferred"}
                    </Button>
                  </div>
                </li>
              {/each}
            </ul>
          {/if}
        </div>
      {/if}
    </Card.Root>
  {/each}

  <Card.Root class="gap-0 overflow-hidden p-0">
    <div class="flex items-center justify-between border-b bg-muted/20 p-4">
      <div class="flex items-center gap-2">
        <TerminalSquare class="size-4 text-muted-foreground" />
        <h4 class="text-sm font-semibold">Runtime logs</h4>
      </div>
      <Badge variant="outline">{logsLoading ? "…" : logs.length}</Badge>
    </div>
    {#if logs.length === 0}
      <p class="p-4 text-sm text-muted-foreground">No runtime observations recorded.</p>
    {:else}
      <ul class="max-h-64 divide-y overflow-auto">
        {#each logs as line}
          <li class="grid gap-2 p-3 text-sm sm:grid-cols-[4rem_minmax(0,1fr)]">
            <Badge variant={line.level === "warn" ? "outline" : "secondary"} class="h-fit text-xs">
              {line.level}
            </Badge>
            <span class="min-w-0 text-muted-foreground [overflow-wrap:anywhere]">{line.message}</span>
          </li>
        {/each}
      </ul>
    {/if}
  </Card.Root>

  <RuntimeActionAudit />

  <RuntimeActionDialog
    request={runtimeActionRequest}
    bind:open={runtimeActionDialogOpen}
    oncompleted={refreshRuntime}
  />

  <Dialog.Root bind:open={ownershipDialogOpen}>
    <Dialog.Content class="sm:max-w-lg">
      <Dialog.Header>
        <Dialog.Title>
          {`Forget ${pendingOwnershipProfile?.display_name ?? "external runtime"}?`}
        </Dialog.Title>
        <Dialog.Description>
          Studio will remove only its saved profile metadata. The external runtime will not be
          stopped, reset, or deleted. Project bindings remain recorded, and the profile may
          reappear after the next provider scan if the runtime still exists.
        </Dialog.Description>
      </Dialog.Header>
      <Dialog.Footer>
        <Button
          type="button"
          variant="outline"
          disabled={ownershipDialogBusy}
          onclick={() => (ownershipDialogOpen = false)}>Cancel</Button
        >
        <Button
          type="button"
          variant="destructive"
          disabled={ownershipDialogBusy}
          onclick={confirmOwnershipAction}
        >
          {ownershipDialogBusy ? "Working..." : "Forget metadata"}
        </Button>
      </Dialog.Footer>
    </Dialog.Content>
  </Dialog.Root>

  <RuntimeMigrationDialog
    profiles={providers.flatMap((provider) => provider.profiles)}
    bind:open={migrationDialogOpen}
    oncompleted={refreshRuntime}
  />
  <RuntimeDataScopeDialog
    profile={dataScopeProfile}
    bind:open={dataScopeDialogOpen}
    oncompleted={refreshRuntime}
  />
  {#if pruneEngineId}
    <PruneDialog
      engineId={pruneEngineId}
      runtimeName={pruneProfile
        ? `${pruneProfile.display_name} (${pruneProfile.provider_runtime_key})`
        : undefined}
      bind:open={pruneDialogOpen}
      oncompleted={refreshRuntime}
    />
  {/if}
</div>
