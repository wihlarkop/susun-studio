<script lang="ts">
  import * as Card from "$lib/components/ui/card/index.js";
  import * as Dialog from "$lib/components/ui/dialog/index.js";
  import { Badge } from "$lib/components/ui/badge/index.js";
  import { Button } from "$lib/components/ui/button/index.js";
  import {
    adoptRuntimeProfile,
    forgetRuntimeProfile,
    readRuntimeLogs,
    readRuntimeStatus,
    runRuntimeAction,
    selectRuntimeProfile,
    type RuntimeAction,
    type RuntimeActionResult,
    type RuntimeDimension,
    type RuntimeEndpointSummary,
    type RuntimeLogLine,
    type RuntimeProfile,
    type RuntimeProviderStatus,
    type RuntimeStatus,
  } from "$lib/daemon/client";
  import {
    AlertCircle,
    CheckCircle2,
    ChevronRight,
    HardDrive,
    Play,
    RefreshCw,
    RotateCw,
    Server,
    Square,
    TerminalSquare,
    Wrench,
  } from "@lucide/svelte";

  let status = $state<RuntimeStatus | null>(null);
  let logs = $state<RuntimeLogLine[]>([]);
  let loading = $state(false);
  let actionResult = $state<RuntimeActionResult | null>(null);
  let errorMessage = $state<string | null>(null);
  let expandedProviders = $state<Set<string>>(new Set());
  let ownershipDialogOpen = $state(false);
  let ownershipDialogBusy = $state(false);
  let pendingOwnershipProfile = $state<RuntimeProfile | null>(null);
  let pendingOwnershipAction = $state<"adopt" | "forget" | null>(null);

  const actionIcons = {
    install: Wrench,
    init: HardDrive,
    start: Play,
    stop: Square,
    restart: RotateCw,
  } as const;

  const providers = $derived(status?.providers ?? []);
  const selectedProfiles = $derived(
    providers.flatMap((provider) => provider.profiles.filter((profile) => profile.is_selected)),
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

  $effect(() => {
    const controller = new AbortController();
    void refresh(controller.signal);
    return () => controller.abort();
  });

  async function refresh(signal?: AbortSignal) {
    loading = true;
    try {
      const [nextStatus, nextLogs] = await Promise.all([
        readRuntimeStatus({ signal }),
        readRuntimeLogs({ signal }),
      ]);
      status = nextStatus;
      logs = nextLogs;
      errorMessage = null;
    } catch (error) {
      if (!signal?.aborted) {
        errorMessage = error instanceof Error ? error.message : String(error);
      }
    } finally {
      loading = false;
    }
  }

  async function handleAction(providerId: string, action: RuntimeAction) {
    actionResult = await runRuntimeAction(providerId, action.id);
    await refresh();
  }

  async function handleSelect(profile: RuntimeProfile) {
    await selectRuntimeProfile(profile.id);
    await refresh();
  }

  async function handleForget(profile: RuntimeProfile) {
    try {
      await forgetRuntimeProfile(profile.id);
      errorMessage = null;
    } catch (error) {
      errorMessage = error instanceof Error ? error.message : String(error);
    }
    await refresh();
  }

  async function handleAdopt(profile: RuntimeProfile) {
    try {
      await adoptRuntimeProfile(profile.id);
      errorMessage = null;
    } catch (error) {
      errorMessage = error instanceof Error ? error.message : String(error);
    }
    await refresh();
  }

  function requestOwnershipAction(profile: RuntimeProfile, action: "adopt" | "forget") {
    pendingOwnershipProfile = profile;
    pendingOwnershipAction = action;
    ownershipDialogOpen = true;
  }

  async function confirmOwnershipAction() {
    if (!pendingOwnershipProfile || !pendingOwnershipAction) return;
    ownershipDialogBusy = true;
    try {
      if (pendingOwnershipAction === "adopt") {
        await handleAdopt(pendingOwnershipProfile);
      } else {
        await handleForget(pendingOwnershipProfile);
      }
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

  function profileStatus(profile: RuntimeProfile): string {
    return [profile.installation.state, profile.process.state, profile.connection.state]
      .filter(Boolean)
      .map(stateLabel)
      .join(" / ");
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

  function toggleProvider(providerId: string) {
    const next = new Set(expandedProviders);
    if (next.has(providerId)) {
      next.delete(providerId);
    } else {
      next.add(providerId);
    }
    expandedProviders = next;
  }

  function providerOpen(providerId: string): boolean {
    return expandedProviders.has(providerId);
  }
</script>

<div class="flex flex-col gap-4">
  <div class="flex flex-wrap items-start justify-between gap-3">
    <div class="max-w-3xl">
      <h3 class="text-lg font-semibold">Runtime</h3>
      <p class="text-sm text-muted-foreground">
        Manage local container providers and choose the profile Studio should use for project
        actions. Existing Docker-compatible engines can still be used as the platform default.
      </p>
    </div>
    <Button size="sm" variant="outline" disabled={loading} onclick={() => refresh()}>
      <RefreshCw />
      {loading ? "Checking" : "Recheck"}
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
      <div class="text-xs font-medium text-muted-foreground">Active profile</div>
      <div class="mt-2 flex min-w-0 items-center gap-2">
        <Server class="size-4 shrink-0 text-muted-foreground" />
        <span class="min-w-0 truncate text-sm">
          {selectedProfiles[0]?.display_name ?? "No profile selected"}
        </span>
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

  {#if actionResult}
    <div class="rounded-md border bg-muted/30 p-3">
      <div class="flex flex-wrap items-center gap-2">
        <Badge variant={actionResult.status === "failed" ? "destructive" : "secondary"}>
          {stateLabel(actionResult.status)}
        </Badge>
        <span class="text-sm">{actionResult.message}</span>
      </div>
      {#if actionResult.next_steps.length > 0}
        <p class="mt-1 text-xs text-muted-foreground">{actionResult.next_steps.join(" ")}</p>
      {/if}
    </div>
  {/if}

  {#if !status && !errorMessage}
    <div class="rounded-md border p-4 text-sm text-muted-foreground">
      Checking runtime providers...
    </div>
  {/if}

  <div class="flex flex-wrap items-end justify-between gap-2">
    <div>
      <h4 class="text-sm font-semibold">Runtime providers</h4>
      <p class="text-xs text-muted-foreground">
        Installed and available runtimes are listed together. Expand one to manage profiles and
        lifecycle actions.
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
        onclick={() => toggleProvider(provider.provider_id)}
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
                Profiles are persisted observations. Projects can pin one or use the active profile.
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
                      <span class="min-w-0 truncate text-sm font-medium">
                        {profile.display_name}
                      </span>
                      {#if profile.is_selected}
                        <Badge variant="default" class="text-xs">
                          <CheckCircle2 />
                          Active
                        </Badge>
                      {/if}
                      {#each ownershipBadges(profile) as badge (badge.label)}
                        <Badge variant={badge.variant} class="text-xs">{badge.label}</Badge>
                      {/each}
                      <Badge variant="secondary" class="text-xs">{profileStatus(profile)}</Badge>
                    </div>
                    <div class="mt-1 flex flex-wrap gap-x-4 gap-y-1 text-xs text-muted-foreground">
                      <span>{profile.provider_runtime_key}</span>
                      <span>{profile.freshness}</span>
                      {#if endpoint}
                        <span>{endpoint}</span>
                      {/if}
                    </div>
                    {#if profile.management.requires_recovery}
                      <p class="mt-2 text-xs text-destructive">
                        Studio can't prove it manages this built-in runtime. Lifecycle actions are
                        blocked until you recover it (adopt with no machine changes) or forget it.
                      </p>
                    {/if}
                  </div>
                  <div class="flex flex-wrap items-center gap-2 md:justify-end">
                    {#if profile.management.can_adopt}
                      <Button
                        size="sm"
                        variant="default"
                        onclick={() => requestOwnershipAction(profile, "adopt")}
                      >
                        Recover built-in
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
                      variant={profile.is_selected ? "secondary" : "outline"}
                      disabled={profile.is_selected || !profile.management.can_select}
                      title={profile.management.can_select
                        ? undefined
                        : "This runtime is missing, so it can't be made active."}
                      onclick={() => handleSelect(profile)}
                    >
                      {profile.is_selected ? "Active" : "Make active"}
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
      <Badge variant="outline">{logs.length}</Badge>
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

  <Dialog.Root bind:open={ownershipDialogOpen}>
    <Dialog.Content class="sm:max-w-lg">
      <Dialog.Header>
        <Dialog.Title>
          {pendingOwnershipAction === "adopt"
            ? `Recover ${pendingOwnershipProfile?.display_name ?? "built-in runtime"}?`
            : `Forget ${pendingOwnershipProfile?.display_name ?? "external runtime"}?`}
        </Dialog.Title>
        <Dialog.Description>
          {#if pendingOwnershipAction === "adopt"}
            Studio will record this existing runtime as Studio-managed and allow lifecycle,
            resource, and destructive runtime actions. This confirmation does not change the
            machine itself. Continue only if you trust this runtime and intend Studio to manage it.
          {:else}
            Studio will remove only its saved profile metadata. The external runtime will not be
            stopped, reset, or deleted. Project bindings remain recorded, and the profile may
            reappear after the next provider scan if the runtime still exists.
          {/if}
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
          variant={pendingOwnershipAction === "forget" ? "destructive" : "default"}
          disabled={ownershipDialogBusy}
          onclick={confirmOwnershipAction}
        >
          {ownershipDialogBusy
            ? "Working..."
            : pendingOwnershipAction === "adopt"
              ? "Record as Studio-managed"
              : "Forget metadata"}
        </Button>
      </Dialog.Footer>
    </Dialog.Content>
  </Dialog.Root>
</div>
