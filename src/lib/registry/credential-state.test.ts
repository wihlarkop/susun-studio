import { describe, expect, it } from "vitest";
import { DaemonRequestError } from "$lib/daemon/client";
import {
  applyCredentialFailure,
  applyCredentialSuccess,
  beginCredentialSubmission,
  initialCredentialOperation,
  resetCredentialOperation,
  toCredentialOperationError,
} from "./credential-state";

describe("credential operation state", () => {
  it("starts idle without retaining credential material", () => {
    const state = initialCredentialOperation();
    expect(state).toEqual({
      phase: "idle",
      error: null,
      generation: 0,
    });
    expect("secret" in state).toBe(false);
  });

  it("starts submission and instructs the form to clear sensitive inputs", () => {
    const result = beginCredentialSubmission(initialCredentialOperation());
    expect(result.state.phase).toBe("submitting");
    expect(result.clearSensitiveInputs).toBe(true);
    expect("secret" in result.state).toBe(false);
  });

  it("applies success for the current generation", () => {
    const submitting = beginCredentialSubmission(initialCredentialOperation()).state;
    const result = applyCredentialSuccess(submitting, submitting.generation);
    expect(result).toEqual({
      phase: "succeeded",
      error: null,
      generation: 0,
    });
  });

  it("applies a bounded failure for the current generation", () => {
    const submitting = beginCredentialSubmission(initialCredentialOperation()).state;
    const result = applyCredentialFailure(submitting, submitting.generation, {
      status: 503,
      message: "Credential storage is unavailable on this device.",
    });
    expect(result.phase).toBe("failed");
    expect(result.error?.status).toBe(503);
  });

  it("resets when the active engine changes", () => {
    const submitting = beginCredentialSubmission(initialCredentialOperation()).state;
    const result = resetCredentialOperation(submitting.generation + 1);
    expect(result).toEqual({
      phase: "idle",
      error: null,
      generation: 1,
    });
  });

  it("ignores stale completions after a reset", () => {
    const submitting = beginCredentialSubmission(initialCredentialOperation()).state;
    const reset = resetCredentialOperation(submitting.generation + 1);
    expect(applyCredentialSuccess(reset, submitting.generation)).toBe(reset);
    expect(
      applyCredentialFailure(reset, submitting.generation, {
        status: 500,
        message: "Internal error.",
      }),
    ).toBe(reset);
  });
});

describe("credential operation errors", () => {
  it("maps daemon failures to fixed messages without forwarding secret-like text", () => {
    const raw = "keyring failed for token sentinel-registry-secret at C:\\Users\\Edo";
    const result = toCredentialOperationError(new DaemonRequestError(503, raw));
    expect(result).toEqual({
      status: 503,
      message: "Credential storage is unavailable on this device.",
    });
    expect(result.message).not.toContain("sentinel-registry-secret");
    expect(result.message).not.toContain("C:\\Users");
  });

  it("uses a fixed network failure message", () => {
    expect(toCredentialOperationError(new Error("host internal.example failed"))).toEqual({
      status: null,
      message: "Could not reach the daemon.",
    });
  });
});
