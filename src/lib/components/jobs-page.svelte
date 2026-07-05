<script lang="ts">
  import * as Card from "$lib/components/ui/card/index.js";
  import * as Table from "$lib/components/ui/table/index.js";
  import StatusBadge from "./status-badge.svelte";
  import {
    listJobs,
    type JobActionResult,
    type StudioJob,
    type StudioProject,
  } from "$lib/daemon/client";
  import { relativeTime } from "$lib/utils";

  let { projects }: { projects: StudioProject[] } = $props();

  let jobs = $state<StudioJob[]>([]);
  let statusFilter = $state("");
  let kindFilter = $state("");
  let projectFilter = $state("");
  let expandedId = $state<string | null>(null);
  let reportView = $state<"pretty" | "json">("pretty");
  let errorMessage = $state<string | null>(null);

  function toEpochMs(time: { secs_since_epoch: number; nanos_since_epoch: number } | null) {
    return time ? time.secs_since_epoch * 1000 + time.nanos_since_epoch / 1e6 : null;
  }

  function actionDuration(action: JobActionResult): string {
    const start = toEpochMs(action.started_at);
    const end = toEpochMs(action.finished_at);
    if (start === null || end === null) return "—";
    const ms = Math.max(0, end - start);
    return ms < 1000 ? `${Math.round(ms)} ms` : `${(ms / 1000).toFixed(1)} s`;
  }

  function actionLabel(job: StudioJob, actionId: string): string {
    const step = job.actions.find((candidate) => candidate.id === actionId);
    return step ? `${step.action} ${step.resource}` : actionId;
  }

  async function refresh() {
    try {
      jobs = await listJobs();
      errorMessage = null;
    } catch (error) {
      errorMessage = error instanceof Error ? error.message : String(error);
    }
  }

  $effect(() => {
    refresh();
    const timer = setInterval(refresh, 5000);
    return () => clearInterval(timer);
  });

  function projectName(projectId: string): string {
    return projects.find((project) => project.id === projectId)?.name ?? projectId;
  }

  const visible = $derived(
    jobs.filter(
      (job) =>
        (statusFilter === "" || job.status === statusFilter) &&
        (kindFilter === "" || job.kind === kindFilter) &&
        (projectFilter === "" || job.project_id === projectFilter),
    ),
  );
</script>

