import { describe, expect, it } from "vitest";
import {
  DaemonRequestError,
  type RuntimeActionResult,
  type TrustedRuntimePlan,
} from "$lib/daemon/client";
import {
  acknowledgeCancellation,
  applyExecutionFailure,
  applyPreparedPlan,
  beginCancellation,
  beginExecution,
  beginPreparation,
  canExecute,
  createRuntimeActionFlow,
  runtimeActionFailureMessage,
} from "./action-flow";

const identity = { providerId: "windows-podman", action: "setup" };
const otherIdentity = { providerId: "windows-podman", action: "start" };

function plan(): TrustedRuntimePlan {
  return {
    plan_id: "trusted-plan-1",
    provider_id: identity.providerId,
    action: identity.action,
    label: "Set up Susun Runtime",
    destructive: false,
    consequence: "Creates the managed runtime.",
    elevation: "current_user",
    command_summary: "Set up the managed runtime.",
    software_provenance: null,
    expires_in_seconds: 300,
    state: "pending",
  };
}

function result(status = "executed"): RuntimeActionResult {
  return { action: identity.action, status, message: "Complete", next_steps: [] };
}

describe("runtime action flow", () => {
  it("does not permit execution without a current plan id", () => {
    const flow = beginExecution(createRuntimeActionFlow());
    expect(flow.phase).toBe("idle");
    expect(canExecute(flow)).toBe(false);
  });

  it("invalidates a prepared plan when the provider or action changes", () => {
    const prepared = applyPreparedPlan(
      beginPreparation(createRuntimeActionFlow(), identity),
      identity,
      plan(),
    );
    const changed = beginPreparation(prepared, otherIdentity);

    expect(changed.phase).toBe("preparing");
    expect(changed.plan).toBeNull();
    expect(changed.identity).toEqual(otherIdentity);
    expect(canExecute(changed)).toBe(false);
    expect(applyPreparedPlan(changed, identity, plan())).toBe(changed);
  });

  it("does not allow a failed or consumed plan to execute again", () => {
    const prepared = applyPreparedPlan(
      beginPreparation(createRuntimeActionFlow(), identity),
      identity,
      plan(),
    );
    const failed = applyExecutionFailure(beginExecution(prepared), result("failed"));

    expect(failed.phase).toBe("failed");
    expect(failed.plan).toBeNull();
    expect(canExecute(failed)).toBe(false);
    expect(beginExecution(failed)).toBe(failed);
  });

  it("remains cancelling until the daemon acknowledges cancellation", () => {
    const prepared = applyPreparedPlan(
      beginPreparation(createRuntimeActionFlow(), identity),
      identity,
      plan(),
    );
    const cancelling = beginCancellation(prepared);

    expect(cancelling.phase).toBe("cancelling");
    expect(cancelling.plan?.plan_id).toBe("trusted-plan-1");
    const cancelled = acknowledgeCancellation(cancelling, result("cancelled"));
    expect(cancelled.phase).toBe("cancelled");
    expect(cancelled.plan).toBeNull();
  });

  it("uses bounded failure text instead of raw daemon command output", () => {
    const raw = "failed to run C:\\secret\\podman.exe --token super-secret";
    expect(runtimeActionFailureMessage(new DaemonRequestError(502, raw))).toBe(
      "The runtime could not be reached.",
    );
    expect(runtimeActionFailureMessage(new Error(raw))).toBe("Could not reach the daemon.");
    expect(runtimeActionFailureMessage(result("failed"))).toBe("The runtime action failed.");

    const prepared = applyPreparedPlan(
      beginPreparation(createRuntimeActionFlow(), identity),
      identity,
      plan(),
    );
    const failed = applyExecutionFailure(
      beginExecution(prepared),
      new DaemonRequestError(502, raw),
    );
    expect(failed.error).toBe("The runtime could not be reached.");
  });
});
