<script lang="ts">
  import * as Card from "$lib/components/ui/card/index.js";
  import { Badge } from "$lib/components/ui/badge/index.js";
  import { Button } from "$lib/components/ui/button/index.js";
  import {
    ArrowDown,
    ArrowUp,
    Circle,
    CircleCheck,
    CircleMinus,
    CircleSlash,
    CircleX,
    Hammer,
    LoaderCircle,
    X,
  } from "@lucide/svelte";
  import {
    cancelJob,
    readJob,
    runAction,
    subscribeJobEvents,
    type StudioJob,
    type StudioProject,
  } from "$lib/daemon/client";

  type StepStatus = "pending" | "running" | "succeeded" | "failed" | "skipped" | "cancelled";
  type Step = {
    id: string;
    action: string;
    resource: string;
    status: StepStatus;
    progress: number;
  };

  let { project }: { project: StudioProject | null } = $props();

  let job = $state<StudioJob | null>(null);
  let steps = $state<Step[]>([]);
  let starting = $state(false);
  let cancelling = $state(false);
  let errorMessage = $state<string | null>(null);
  let source: EventSource | null = null;

  const projectId = $derived(project?.id ?? null);

  $effect(() => {
    void projectId;
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

  function statusVariant(status: string): "default" | "destructive" | "secondary" | "outline" {
    if (status === "succeeded") return "default";
    if (status === "failed") return "destructive";
    if (status === "cancelled") return "outline";
    return "secondary";
  }

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

  async function run(action: "up" | "down" | "build") {
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
        <Badge variant={statusVariant(job.status)}>{job.status}</Badge>
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
</Card.Root>