<div class="flex flex-col gap-4">
  <div>
    <h3 class="text-lg font-semibold">Jobs</h3>
    <p class="text-muted-foreground text-sm">Every job across every project, newest first.</p>
  </div>

  {#if errorMessage}
    <p class="text-destructive text-sm">{errorMessage}</p>
  {/if}

  <div class="flex flex-wrap items-center gap-2 text-sm">
    <select
      bind:value={statusFilter}
      class="border-input rounded-md border bg-transparent bg-none px-2 py-1"
    >
      <option value="">All statuses</option>
      <option value="running">running</option>
      <option value="succeeded">succeeded</option>
      <option value="failed">failed</option>
      <option value="cancelled">cancelled</option>
    </select>
    <select
      bind:value={kindFilter}
      class="border-input rounded-md border bg-transparent bg-none px-2 py-1"
    >
      <option value="">All kinds</option>
      <option value="up">up</option>
      <option value="down">down</option>
      <option value="build">build</option>
      <option value="clean">clean</option>
    </select>
    <select
      bind:value={projectFilter}
      class="border-input rounded-md border bg-transparent bg-none px-2 py-1"
    >
      <option value="">All projects</option>
      {#each projects as project (project.id)}
        <option value={project.id}>{project.name}</option>
      {/each}
    </select>
    <span class="text-muted-foreground ml-auto text-xs">{visible.length} / {jobs.length}</span>
  </div>

  <Card.Root class="p-0">
    <Table.Root>
      <Table.Header>
        <Table.Row>
          <Table.Head>Project</Table.Head>
          <Table.Head>Kind</Table.Head>
          <Table.Head>Status</Table.Head>
          <Table.Head>When</Table.Head>
        </Table.Row>
      </Table.Header>
      <Table.Body>
        {#if visible.length === 0}
          <Table.Row>
            <Table.Cell colspan={4} class="text-muted-foreground h-24 text-center">
              No jobs match these filters.
            </Table.Cell>
          </Table.Row>
        {:else}
          {#each visible as job (job.id)}
            <Table.Row
              class="cursor-pointer"
              onclick={() => (expandedId = expandedId === job.id ? null : job.id)}
            >
              <Table.Cell>{projectName(job.project_id)}</Table.Cell>
              <Table.Cell class="font-medium">{job.kind}</Table.Cell>
              <Table.Cell><StatusBadge status={job.status} /></Table.Cell>
              <Table.Cell class="text-muted-foreground text-xs">
                {relativeTime(job.created_at_ms)}
              </Table.Cell>
            </Table.Row>
            {#if expandedId === job.id}
              <Table.Row>
                <Table.Cell colspan={4} class="bg-muted/40 whitespace-normal">
                  <div class="flex flex-col gap-3 py-1">
                    {#if job.error}
                      <p class="text-destructive text-xs">
                        {job.error}
                        {#if job.error_code}<span class="font-mono">[{job.error_code}]</span>{/if}
                      </p>
                    {/if}

                    {#if !job.error && !job.result}
                      <p class="text-muted-foreground text-xs">No report yet.</p>
                    {/if}

                    {#if job.result}
                      <div class="border-input inline-flex w-fit rounded-md border p-0.5 text-xs">
                        <button
                          type="button"
                          class={[
                            "rounded px-2 py-1 font-medium",
                            reportView === "pretty"
                              ? "bg-primary text-primary-foreground"
                              : "text-muted-foreground",
                          ]}
                          onclick={() => (reportView = "pretty")}
                        >
                          Pretty
                        </button>
                        <button
                          type="button"
                          class={[
                            "rounded px-2 py-1 font-medium",
                            reportView === "json"
                              ? "bg-primary text-primary-foreground"
                              : "text-muted-foreground",
                          ]}
                          onclick={() => (reportView = "json")}
                        >
                          JSON
                        </button>
                      </div>

                      {#if reportView === "pretty"}
                        <div class="grid grid-cols-5 gap-2 text-xs">
                          <div class="rounded-md border px-2 py-1">
                            <div class="text-muted-foreground text-[0.65rem] tracking-wide uppercase">
                              Total
                            </div>
                            <div class="font-semibold tabular-nums">
                              {job.result.summary.total_actions}
                            </div>
                          </div>
                          <div class="rounded-md border px-2 py-1">
                            <div class="text-muted-foreground text-[0.65rem] tracking-wide uppercase">
                              Succeeded
                            </div>
                            <div class="text-success font-semibold tabular-nums">
                              {job.result.summary.succeeded}
                            </div>
                          </div>
                          <div class="rounded-md border px-2 py-1">
                            <div class="text-muted-foreground text-[0.65rem] tracking-wide uppercase">
                              Failed
                            </div>
                            <div class="text-destructive font-semibold tabular-nums">
                              {job.result.summary.failed}
                            </div>
                          </div>
                          <div class="rounded-md border px-2 py-1">
                            <div class="text-muted-foreground text-[0.65rem] tracking-wide uppercase">
                              Skipped
                            </div>
                            <div class="font-semibold tabular-nums">
                              {job.result.summary.skipped}
                            </div>
                          </div>
                          <div class="rounded-md border px-2 py-1">
                            <div class="text-muted-foreground text-[0.65rem] tracking-wide uppercase">
                              Cancelled
                            </div>
                            <div class="font-semibold tabular-nums">
                              {job.result.summary.cancelled}
                            </div>
                          </div>
                        </div>

                        {#if job.result.actions}
                          <ul class="flex flex-col gap-1">
                            {#each Object.values(job.result.actions) as action (action.action_id)}
                              <li class="flex items-center gap-2 rounded-md border px-2 py-1 text-xs">
                                <StatusBadge status={action.status} />
                                <span class="min-w-0 flex-1 break-words">
                                  {actionLabel(job, action.action_id)}
                                </span>
                                <span class="text-muted-foreground shrink-0 tabular-nums">
                                  {actionDuration(action)}
                                </span>
                              </li>
                              {#if action.error}
                                <li class="text-destructive px-2 text-xs break-words">
                                  {action.error}
                                </li>
                              {/if}
                            {/each}
                          </ul>
                        {:else}
                          <p class="text-muted-foreground text-xs">
                            No per-action detail available for this report.
                          </p>
                        {/if}
                      {:else}
                        <pre class="overflow-x-auto text-xs">{JSON.stringify(job.result, null, 2)}</pre>
                      {/if}
                    {/if}
                  </div>
                </Table.Cell>
              </Table.Row>
            {/if}
          {/each}
        {/if}
      </Table.Body>
    </Table.Root>
  </Card.Root>
</div>
