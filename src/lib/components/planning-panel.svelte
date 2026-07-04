<script lang="ts">
  import * as Card from "$lib/components/ui/card/index.js";
  import * as Table from "$lib/components/ui/table/index.js";
  import { Badge } from "$lib/components/ui/badge/index.js";
  import { Button } from "$lib/components/ui/button/index.js";
  import { Separator } from "$lib/components/ui/separator/index.js";
  import { ArrowDown, ArrowUp, Download } from "@lucide/svelte";
  import { cn, formatTimestamp, relativeTime } from "$lib/utils";
  import {
    createDownPlan,
    createUpPlan,
    listProjectPlans,
    readPlan,
    type PlanActionSafety,
    type StudioPlan,
    type StudioProject,
  } from "$lib/daemon/client";

  let { project }: { project: StudioProject | null } = $props();

  const projectId = $derived(project?.id ?? null);

  let plans = $state<StudioPlan[]>([]);
  let activePlan = $state<StudioPlan | null>(null);
  let planningError = $state<string | null>(null);
  let planning = $state<"up" | "down" | null>(null);

  $effect(() => {
    const id = projectId;
    if (!id) {
      plans = [];
      activePlan = null;
      return;
    }

    const controller = new AbortController();
    listProjectPlans(id, { signal: controller.signal })
      .then((result) => {
        plans = result;
      })
      .catch(() => {
        plans = [];
      });

    return () => controller.abort();
  });

  async function generate(operation: "up" | "down") {
    if (!projectId) {
      return;
    }
    planning = operation;
    planningError = null;
    try {
      const plan =
        operation === "up" ? await createUpPlan(projectId) : await createDownPlan(projectId);
      plans = [plan, ...plans];
      activePlan = plan;
    } catch (error) {
      planningError = error instanceof Error ? error.message : "Planning failed";
    } finally {
      planning = null;
    }
  }

  async function selectPlan(plan: StudioPlan) {
    if (activePlan?.id === plan.id) {
      return;
    }
    // List rows carry summary counts but no action detail; fetch the full plan.
    activePlan = plan;
    try {
      activePlan = await readPlan(plan.id);
    } catch (error) {
      planningError = error instanceof Error ? error.message : "Failed to load plan";
    }
  }

  function safetyVariant(safety: PlanActionSafety): "secondary" | "outline" | "destructive" {
    if (safety === "destructive") return "destructive";
    if (safety === "caution") return "outline";
    return "secondary";
  }

  function exportJson() {
    if (!activePlan) {
      return;
    }
    const blob = new Blob([JSON.stringify(activePlan, null, 2)], { type: "application/json" });
    const url = URL.createObjectURL(blob);
    const link = document.createElement("a");
    link.href = url;
    link.download = `${activePlan.id}.json`;
    link.click();
    URL.revokeObjectURL(url);
  }
</script>

