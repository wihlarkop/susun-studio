<script lang="ts">
  import * as Table from "$lib/components/ui/table/index.js";
  import { Button } from "$lib/components/ui/button/index.js";
  import { RefreshCw } from "@lucide/svelte";
  import StatusBadge from "./status-badge.svelte";
  import ArtifactsStateBanner from "./artifacts-state-banner.svelte";
  import {
    readProjectBuildTargets,
    listProjectJobs,
    startImageBuild,
    cancelJob,
    readJob,
    type BuildTargetsResponse,
    type StudioJob,
    type StudioProject,
  } from "$lib/daemon/client";
  import { resolveArtifactViewState } from "$lib/artifacts/workspace-state";
  import { toArtifactRequestError } from "$lib/artifacts/fetch-error";
  import {
    applyLoadError,
    applyLoadSuccess,
    initialScopedFetchState,
    resetForNewEngine,
    withLoading,
  } from "$lib/artifacts/scoped-fetch";
  import { isBuildJobActive, isImageBuildResult, visibleBuildProgress } from "$lib/jobs/build-job";
  import { relativeTime } from "$lib/utils";

  let {
    engineId,
    connected,
    projects,
  }: { engineId: string; connected: boolean; projects: StudioProject[] } = $props();

  let selectedProjectId = $state<string | null>(null);
  let targetsState = $state(initialScopedFetchState<BuildTargetsResponse>());
  let buildsState = $state(initialScopedFetchState<StudioJob[]>());
  let expandedId = $state<string | null>(null);
  let detailCache = $state<Record<string, StudioJob>>({});
  let startingService = $state<string | null>(null);
  let startError = $state<string | null>(null);
  // Plain (non-reactive) counter — see the other artifacts tabs for why this
  // must never be read back out of *State inside the effect below.
  let generation = 0;

  // Runs only in an async continuation, after the request's first `await` —
  // never synchronously inside the effect.
  async function loadTargets(projectId: string, signal: AbortSignal, requestGeneration: number) {
    try {
      const result = await readProjectBuildTargets(projectId, { signal });
      if (signal.aborted) return;
      targetsState = applyLoadSuccess(targetsState, requestGeneration, result);
    } catch (caught) {
      if (signal.aborted) return;
      targetsState = applyLoadError(targetsState, requestGeneration, toArtifactRequestError(caught));
    }
  }

  async function loadBuilds(projectId: string, signal: AbortSignal, requestGeneration: number) {
    try {
      const jobs = await listProjectJobs(projectId, { signal });
      if (signal.aborted) return;
      const builds = jobs.filter((job) => job.kind === "image_build");
      buildsState = applyLoadSuccess(buildsState, requestGeneration, builds);
    } catch (caught) {
      if (signal.aborted) return;
      buildsState = applyLoadError(buildsState, requestGeneration, toArtifactRequestError(caught));
    }
  }

  // Re-arms on engine change, project selection change, or connection
  // change, so a response for a target the user has since switched away
  // from can never be applied to this one. Also owns the live-status poll:
  // one interval, created and torn down together with everything else here,
  // so an engine/project switch can never leave an orphaned poll running
  // for the previous selection.
  $effect(() => {
    const id = engineId;
    const isConnected = connected;
    const projectId = selectedProjectId;

    generation += 1;
    const myGeneration = generation;
    detailCache = {};
    expandedId = null;
    startError = null;

    const controller = new AbortController();
    const hasTarget = isConnected && projectId !== null;
    targetsState = resetForNewEngine(myGeneration, hasTarget);
    buildsState = resetForNewEngine(myGeneration, hasTarget);
    if (isConnected && projectId) {
      void loadTargets(projectId, controller.signal, myGeneration);
      void loadBuilds(projectId, controller.signal, myGeneration);
    }

    // Poll only while at least one build for this project is still active
    // (queued/running) — never an unconditional background loop.
    const timer = setInterval(() => {
      if (!isConnected || !projectId) return;
      if (!buildsState.data?.some((job) => isBuildJobActive(job.status))) return;
      const pollController = new AbortController();
      void loadBuilds(projectId, pollController.signal, myGeneration);
    }, 4000);

    void id;
    return () => {
      controller.abort();
      clearInterval(timer);
    };
  });

  function refresh() {
    if (!selectedProjectId) return;
    targetsState = withLoading(targetsState);
    buildsState = withLoading(buildsState);
    const controller = new AbortController();
    void loadTargets(selectedProjectId, controller.signal, targetsState.generation);
    void loadBuilds(selectedProjectId, controller.signal, buildsState.generation);
  }

  async function startBuild(serviceName: string) {
    if (!selectedProjectId || startingService) return;
    startingService = serviceName;
    startError = null;
    try {
      await startImageBuild(selectedProjectId, serviceName);
      refresh();
    } catch (caught) {
      startError = toArtifactRequestError(caught).message;
    } finally {
      startingService = null;
    }
  }

  async function cancelBuild(jobId: string) {
    try {
      await cancelJob(jobId);
    } finally {
      refresh();
    }
  }

  async function toggleDetail(jobId: string) {
    if (expandedId === jobId) {
      expandedId = null;
      return;
    }
    expandedId = jobId;
    if (detailCache[jobId]) return;
    try {
      const detail = await readJob(jobId);
      detailCache = { ...detailCache, [jobId]: detail };
    } catch {
      // Leave undetailed — the row still shows its list-level status; the
      // user can retry by collapsing and re-expanding.
    }
  }

  const anyUnsupportedTargets = $derived(
    targetsState.data !== null &&
      targetsState.data.services.length > 0 &&
      targetsState.data.services.every((service) => !service.supported),
  );

  const viewState = $derived(
    resolveArtifactViewState({
      connected,
      loading: targetsState.loading || buildsState.loading,
      hasData: targetsState.data !== null,
      error: targetsState.error ?? buildsState.error,
      capability: anyUnsupportedTargets ? "unsupported" : null,
      itemCount: targetsState.data?.services.length ?? null,
    }),
  );
