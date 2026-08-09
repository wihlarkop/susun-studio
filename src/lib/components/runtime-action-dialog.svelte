<script lang="ts">
  import * as Dialog from "$lib/components/ui/dialog/index.js";
  import { Badge } from "$lib/components/ui/badge/index.js";
  import { Button } from "$lib/components/ui/button/index.js";
  import {
    cancelRuntimePlan,
    executeRuntimePlan,
    type PrepareRuntimeActionResponse,
    type RuntimeActionResult,
  } from "$lib/daemon/client";
  import {
    acknowledgeCancellation,
    acknowledgeExecution,
    applyCancellationFailure,
    applyExecutionFailure,
    applyPreparationFailure,
    applyPreparationResult,
    applyPreparedPlan,
    beginCancellation,
    beginExecution,
    beginPreparation,
    canExecute,
    createRuntimeActionFlow,
    type RuntimeActionIdentity,
  } from "$lib/runtime/action-flow";

  export type RuntimeActionDialogRequest = {
    identity: RuntimeActionIdentity;
    prepare: () => Promise<PrepareRuntimeActionResponse>;
  };

  let {
    request = null,
    open = $bindable(false),
    oncompleted,
    oncancelled,
  }: {
    request?: RuntimeActionDialogRequest | null;
    open?: boolean;
    oncompleted?: () => void | Promise<void>;
    oncancelled?: () => void | Promise<void>;
  } = $props();

  let flow = $state(createRuntimeActionFlow());
  let preparedRequestKey: string | null = null;

  $effect(() => {
    const current = request;
    const active = open;
    const key = current ? `${current.identity.providerId}:${current.identity.action}` : null;
    if (!active || !current || !key) {
      preparedRequestKey = null;
      return;
    }
    if (preparedRequestKey === key) return;
    preparedRequestKey = key;
    void prepare(current);
  });

  async function prepare(current: RuntimeActionDialogRequest) {
    flow = beginPreparation(flow, current.identity);
    try {
      const response = await current.prepare();
      if (response.plan) {
        flow = applyPreparedPlan(flow, current.identity, response.plan);
      } else if (response.result) {
        flow = applyPreparationResult(flow, response.result);
      } else {
        flow = applyPreparationFailure(flow, new Error("missing runtime plan"));
      }
    } catch (error) {
      flow = applyPreparationFailure(flow, error);
    }
  }

  async function execute() {
    const planId = flow.plan?.plan_id;
    flow = beginExecution(flow);
    if (!planId || flow.phase !== "executing") return;
    try {
      const result = await executeRuntimePlan(planId);
      flow = acknowledgeExecution(flow, result);
      if (flow.phase === "succeeded") {
        await oncompleted?.();
      }
    } catch (error) {
      flow = applyExecutionFailure(flow, error);
    }
  }

  async function cancel() {
    const planId = flow.plan?.plan_id;
    flow = beginCancellation(flow);
    if (!planId || flow.phase !== "cancelling") return;
    try {
      const result = await cancelRuntimePlan(planId);
      flow = acknowledgeCancellation(flow, result);
      if (flow.phase === "cancelled") {
        open = false;
        await oncancelled?.();
      }
    } catch (error) {
      flow = applyCancellationFailure(flow, error);
    }
  }

  function requestClose() {
    if (flow.phase === "previewed" && flow.plan) {
      void cancel();
      return;
    }
    if (flow.phase === "executing" || flow.phase === "cancelling") return;
    flow = createRuntimeActionFlow();
    open = false;
  }

  function setOpen(next: boolean) {
    if (next) {
      open = true;
      return;
    }
    requestClose();
  }

  function statusLabel(result: RuntimeActionResult): string {
    return result.status === "executed" ? "The runtime action completed." : "Runtime action update received.";
  }
</script>

<Dialog.Root bind:open={() => open, setOpen}>
  <Dialog.Content class="sm:max-w-lg">
    <Dialog.Header>
      <Dialog.Title>{flow.plan?.label ?? "Approve runtime action"}</Dialog.Title>
      <Dialog.Description>
        Review the exact consequence before allowing this single-use runtime plan.
      </Dialog.Description>
    </Dialog.Header>

    {#if flow.phase === "preparing"}
      <p class="text-sm text-muted-foreground">Preparing a trusted runtime plan...</p>
    {/if}

    {#if flow.error}
      <p class="text-sm text-destructive">{flow.error}</p>
    {/if}

    {#if flow.plan}
      <div class="grid gap-3 text-sm">
        <div class="grid gap-1">
          <span class="font-medium">Consequence</span>
          <span class="text-muted-foreground">{flow.plan.consequence}</span>
        </div>
        <div class="grid gap-1">
          <span class="font-medium">Verified operation</span>
          <span class="text-muted-foreground">{flow.plan.command_summary}</span>
        </div>
        {#if flow.plan.software_provenance}
          <dl class="grid grid-cols-[minmax(7rem,auto)_minmax(0,1fr)] gap-x-4 gap-y-1 border-y py-3 text-xs">
            <dt class="text-muted-foreground">Package</dt>
            <dd class="min-w-0 font-mono [overflow-wrap:anywhere]">
              {flow.plan.software_provenance.package_id}
            </dd>
            <dt class="text-muted-foreground">Source</dt>
            <dd class="min-w-0 [overflow-wrap:anywhere]">
              {flow.plan.software_provenance.source} · {flow.plan.software_provenance.source_url}
            </dd>
            <dt class="text-muted-foreground">Expected publisher</dt>
            <dd>{flow.plan.software_provenance.expected_publisher}</dd>
            <dt class="text-muted-foreground">Version</dt>
            <dd>{flow.plan.software_provenance.version_intent}</dd>
            <dt class="text-muted-foreground">Restart impact</dt>
            <dd>{flow.plan.software_provenance.restart_impact}</dd>
          </dl>
        {/if}
        <div class="flex flex-wrap gap-2">
          <Badge variant={flow.plan.destructive ? "destructive" : "secondary"}>
            {flow.plan.destructive ? "Destructive" : "Runtime mutation"}
          </Badge>
          <Badge variant="outline">
            {flow.plan.elevation === "os_mediated_consent"
              ? "Administrator consent expected"
              : "Current user"}
          </Badge>
          <Badge variant="outline">Expires in {flow.plan.expires_in_seconds}s</Badge>
        </div>
        <p class="text-xs text-muted-foreground">
          Executable paths, arguments, environment values, and credentials are intentionally hidden.
          They are fixed by Studio and cannot be changed from this dialog.
        </p>
      </div>
    {/if}

    {#if flow.result && flow.phase === "succeeded"}
      <p class="rounded-md border bg-muted/40 p-3 text-sm">{statusLabel(flow.result)}</p>
    {/if}

    <Dialog.Footer>
      <Button
        type="button"
        variant="outline"
        disabled={flow.phase === "executing" || flow.phase === "cancelling"}
        onclick={requestClose}
      >
        {flow.phase === "cancelling" ? "Cancelling..." : "Cancel"}
      </Button>
      <Button
        type="button"
        variant={flow.plan?.destructive ? "destructive" : "default"}
        disabled={!canExecute(flow)}
        onclick={execute}
      >
        {flow.phase === "executing" ? "Working..." : "Approve and run"}
      </Button>
    </Dialog.Footer>
  </Dialog.Content>
</Dialog.Root>
