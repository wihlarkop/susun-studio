<script lang="ts">
  import * as Card from "$lib/components/ui/card/index.js";
  import { Badge } from "$lib/components/ui/badge/index.js";
  import { Button } from "$lib/components/ui/button/index.js";
  import { Separator } from "$lib/components/ui/separator/index.js";
  import { ChevronRight, LoaderCircle } from "@lucide/svelte";
  import StatusBadge from "./status-badge.svelte";
  import ExecDialog from "./exec-dialog.svelte";
  import RunDialog from "./run-dialog.svelte";
  import CopyDialog from "./copy-dialog.svelte";
  import ServiceOutput from "./service-output.svelte";
  import {
    readProjectSnapshot,
    serviceAction,
    waitService,
    readServicePorts,
    type ProjectSnapshot,
    type PortBinding,
    type StudioProject,
  } from "$lib/daemon/client";

  let { project }: { project: StudioProject | null } = $props();

  let snapshot = $state<ProjectSnapshot | null>(null);
  let snapshotError = $state<string | null>(null);
  let busy = $state<Record<string, string | null>>({});
  let inlineResult = $state<Record<string, string | null>>({});
  let portRows = $state<Record<string, PortBinding[] | null>>({});
  let portsLoading = $state<Record<string, boolean>>({});
  let execService = $state<string | null>(null);
  let execDialogOpen = $state(false);
  let runService = $state<string | null>(null);
  let runDialogOpen = $state(false);
  let copyTargetService = $state<string | null>(null);
  let copyDialogOpen = $state(false);

  function openExec(service: string) {
    execService = service;
    execDialogOpen = true;
  }

  function openRun(service: string) {
    runService = service;
    runDialogOpen = true;
  }

  function openCopy(service: string) {
    copyTargetService = service;
    copyDialogOpen = true;
  }

  let openOutputService = $state<string | null>(null);
  let outputTokens = $state<Record<string, number>>({});

  function toggleOutput(service: string) {
    openOutputService = openOutputService === service ? null : service;
  }

  async function refresh() {
    if (!project) return;
    try {
      snapshot = await readProjectSnapshot(project.id);
      snapshotError = null;
    } catch (error) {
      snapshotError = error instanceof Error ? error.message : String(error);
    }
  }

  $effect(() => {
    if (!project) return;
    refresh();
    const timer = setInterval(refresh, 5000);
    return () => clearInterval(timer);
  });

  const services = $derived(
    (project?.summary?.services ?? []).map((summary) => ({
      summary,
      containers: (snapshot?.containers ?? []).filter((c) => c.service === summary.name),
    })),
  );

  function hasRunning(containers: { state: string }[]): boolean {
    return containers.some((c) => c.state === "running");
  }

  async function act(service: string, action: "start" | "stop" | "restart") {
    if (!project) return;
    busy = { ...busy, [service]: action };
    inlineResult = { ...inlineResult, [service]: null };
    try {
      await serviceAction(project.id, service, action);
      await refresh();
      openOutputService = service;
      outputTokens = { ...outputTokens, [service]: (outputTokens[service] ?? 0) + 1 };
    } catch (error) {
      inlineResult = {
        ...inlineResult,
        [service]: `${action} failed: ${error instanceof Error ? error.message : String(error)}`,
      };
    } finally {
      busy = { ...busy, [service]: null };
    }
  }

  async function wait(service: string) {
    if (!project) return;
    busy = { ...busy, [service]: "wait" };
    inlineResult = { ...inlineResult, [service]: null };
    try {
      const result = await waitService(project.id, service);
      inlineResult = { ...inlineResult, [service]: `exited with code ${result.exit_code}` };
      await refresh();
    } catch (error) {
      inlineResult = {
        ...inlineResult,
        [service]: `wait failed: ${error instanceof Error ? error.message : String(error)}`,
      };
    } finally {
      busy = { ...busy, [service]: null };
    }
  }

  async function loadPorts(service: string) {
    if (!project) return;
    portsLoading = { ...portsLoading, [service]: true };
    try {
      const response = await readServicePorts(project.id, service);
      portRows = { ...portRows, [service]: response.bindings };
    } catch (error) {
      inlineResult = {
        ...inlineResult,
        [service]: `ports failed: ${error instanceof Error ? error.message : String(error)}`,
      };
    } finally {
      portsLoading = { ...portsLoading, [service]: false };
    }
  }