</script>

<div class="flex flex-col gap-3">
  <div class="flex flex-wrap items-center justify-between gap-2">
    <label class="flex items-center gap-2 text-sm">
      <span class="text-muted-foreground">Project</span>
      <select
        bind:value={selectedProjectId}
        class="border-input rounded-md border bg-transparent bg-none px-2 py-1"
      >
        <option value={null}>Select a project…</option>
        {#each projects as project (project.id)}
          <option value={project.id}>{project.name}</option>
        {/each}
      </select>
    </label>
    {#if selectedProjectId}
      <Button size="sm" variant="outline" disabled={targetsState.loading || buildsState.loading} onclick={refresh}>
        <RefreshCw class={targetsState.loading || buildsState.loading ? "animate-spin" : undefined} />
        Refresh
      </Button>
    {/if}
  </div>

  {#if !selectedProjectId}
    <p class="text-sm text-muted-foreground">
      Select a project to see its build-declared services and build jobs.
    </p>
  {:else if viewState.kind === "ready" || viewState.kind === "refreshing" || viewState.kind === "stale" || viewState.kind === "empty"}
    <div class="flex flex-col gap-4">
      {#if viewState.kind === "stale"}
        <p class="text-xs text-destructive">
          Couldn't refresh ({viewState.error.message}). Showing the last known data.
        </p>
      {/if}

      {#if startError}
        <p class="text-xs text-destructive">{startError}</p>
      {/if}

      <div class="flex flex-col gap-2">
        <p class="text-sm font-medium">Build targets</p>
        {#if targetsState.data && targetsState.data.services.length > 0}
          <div class="flex flex-wrap gap-2">
            {#each targetsState.data.services as target (target.service_name)}
              <div class="flex items-center gap-2 rounded-md border px-3 py-2 text-sm">
                <span class="font-mono">{target.service_name}</span>
                {#if !target.supported}
                  <span class="text-xs text-muted-foreground" title="This build declares secrets or SSH forwarding, which Studio does not support yet.">
                    unsupported
                  </span>
                {/if}
                <Button
                  size="sm"
                  variant="outline"
                  disabled={!target.supported || startingService !== null}
                  onclick={() => startBuild(target.service_name)}
                >
                  {startingService === target.service_name ? "Starting…" : "Build"}
                </Button>
              </div>
            {/each}
          </div>
        {:else}
          <p class="text-sm text-muted-foreground">
            This project has no build-declared services.
          </p>
        {/if}
      </div>

      <div class="flex flex-col gap-2">
        <p class="text-sm font-medium">Build jobs</p>
        {#if buildsState.data && buildsState.data.length > 0}
          <Table.Root>
            <Table.Header>
              <Table.Row>
                <Table.Head>Service</Table.Head>
                <Table.Head>Status</Table.Head>
                <Table.Head>When</Table.Head>
                <Table.Head class="text-right">Actions</Table.Head>
              </Table.Row>
            </Table.Header>
            <Table.Body>
              {#each buildsState.data as job (job.id)}
                <Table.Row class="cursor-pointer" onclick={() => toggleDetail(job.id)}>
                  <Table.Cell class="font-mono">{job.service_name ?? "—"}</Table.Cell>
                  <Table.Cell><StatusBadge status={job.status} /></Table.Cell>
                  <Table.Cell class="text-xs text-muted-foreground">
                    {relativeTime(job.created_at_ms)}
                  </Table.Cell>
                  <Table.Cell class="text-right">
                    {#if isBuildJobActive(job.status)}
                      <Button
                        size="sm"
                        variant="outline"
                        onclick={(event) => {
                          event.stopPropagation();
                          cancelBuild(job.id);
                        }}
                      >
                        Cancel
                      </Button>
                    {/if}
                  </Table.Cell>
                </Table.Row>
                {#if expandedId === job.id}
                  <Table.Row>
                    <Table.Cell colspan={4} class="bg-muted/40 whitespace-normal">
                      {@const detail = detailCache[job.id]}
                      <div class="flex flex-col gap-2 py-1 text-xs">
                        {#if !detail}
                          <p class="text-muted-foreground">Loading detail…</p>
                        {:else}
                          {#if detail.error}
                            <p class="text-destructive">
                              {detail.error}
                              {#if detail.error_code}<span class="font-mono">[{detail.error_code}]</span>{/if}
                            </p>
                          {/if}
                          {#if detail.result && isImageBuildResult(detail.result)}
                            <div>
                              <span class="text-muted-foreground">Image</span><br />
                              <span class="font-mono break-all">{detail.result.image_reference}</span>
                            </div>
                          {/if}
                          {#if detail.progress && detail.progress.length > 0}
                            {@const windowed = visibleBuildProgress(detail.progress, 200)}
                            <div class="flex flex-col gap-1">
                              {#if windowed.hiddenCount > 0}
                                <p class="text-muted-foreground">
                                  {windowed.hiddenCount} earlier entries hidden.
                                </p>
                              {/if}
                              <div class="max-h-64 overflow-y-auto rounded-md border">
                                {#each windowed.visible as entry (entry.sequence)}
                                  {#if entry.kind === "vertex_log" && entry.text}
                                    <div class="border-b px-2 py-1 font-mono last:border-b-0">
                                      {entry.text}
                                    </div>
                                  {/if}
                                {/each}
                              </div>
                            </div>
                          {:else if !detail.error}
                            <p class="text-muted-foreground">No progress recorded yet.</p>
                          {/if}
                        {/if}
                      </div>
                    </Table.Cell>
                  </Table.Row>
                {/if}
              {/each}
            </Table.Body>
          </Table.Root>
        {:else}
          <p class="text-sm text-muted-foreground">No build jobs for this project yet.</p>
        {/if}
      </div>
    </div>
  {:else}
    <ArtifactsStateBanner state={viewState} itemNoun="build targets" onRetry={refresh} />
  {/if}
</div>
