<script lang="ts">
  import * as Card from "$lib/components/ui/card/index.js";
  import * as Dialog from "$lib/components/ui/dialog/index.js";
  import { Badge } from "$lib/components/ui/badge/index.js";
  import { Button } from "$lib/components/ui/button/index.js";
  import RuntimeMigrationDialog from "$lib/components/runtime-migration-dialog.svelte";
  import RuntimeDataScopeDialog from "$lib/components/runtime-data-scope-dialog.svelte";
  import RuntimeActionAudit from "$lib/components/runtime-action-audit.svelte";
  import {
    cancelRuntimePlan,
    executeRuntimePlan,
    forgetRuntimeProfile,
    readRuntimeLogs,
    readRuntimeStatus,
    prepareRuntimeAction,
    selectRuntimeProfile,
    type RuntimeAction,
    type RuntimeActionResult,
    type RuntimeDimension,
    type RuntimeEndpointSummary,
    type RuntimeLogLine,
    type RuntimeProfile,
    type RuntimeProviderStatus,
    type RuntimeStatus,
    type TrustedRuntimePlan,
  } from "$lib/daemon/client";
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
  let pendingOwnershipAction = $state<"forget" | null>(null);
  let trustedPlan = $state<TrustedRuntimePlan | null>(null);
  let trustedPlanDialogOpen = $state(false);
  let trustedPlanBusy = $state(false);
  let migrationDialogOpen = $state(false);
  let dataScopeDialogOpen = $state(false);
  let dataScopeProfile = $state<RuntimeProfile | null>(null);

  const actionIcons = {
    install: Wrench,
    setup: HardDrive,
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

  $effect(() => {
    const controller = new AbortController();
    void refresh(controller.signal);
    return () => controller.abort();
  });

  $effect(() => {
    if (!trustedPlanDialogOpen && trustedPlan && !trustedPlanBusy) {
      void cancelPreparedPlan();
    }
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
    try {
      const prepared = await prepareRuntimeAction(providerId, action.id);
      if (prepared.result) {
        actionResult = prepared.result;
        await refresh();
        return;
      }
      if (prepared.plan) {
        trustedPlan = prepared.plan;
        trustedPlanDialogOpen = true;
      }
    } catch (error) {
      errorMessage = error instanceof Error ? error.message : String(error);
    }
  }

  async function executePreparedPlan() {
    if (!trustedPlan) return;
    trustedPlanBusy = true;
    try {
      actionResult = await executeRuntimePlan(trustedPlan.plan_id);
      trustedPlanDialogOpen = false;
      trustedPlan = null;
      await refresh();
    } catch (error) {
      errorMessage = error instanceof Error ? error.message : String(error);
    } finally {
      trustedPlanBusy = false;
    }
  }

  async function cancelPreparedPlan() {
    if (!trustedPlan) {
      trustedPlanDialogOpen = false;
      return;
    }
    trustedPlanBusy = true;
    try {
      actionResult = await cancelRuntimePlan(trustedPlan.plan_id);
      trustedPlanDialogOpen = false;
      trustedPlan = null;
    } catch (error) {
      errorMessage = error instanceof Error ? error.message : String(error);
    } finally {
      trustedPlanBusy = false;
    }
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

  function showExistingRuntimes() {
    expandedProviders = new Set(providers.map((provider) => provider.provider_id));
  }

  function reviewDataScope(profile: RuntimeProfile) {
    dataScopeProfile = profile;
    dataScopeDialogOpen = true;
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
    <Button size="sm" variant="outline" onclick={() => (migrationDialogOpen = true)}>
      <ArrowRightLeft />
      Migrate projects
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

  {#if status && !managedBuiltIn && podmanProvider}
    <div class="grid gap-4 border-y py-4 md:grid-cols-[minmax(0,1fr)_auto] md:items-center">
      <div class="min-w-0">
        <div class="flex flex-wrap items-center gap-2">
          <Server class="size-4 text-primary" />
          <h4 class="text-sm font-semibold">Susun Runtime</h4>
          <Badge variant="secondary">Recommended</Badge>
        </div>
        <p class="mt-1 text-sm text-muted-foreground">
          A dedicated local runtime managed by Studio. Powered by Podman.
        </p>
      </div>
      <div class="flex flex-wrap gap-2 md:justify-end">
        <Button size="sm" variant="outline" onclick={showExistingRuntimes}>
          Use existing runtime
        </Button>
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
                      {#if profile.runtime_class === "built_in"}
                        <span class="text-xs text-muted-foreground">Powered by Podman</span>
                      {/if}
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
                        blocked. Remove the conflicting susun-runtime-default machine before using
                        Set up Susun Runtime.
                      </p>
                    {/if}
                  </div>
                  <div class="flex flex-wrap items-center gap-2 md:justify-end">
                    {#if profile.runtime_class === "built_in"}
                      <Button size="sm" variant="outline" onclick={() => reviewDataScope(profile)}>
                        <SlidersHorizontal />
                        Data scope
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

  <RuntimeActionAudit />

  <Dialog.Root bind:open={trustedPlanDialogOpen}>
    <Dialog.Content class="sm:max-w-lg">
      <Dialog.Header>
        <Dialog.Title>{trustedPlan?.label ?? "Approve runtime action"}</Dialog.Title>
        <Dialog.Description>
          Review the exact consequence before allowing this single-use runtime plan.
        </Dialog.Description>
      </Dialog.Header>
      {#if trustedPlan}
        <div class="grid gap-3 text-sm">
          <div class="grid gap-1">
            <span class="font-medium">Consequence</span>
            <span class="text-muted-foreground">{trustedPlan.consequence}</span>
          </div>
          <div class="grid gap-1">
            <span class="font-medium">Verified operation</span>
            <span class="text-muted-foreground">{trustedPlan.command_summary}</span>
          </div>
          {#if trustedPlan.software_provenance}
            <dl class="grid grid-cols-[minmax(7rem,auto)_minmax(0,1fr)] gap-x-4 gap-y-1 border-y py-3 text-xs">
              <dt class="text-muted-foreground">Package</dt>
              <dd class="min-w-0 font-mono [overflow-wrap:anywhere]">
                {trustedPlan.software_provenance.package_id}
              </dd>
              <dt class="text-muted-foreground">Source</dt>
              <dd class="min-w-0 [overflow-wrap:anywhere]">
                {trustedPlan.software_provenance.source} · {trustedPlan.software_provenance.source_url}
              </dd>
              <dt class="text-muted-foreground">Expected publisher</dt>
              <dd>{trustedPlan.software_provenance.expected_publisher}</dd>
              <dt class="text-muted-foreground">Version</dt>
              <dd>{trustedPlan.software_provenance.version_intent}</dd>
              <dt class="text-muted-foreground">Restart impact</dt>
              <dd>{trustedPlan.software_provenance.restart_impact}</dd>
            </dl>
          {/if}
          <div class="flex flex-wrap gap-2">
            <Badge variant={trustedPlan.destructive ? "destructive" : "secondary"}>
              {trustedPlan.destructive ? "Destructive" : "Runtime mutation"}
            </Badge>
            <Badge variant="outline">
              {trustedPlan.elevation === "os_mediated_consent"
                ? "Administrator consent expected"
                : "Current user"}
            </Badge>
            <Badge variant="outline">Expires in {trustedPlan.expires_in_seconds}s</Badge>
          </div>
          <p class="text-xs text-muted-foreground">
            Executable paths, arguments, environment values, and credentials are intentionally
            hidden. They are fixed by Studio and cannot be changed from this dialog.
          </p>
        </div>
      {/if}
      <Dialog.Footer>
        <Button type="button" variant="outline" disabled={trustedPlanBusy} onclick={cancelPreparedPlan}>
          Cancel
        </Button>
        <Button
          type="button"
          variant={trustedPlan?.destructive ? "destructive" : "default"}
          disabled={trustedPlanBusy || !trustedPlan}
          onclick={executePreparedPlan}
        >
          {trustedPlanBusy ? "Working..." : "Approve and run"}
        </Button>
      </Dialog.Footer>
    </Dialog.Content>
  </Dialog.Root>

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
    oncompleted={() => refresh()}
  />
  <RuntimeDataScopeDialog profile={dataScopeProfile} bind:open={dataScopeDialogOpen} />
</div>
