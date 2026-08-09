import {
  DaemonRequestError,
  type RuntimeActionResult,
  type TrustedRuntimePlan,
} from "$lib/daemon/client";

export type RuntimeActionIdentity = {
  providerId: string;
  action: string;
};

export type RuntimeActionPhase =
  | "idle"
  | "preparing"
  | "previewed"
  | "executing"
  | "succeeded"
  | "failed"
  | "cancelling"
  | "cancelled";

export type RuntimeActionFlow = {
  phase: RuntimeActionPhase;
  identity: RuntimeActionIdentity | null;
  plan: TrustedRuntimePlan | null;
  result: RuntimeActionResult | null;
  error: string | null;
};

export function createRuntimeActionFlow(): RuntimeActionFlow {
  return { phase: "idle", identity: null, plan: null, result: null, error: null };
}

export function beginPreparation(
  _flow: RuntimeActionFlow,
  identity: RuntimeActionIdentity,
): RuntimeActionFlow {
  return { phase: "preparing", identity, plan: null, result: null, error: null };
}

export function applyPreparedPlan(
  flow: RuntimeActionFlow,
  identity: RuntimeActionIdentity,
  plan: TrustedRuntimePlan,
): RuntimeActionFlow {
  if (flow.phase !== "preparing" || !sameIdentity(flow.identity, identity)) return flow;
  if (
    plan.provider_id !== identity.providerId ||
    plan.action !== identity.action ||
    !plan.plan_id
  ) {
    return {
      ...flow,
      phase: "failed",
      plan: null,
      error: "This runtime action changed. Prepare it again.",
    };
  }
  return { ...flow, phase: "previewed", plan, error: null };
}

export function applyPreparationFailure(
  flow: RuntimeActionFlow,
  error: unknown,
): RuntimeActionFlow {
  if (flow.phase !== "preparing") return flow;
  return { ...flow, phase: "failed", plan: null, error: runtimeActionFailureMessage(error) };
}

export function applyPreparationResult(
  flow: RuntimeActionFlow,
  result: RuntimeActionResult,
): RuntimeActionFlow {
  if (flow.phase !== "preparing") return flow;
  return {
    ...flow,
    phase: "failed",
    plan: null,
    result: null,
    error: runtimeActionFailureMessage(result),
  };
}

export function canExecute(flow: RuntimeActionFlow): boolean {
  return flow.phase === "previewed" && Boolean(flow.plan?.plan_id);
}

export function beginExecution(flow: RuntimeActionFlow): RuntimeActionFlow {
  if (!canExecute(flow)) return flow;
  return { ...flow, phase: "executing", error: null };
}

export function acknowledgeExecution(
  flow: RuntimeActionFlow,
  result: RuntimeActionResult,
): RuntimeActionFlow {
  if (flow.phase !== "executing") return flow;
  if (result.status !== "executed") {
    return {
      ...flow,
      phase: "failed",
      plan: null,
      result: null,
      error: runtimeActionFailureMessage(result),
    };
  }
  return { ...flow, phase: "succeeded", plan: null, result, error: null };
}

export function applyExecutionFailure(flow: RuntimeActionFlow, error: unknown): RuntimeActionFlow {
  if (flow.phase !== "executing") return flow;
  return {
    ...flow,
    phase: "failed",
    plan: null,
    result: null,
    error: runtimeActionFailureMessage(error),
  };
}

export function beginCancellation(flow: RuntimeActionFlow): RuntimeActionFlow {
  if (flow.phase !== "previewed" || !flow.plan?.plan_id) return flow;
  return { ...flow, phase: "cancelling", error: null };
}

export function acknowledgeCancellation(
  flow: RuntimeActionFlow,
  result: RuntimeActionResult,
): RuntimeActionFlow {
  if (flow.phase !== "cancelling") return flow;
  if (result.status !== "cancelled") {
    return {
      ...flow,
      phase: "failed",
      plan: null,
      result: null,
      error: runtimeActionFailureMessage(result),
    };
  }
  return { ...flow, phase: "cancelled", plan: null, result, error: null };
}

export function applyCancellationFailure(
  flow: RuntimeActionFlow,
  error: unknown,
): RuntimeActionFlow {
  if (flow.phase !== "cancelling") return flow;
  return {
    ...flow,
    phase: "failed",
    plan: null,
    result: null,
    error: runtimeActionFailureMessage(error),
  };
}

export function runtimeActionFailureMessage(error: unknown): string {
  if (error instanceof DaemonRequestError) {
    if (error.status === 401) return "You're not authorized to talk to the daemon.";
    if (error.status === 404) return "This runtime action is no longer available.";
    if (error.status === 422) return "This runtime action could not be completed right now.";
    if (error.status === 500) return "The daemon hit an internal error.";
    if (error.status === 502) return "The runtime could not be reached.";
    return `The daemon request failed (HTTP ${error.status}).`;
  }
  if (isRuntimeActionResult(error)) return "The runtime action failed.";
  return "Could not reach the daemon.";
}

function sameIdentity(left: RuntimeActionIdentity | null, right: RuntimeActionIdentity): boolean {
  return left?.providerId === right.providerId && left.action === right.action;
}

function isRuntimeActionResult(value: unknown): value is RuntimeActionResult {
  return typeof value === "object" && value !== null && "status" in value && "action" in value;
}
