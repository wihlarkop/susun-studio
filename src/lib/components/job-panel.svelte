<script lang="ts">
  import * as Card from "$lib/components/ui/card/index.js";
  import * as Dialog from "$lib/components/ui/dialog/index.js";
  import { Button } from "$lib/components/ui/button/index.js";
  import StatusBadge from "./status-badge.svelte";
  import {
    ArrowDown,
    ArrowUp,
    ChevronRight,
    Circle,
    CircleCheck,
    CircleMinus,
    CircleSlash,
    CircleX,
    Hammer,
    LoaderCircle,
    Trash2,
    X,
  } from "@lucide/svelte";
  import {
    cancelJob,
    listProjectJobs,
    readJob,
    runAction,
    subscribeJobEvents,
    type StudioJob,
    type StudioProject,
  } from "$lib/daemon/client";
  import { relativeTime } from "$lib/utils";

  type StepStatus = "pending" | "running" | "succeeded" | "failed" | "skipped" | "cancelled";
  type Step = {
    id: string;
    action: string;
    resource: string;
    status: StepStatus;
    progress: number;
  };

  let {
    project,
    onJobFinished,
  }: { project: StudioProject | null; onJobFinished?: () => void } = $props();

  let job = $state<StudioJob | null>(null);
  let steps = $state<Step[]>([]);
  let starting = $state(false);
  let cancelling = $state(false);
  let errorMessage = $state<string | null>(null);
  let cleanConfirmOpen = $state(false);
  let pastJobs = $state<StudioJob[]>([]);
  let pastJobsOpen = $state(false);
  let source: EventSource | null = null;

  const projectId = $derived(project?.id ?? null);

  async function refreshPastJobs() {
    if (!projectId) return;
    try {
      pastJobs = await listProjectJobs(projectId);
    } catch {
      // best-effort — the live job UI above already reflects current state
    }
  }

  $effect(() => {
    void projectId;
    refreshPastJobs();
    return () => source?.close();
  });

  const TERMINAL: StepStatus[] = ["succeeded", "failed", "skipped", "cancelled"];
  const done = $derived(steps.filter((s) => TERMINAL.includes(s.status)).length);

  const STEP_ICON = {
    pending: { icon: Circle, class: "text-muted-foreground/50" },
    running: { icon: LoaderCircle, class: "animate-spin text-primary" },
    succeeded: { icon: CircleCheck, class: "text-primary" },
    failed: { icon: CircleX, class: "text-destructive" },
    cancelled: { icon: CircleSlash, class: "text-muted-foreground" },
    skipped: { icon: CircleMinus, class: "text-muted-foreground" },
  } as const;

  function setStep(id: string, patch: Partial<Step>) {
    steps = steps.map((step) => (step.id === id ? { ...step, ...patch } : step));
  }

  function mapFinish(status: string | undefined): StepStatus {
    if (status === "succeeded") return "succeeded";
    if (status === "failed") return "failed";
    if (status === "cancelled") return "cancelled";
    return "skipped";
  }

  function finalize(jobStatus: string) {
    const fallback: StepStatus =
      jobStatus === "succeeded" ? "succeeded" : jobStatus === "cancelled" ? "cancelled" : "skipped";
    steps = steps.map((step) =>
      TERMINAL.includes(step.status) ? step : { ...step, status: fallback },
    );
  }

  async function run(action: "up" | "down" | "build" | "clean") {
    if (!projectId) {
      return;
    }
    starting = true;
    cancelling = false;
    errorMessage = null;
    steps = [];
    try {
      const started = await runAction(projectId, action);
      job = started;
      steps = started.actions.map((a) => ({ ...a, status: "pending", progress: 0 }));
      attach(started.id);
    } catch (error) {
      errorMessage = error instanceof Error ? error.message : "Failed to start action";
    } finally {
      starting = false;
    }
  }

  function attach(jobId: string) {
    source?.close();
    subscribeJobEvents(jobId)
      .then((eventSource) => {
        source = eventSource;
        source.onmessage = (message) => {
          try {
            const event = JSON.parse(message.data) as {
              type: string;
              payload?: { action_id?: string; status?: string };
            };
            const id = event.payload?.action_id;
            if (id && event.type === "action_started") {
              setStep(id, { status: "running" });
            } else if (id && event.type === "action_finished") {
              setStep(id, { status: mapFinish(event.payload?.status) });
            } else if (id && event.type === "action_progress") {
              const current = steps.find((s) => s.id === id);
              setStep(id, { progress: (current?.progress ?? 0) + 1 });
            }
            if (event.type === "plan_finished") {
              readJob(jobId).then((updated) => {
                job = updated;
                cancelling = false;
                finalize(updated.status);
                onJobFinished?.();
                void refreshPastJobs();
              });
              source?.close();
            }
          } catch {
            // ignore malformed frames
          }
        };
        source.onerror = () => {
          readJob(jobId).then((updated) => {
            job = updated;
            cancelling = false;
            finalize(updated.status);
            onJobFinished?.();
            void refreshPastJobs();
          });
          source?.close();
        };
      })
      .catch((error) => {
        errorMessage = error instanceof Error ? error.message : "Failed to open event stream";
      });
  }

  async function cancel() {
    if (job) {
      cancelling = true;
      await cancelJob(job.id);
    }
  }
