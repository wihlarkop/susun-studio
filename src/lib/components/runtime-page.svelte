<script lang="ts">
  import * as Card from "$lib/components/ui/card/index.js";
  import { Badge } from "$lib/components/ui/badge/index.js";
  import { Button } from "$lib/components/ui/button/index.js";
  import {
    readRuntimeLogs,
    readRuntimeStatus,
    runRuntimeAction,
    selectRuntimeProfile,
    type RuntimeAction,
    type RuntimeDimension,
    type RuntimeEndpointSummary,
    type RuntimeLogLine,
    type RuntimeProfile,
    type RuntimeProviderStatus,
    type RuntimeStatus,
  } from "$lib/daemon/client";
  import { HardDrive, Play, RefreshCw, RotateCw, Square, Wrench } from "@lucide/svelte";

  let status = $state<RuntimeStatus | null>(null);
  let logs = $state<RuntimeLogLine[]>([]);
  let loading = $state(false);
  let actionMessage = $state<string | null>(null);
  let errorMessage = $state<string | null>(null);

  const actionIcons = {
    install: Wrench,
    init: HardDrive,
    start: Play,
    stop: Square,
    restart: RotateCw,
  } as const;

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
    const result = await runRuntimeAction(providerId, action.id);
    actionMessage = `${result.message} ${result.next_steps.join(" ")}`;
    await refresh();
  }

  async function handleSelect(profile: RuntimeProfile) {
    await selectRuntimeProfile(profile.id);
    await refresh();
  }

  function dimensionVariant(dimension: RuntimeDimension): "default" | "secondary" | "outline" {
    if (["installed", "running", "reachable"].includes(dimension.state)) return "default";
    if (["unknown", "not_applicable"].includes(dimension.state)) return "secondary";
    return "outline";
  }

  function profileStatus(profile: RuntimeProfile): string {
    return [profile.installation.state, profile.process.state, profile.connection.state]
      .filter(Boolean)
      .join(" / ")
      .replaceAll("_", " ");
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
      { label: "Installation", value: provider.installation },
      { label: "Process", value: provider.process },
      { label: "Connection", value: provider.connection },
    ];
  }
</script>

<div class="flex flex-col gap-4">
  <div class="flex items-center justify-between gap-3">
    <div>
      <h3 class="text-lg font-semibold">Runtime</h3>
      <p class="text-sm text-muted-foreground">
        Studio can guide you through installing and managing a container runtime. Existing
        Docker-compatible engines keep working through the Engines page either way.
      </p>
    </div>
    <Button size="sm" variant="outline" disabled={loading} onclick={() => refresh()}>
      <RefreshCw />
      Recheck
    </Button>
  </div>

  {#if errorMessage}
    <p class="text-sm text-destructive">{errorMessage}</p>
  {/if}

  {#if actionMessage}
    <p class="text-sm text-muted-foreground">{actionMessage}</p>
  {/if}

  {#each status?.providers ?? [] as provider (provider.provider_id)}
    <Card.Root class="gap-4 p-4">
      <div class="flex flex-wrap items-center gap-2">
        <h4 class="text-base font-semibold">{provider.display_name}</h4>
        <Badge variant={provider.supported ? "default" : "outline"}>
          {provider.supported ? "Supported target" : "Unsupported platform"}
        </Badge>
        <Badge variant="secondary">{provider.platform}</Badge>
        <span class="text-xs text-muted-foreground">{provider.freshness}</span>
      </div>

      <p class="text-sm text-muted-foreground">{provider.summary}</p>

      <div class="grid gap-2 md:grid-cols-3">
        {#each providerDimensions(provider) as item (item.label)}
          <div class="rounded-md border p-3">
            <div class="text-xs font-medium text-muted-foreground">{item.label}</div>
            <div class="mt-2 flex items-center gap-2">
              <Badge variant={dimensionVariant(item.value)}>
                {item.value.state.replace("_", " ")}
              </Badge>
            </div>
            {#if item.value.detail}
              <p class="mt-2 text-xs text-muted-foreground">{item.value.detail}</p>
            {/if}
          </div>
        {/each}
      </div>

      <div class="flex flex-wrap gap-2">
        {#each provider.actions as action (action.id)}
          {@const Icon = actionIcons[action.id]}
          <Button
            size="sm"
            variant={action.destructive ? "destructive" : "outline"}
            disabled={!action.enabled}
            title={action.reason}
            onclick={() => handleAction(provider.provider_id, action)}
          >
            <Icon />
            {action.label}
          </Button>
        {/each}
      </div>

      {#if provider.remediation.length > 0}
        <div class="rounded-md border bg-muted/30 p-3">
          <div class="text-xs font-medium text-muted-foreground">Next steps</div>
          <ul class="mt-2 space-y-1 text-sm">
            {#each provider.remediation as step}
              <li>{step}</li>
            {/each}
          </ul>
        </div>
      {/if}

      <div class="flex items-center justify-between">
        <h4 class="text-sm font-semibold">Profiles</h4>
        <Badge variant="outline">{provider.profiles.length}</Badge>
      </div>
      {#if provider.profiles.length === 0}
        <p class="text-sm text-muted-foreground">No profiles have been observed yet.</p>
      {:else}
        <ul class="space-y-2">
          {#each provider.profiles as profile (profile.id)}
            {@const endpoint = endpointLabel(profile.endpoint_summary)}
            <li class="rounded-md border p-3">
              <div class="flex flex-wrap items-center justify-between gap-2">
                <div class="flex flex-wrap items-center gap-2">
                  <span class="text-sm font-medium">{profile.display_name}</span>
                  {#if profile.is_selected}
                    <Badge variant="default" class="text-xs">Selected</Badge>
                  {/if}
                  <Badge variant="secondary" class="text-xs">{profileStatus(profile)}</Badge>
                </div>
                <Button
                  size="sm"
                  variant="outline"
                  disabled={profile.is_selected}
                  onclick={() => handleSelect(profile)}
                >
                  Select
                </Button>
              </div>
              <div class="mt-2 flex flex-wrap gap-x-4 gap-y-1 text-xs text-muted-foreground">
                <span>{profile.provider_runtime_key}</span>
                <span>{profile.freshness}</span>
                {#if endpoint}
                  <span>{endpoint}</span>
                {/if}
              </div>
            </li>
          {/each}
        </ul>
      {/if}
    </Card.Root>
  {/each}

  <Card.Root class="gap-3 p-4">
    <div class="flex items-center justify-between">
      <h4 class="text-sm font-semibold">Runtime logs</h4>
      <Badge variant="outline">{logs.length}</Badge>
    </div>
    {#if logs.length === 0}
      <p class="text-sm text-muted-foreground">No runtime observations recorded.</p>
    {:else}
      <ul class="space-y-1 text-sm">
        {#each logs as line}
          <li class="flex gap-2">
            <Badge variant={line.level === "warn" ? "outline" : "secondary"} class="h-fit text-xs">
              {line.level}
            </Badge>
            <span class="text-muted-foreground">{line.message}</span>
          </li>
        {/each}
      </ul>
    {/if}
  </Card.Root>
</div>