<Card.Root class="gap-0 p-0">
  <Card.Header class="flex flex-row items-center justify-between gap-2 p-4">
    <div>
      <Card.Title class="text-lg">Planning</Card.Title>
      <Card.Description>Preview what Susun would do, without touching an engine.</Card.Description>
    </div>
    <div class="flex items-center gap-2">
      <Button size="sm" disabled={!project || planning !== null} onclick={() => generate("up")}>
        <ArrowUp />
        {planning === "up" ? "Planning…" : "Plan Up"}
      </Button>
      <Button
        size="sm"
        variant="outline"
        disabled={!project || planning !== null}
        onclick={() => generate("down")}
      >
        <ArrowDown />
        {planning === "down" ? "Planning…" : "Plan Down"}
      </Button>
    </div>
  </Card.Header>

  <Separator />

  <Card.Content class="flex flex-col gap-4 p-4">
    {#if !project}
      <p class="text-sm text-muted-foreground">Select a project to generate a plan.</p>
    {:else}
      {#if planningError}
        <p class="text-sm text-destructive">{planningError}</p>
      {/if}

      {#if plans.length > 0}
        <div class="flex flex-wrap gap-2">
          {#each plans as plan (plan.id)}
            <button
              type="button"
              class={cn(
                "flex flex-col items-start gap-0.5 rounded-md border px-3 py-2 text-left text-sm transition-colors hover:bg-muted focus-visible:ring-2 focus-visible:ring-ring focus-visible:outline-none",
                activePlan?.id === plan.id && "bg-muted",
              )}
              onclick={() => selectPlan(plan)}
            >
              <span class="flex items-center gap-1.5 font-medium">
                {#if plan.operation === "up"}<ArrowUp class="size-3.5" />{:else}<ArrowDown
                    class="size-3.5"
                  />{/if}
                {plan.operation}
              </span>
              <span class="text-xs text-muted-foreground" title={formatTimestamp(plan.created_at_ms)}>
                {plan.summary.total_actions} action{plan.summary.total_actions === 1 ? "" : "s"}
                · {relativeTime(plan.created_at_ms)}
              </span>
            </button>
          {/each}
        </div>
      {:else}
        <p class="text-sm text-muted-foreground">
          No plans yet. Generate an up or down plan to preview the action graph.
        </p>
      {/if}

      {#if activePlan}
        <Separator />

        <div class="flex items-center justify-between gap-2">
          <div class="flex flex-wrap gap-4 text-sm">
            <span><span class="text-muted-foreground">Total</span>
              <span class="ml-1 font-medium tabular-nums">{activePlan.summary.total_actions}</span
              ></span>
            <span><span class="text-muted-foreground">Safe</span>
              <span class="ml-1 font-medium tabular-nums">{activePlan.summary.safe_actions}</span
              ></span>
            <span><span class="text-muted-foreground">Caution</span>
              <span class="ml-1 font-medium tabular-nums">{activePlan.summary.caution_actions}</span
              ></span>
            <span><span class="text-muted-foreground">Destructive</span>
              <span class="ml-1 font-medium tabular-nums"
                >{activePlan.summary.destructive_actions}</span
              ></span>
          </div>
          <Button size="sm" variant="outline" onclick={exportJson}>
            <Download />
            Export JSON
          </Button>
        </div>

        {#if activePlan.blocked_diagnostics && activePlan.blocked_diagnostics.diagnostics.length > 0}
          <div class="space-y-2">
            <p class="text-sm font-medium">Planning was blocked</p>
            {#each activePlan.blocked_diagnostics.diagnostics as diagnostic}
              <div class="rounded-md border p-3 text-sm">
                <div class="flex items-center gap-2">
                  <Badge variant={diagnostic.severity === "error" ? "destructive" : "outline"}>
                    {diagnostic.severity}
                  </Badge>
                  <span class="font-mono text-xs text-muted-foreground">{diagnostic.code}</span>
                </div>
                <p class="mt-1">{diagnostic.message}</p>
                {#if diagnostic.help}
                  <p class="text-muted-foreground">{diagnostic.help}</p>
                {/if}
              </div>
            {/each}
          </div>
        {:else if activePlan.actions.length > 0}
          <div class="overflow-x-auto">
            <Table.Root>
              <Table.Header>
                <Table.Row>
                  <Table.Head>Resource</Table.Head>
                  <Table.Head>Action</Table.Head>
                  <Table.Head>Safety</Table.Head>
                  <Table.Head>Reason</Table.Head>
                  <Table.Head>Depends on</Table.Head>
                </Table.Row>
              </Table.Header>
              <Table.Body>
                {#each activePlan.actions as action (action.id)}
                  <Table.Row>
                    <Table.Cell class="font-medium">{action.resource}</Table.Cell>
                    <Table.Cell class="font-mono text-xs">{action.kind}</Table.Cell>
                    <Table.Cell>
                      <Badge variant={safetyVariant(action.safety)}>{action.safety}</Badge>
                    </Table.Cell>
                    <Table.Cell class="text-muted-foreground">{action.reason}</Table.Cell>
                    <Table.Cell class="text-muted-foreground">
                      {action.dependencies.join(", ") || "—"}
                    </Table.Cell>
                  </Table.Row>
                {/each}
              </Table.Body>
            </Table.Root>
          </div>
        {:else}
          <p class="text-sm text-muted-foreground">
            This plan has no actions (nothing to do for this operation).
          </p>
        {/if}
      {/if}
    {/if}
  </Card.Content>
</Card.Root>