</script>

<Card.Root class="gap-3 p-4">
  <div class="flex items-center justify-between gap-2">
    <div class="flex items-center gap-2">
      <h3 class="text-lg font-semibold">Runtime Actions</h3>
      {#if job}
        <StatusBadge status={job.status} />
      {/if}
    </div>
    <div class="flex items-center gap-2">
      <Button size="sm" disabled={!project || starting} onclick={() => run("up")}>
        <ArrowUp />
        Up
      </Button>
      <Button
        size="sm"
        variant="outline"
        disabled={!project || starting}
        onclick={() => run("build")}
      >
        <Hammer />
        Build
      </Button>
      <Button
        size="sm"
        variant="outline"
        disabled={!project || starting}
        onclick={() => run("down")}
      >
        <ArrowDown />
        Down
      </Button>
      <Button
        size="sm"
        variant="destructive"
        disabled={!project || starting}
        onclick={() => (cleanConfirmOpen = true)}
      >
        <Trash2 />
        Clean
      </Button>
      {#if job?.status === "running"}
        <Button size="sm" variant="destructive" disabled={cancelling} onclick={cancel}>
          <X />
          {cancelling ? "Cancelling…" : "Cancel"}
        </Button>
      {/if}
    </div>
  </div>

  {#if !project}
    <p class="text-sm text-muted-foreground">Select a project to run actions.</p>
  {/if}

  {#if errorMessage}
    <p class="text-sm text-destructive">{errorMessage}</p>
  {/if}

  {#if steps.length > 0}
    <div class="space-y-2 rounded-md border p-3">
      <div class="flex items-center justify-between text-xs text-muted-foreground">
        <span>{done} / {steps.length} steps</span>
      </div>
      <div class="h-1.5 w-full overflow-hidden rounded-full bg-muted">
        <div
          class="h-full rounded-full bg-primary transition-[width] duration-300 ease-out"
          style="width: {steps.length ? (done / steps.length) * 100 : 0}%"
        ></div>
      </div>
      <ul class="space-y-1">
        {#each steps as step (step.id)}
          {@const info = STEP_ICON[step.status]}
          <li class="flex items-center gap-2 text-sm">
            <info.icon class="size-4 shrink-0 {info.class}" />
            <span class="text-muted-foreground">{step.action}</span>
            <span class="font-medium">{step.resource}</span>
            {#if step.status === "running" && step.progress > 0}
              <span class="text-xs text-muted-foreground">· pulling… ({step.progress})</span>
            {/if}
          </li>
        {/each}
      </ul>
    </div>
  {/if}

  {#if job?.error}
    <p class="text-sm text-destructive">{job.error}</p>
  {/if}

  {#if pastJobs.length > 0}
    <button
      type="button"
      class="text-muted-foreground hover:text-foreground flex w-fit items-center gap-1 text-xs font-medium"
      onclick={() => (pastJobsOpen = !pastJobsOpen)}
    >
      <ChevronRight class="size-3 transition-transform {pastJobsOpen ? 'rotate-90' : ''}" />
      Past jobs ({pastJobs.length})
    </button>
    {#if pastJobsOpen}
      <ul class="flex flex-col gap-1 text-sm">
        {#each pastJobs as pastJob (pastJob.id)}
          <li class="flex items-center gap-2 rounded-md border px-2 py-1">
            <span class="font-medium">{pastJob.kind}</span>
            <StatusBadge status={pastJob.status} />
            <span class="text-muted-foreground ml-auto text-xs">
              {relativeTime(pastJob.created_at_ms)}
            </span>
          </li>
          {#if pastJob.error}
            <li class="text-destructive px-2 text-xs">{pastJob.error}</li>
          {/if}
        {/each}
      </ul>
    {/if}
  {/if}
</Card.Root>

<Dialog.Root bind:open={cleanConfirmOpen}>
  <Dialog.Content class="sm:max-w-md">
    <Dialog.Header>
      <Dialog.Title>Clean this project?</Dialog.Title>
      <Dialog.Description>
        Stops and removes all containers, networks, and <b>named volumes</b> for this project.
        Volume data (e.g. database contents) will be permanently deleted. This cannot be undone.
      </Dialog.Description>
    </Dialog.Header>
    <Dialog.Footer>
      <Button type="button" variant="outline" onclick={() => (cleanConfirmOpen = false)}>
        Cancel
      </Button>
      <Button
        type="button"
        variant="destructive"
        onclick={() => {
          cleanConfirmOpen = false;
          run("clean");
        }}
      >
        Clean
      </Button>
    </Dialog.Footer>
  </Dialog.Content>
</Dialog.Root>