</script>

{#if !project}
  <p class="text-muted-foreground text-sm">Select a project to see its services.</p>
{:else if !project.summary || project.summary.services.length === 0}
  <p class="text-muted-foreground text-sm">This project has no services in its summary.</p>
{:else}
  <div class="flex flex-col gap-4">
    {#if snapshotError}
      <div class="border-warning/40 bg-warning/10 rounded-md border p-3 text-sm">
        Engine unreachable — live state unavailable. Actions are disabled until it reconnects.
      </div>
    {/if}

    {#each services as { summary, containers } (summary.name)}
      {@const running = hasRunning(containers)}
      {@const hasContainers = containers.length > 0}
      {@const disabledReason = snapshotError
        ? "Engine unreachable"
        : !hasContainers
          ? "No containers — run Up first"
          : null}
      <Card.Root class="gap-3 p-4">
        <div class="flex items-center justify-between gap-2">
          <div>
            <div class="flex items-center gap-2">
              <h4 class="font-semibold">{summary.name}</h4>
              {#if containers.length > 0}
                {#each containers as container (container.id)}
                  <StatusBadge status={container.state} />
                  {#if container.health}
                    <StatusBadge status={container.health} label={`health: ${container.health}`} />
                  {/if}
                {/each}
              {:else}
                <Badge variant="outline">no containers</Badge>
              {/if}
            </div>
            <p class="text-muted-foreground mt-1 font-mono text-xs [overflow-wrap:anywhere]">
              {summary.image ?? (summary.has_build ? "built from source" : "no image")}
            </p>
          </div>
          {#if summary.profiles.length > 0}
            <div class="flex flex-wrap gap-1">
              {#each summary.profiles as profile (profile)}
                <Badge variant="secondary" class="text-xs">{profile}</Badge>
              {/each}
            </div>
          {/if}
        </div>

        <div class="grid grid-cols-2 gap-2 text-xs sm:grid-cols-5">
          <div>
            <span class="text-muted-foreground">Ports</span>
            <p class="font-medium tabular-nums">{summary.port_count}</p>
          </div>
          <div>
            <span class="text-muted-foreground">Volumes</span>
            <p class="font-medium tabular-nums">{summary.volume_count}</p>
          </div>
          <div>
            <span class="text-muted-foreground">Networks</span>
            <p class="font-medium tabular-nums">{summary.network_count}</p>
          </div>
          <div>
            <span class="text-muted-foreground">Configs</span>
            <p class="font-medium tabular-nums">{summary.config_count}</p>
          </div>
          <div>
            <span class="text-muted-foreground">Secrets</span>
            <p class="font-medium tabular-nums">{summary.secret_count}</p>
          </div>
        </div>

        {#if summary.secrets.length > 0}
          <div class="flex flex-wrap gap-1">
            {#each summary.secrets as secretName (secretName)}
              <Badge variant="outline" class="text-xs">🔒 {secretName}</Badge>
            {/each}
          </div>
        {/if}

        {#if summary.dependencies.length > 0}
          <p class="text-xs">
            <span class="text-muted-foreground">Depends on:</span>
            {summary.dependencies.join(", ")}
          </p>
        {/if}

        <Separator />

        <div class="flex flex-wrap items-center gap-2">
          <Button
            size="sm"
            disabled={!hasContainers || running || busy[summary.name] === "start" || !!snapshotError}
            title={disabledReason ?? undefined}
            onclick={() => act(summary.name, "start")}
          >
            {#if busy[summary.name] === "start"}<LoaderCircle class="animate-spin" />{/if}
            Start
          </Button>
          <Button
            size="sm"
            variant="outline"
            disabled={!running || busy[summary.name] === "stop" || !!snapshotError}
            title={!running ? "No running container" : undefined}
            onclick={() => act(summary.name, "stop")}
          >
            {#if busy[summary.name] === "stop"}<LoaderCircle class="animate-spin" />{/if}
            Stop
          </Button>
          <Button
            size="sm"
            variant="outline"
            disabled={!running || busy[summary.name] === "restart" || !!snapshotError}
            title={!running ? "No running container" : undefined}
            onclick={() => act(summary.name, "restart")}
          >
            {#if busy[summary.name] === "restart"}<LoaderCircle class="animate-spin" />{/if}
            Restart
          </Button>
          <Button
            size="sm"
            variant="outline"
            disabled={!hasContainers || busy[summary.name] === "wait" || !!snapshotError}
            title={disabledReason ?? undefined}
            onclick={() => wait(summary.name)}
          >
            {#if busy[summary.name] === "wait"}<LoaderCircle class="animate-spin" />{/if}
            Wait
          </Button>
          <Button
            size="sm"
            variant="outline"
            disabled={!hasContainers || portsLoading[summary.name] || !!snapshotError}
            title={disabledReason ?? undefined}
            onclick={() => loadPorts(summary.name)}
          >
            {#if portsLoading[summary.name]}<LoaderCircle class="animate-spin" />{/if}
            Ports
          </Button>
          <Button
            size="sm"
            variant="outline"
            disabled={!running || !!snapshotError}
            title={!running ? "Needs a running container" : undefined}
            onclick={() => openExec(summary.name)}
          >
            Exec
          </Button>
          <Button
            size="sm"
            variant="outline"
            disabled={!summary.image || !!snapshotError}
            title={
              !summary.image
                ? "Build-only service; images can't be built yet (BollardEngine limitation)"
                : "Runs with service env/volumes/networks; no published ports, no config/secret mounts"
            }
            onclick={() => openRun(summary.name)}
          >
            Run
          </Button>
          <Button
            size="sm"
            variant="outline"
            disabled={!hasContainers || !!snapshotError}
            title={disabledReason ?? undefined}
            onclick={() => openCopy(summary.name)}
          >
            Copy
          </Button>
        </div>

        {#if !snapshotError && !hasContainers}
          <p class="bg-warning/10 text-warning-foreground rounded-md px-2 py-1 text-xs">
            No containers yet — click <b>Up</b> above to create them.
          </p>
        {/if}

        {#if portRows[summary.name]}
          <div class="bg-muted/40 rounded-md border p-2 font-mono text-xs">
            {#if portRows[summary.name]?.length === 0}
              <span class="text-muted-foreground">No published ports.</span>
            {:else}
              {#each portRows[summary.name] ?? [] as binding (`${binding.private_port}-${binding.protocol}-${binding.host_port}`)}
                <div>
                  {binding.private_port}/{binding.protocol} → {binding.host_ip ?? "0.0.0.0"}:{binding.host_port}
                </div>
              {/each}
            {/if}
          </div>
        {/if}

        {#if inlineResult[summary.name]}
          <p class="text-destructive text-xs">{inlineResult[summary.name]}</p>
        {/if}

        <button
          type="button"
          class="text-muted-foreground hover:text-foreground flex w-fit items-center gap-1 text-xs font-medium"
          onclick={() => toggleOutput(summary.name)}
        >
          <ChevronRight
            class="size-3 transition-transform {openOutputService === summary.name
              ? 'rotate-90'
              : ''}"
          />
          Output
        </button>
        {#if project}
          <ServiceOutput
            {project}
            service={summary.name}
            open={openOutputService === summary.name}
            autoStartToken={outputTokens[summary.name] ?? 0}
          />
        {/if}
      </Card.Root>
    {/each}
  </div>

  {#if execService && project}
    <ExecDialog {project} service={execService} bind:open={execDialogOpen} />
  {/if}
  {#if runService && project}
    <RunDialog {project} service={runService} bind:open={runDialogOpen} />
  {/if}
  {#if copyTargetService && project}
    <CopyDialog {project} service={copyTargetService} bind:open={copyDialogOpen} />
  {/if}
{/if}
