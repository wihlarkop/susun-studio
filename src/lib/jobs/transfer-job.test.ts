import { describe, expect, test } from "vitest";

import type { RegistryCredential, StudioJob, TransferProgressEntry } from "$lib/daemon/client";
import {
  deriveRegistryIdentity,
  isCurrentTransferRequest,
  isArtifactTransferResult,
  isJobExecutionResult,
  isTransferJobActive,
  resolvePullAuthSelection,
  transferJobOutcome,
  visibleTransferProgress,
} from "$lib/jobs/transfer-job";

const credential: RegistryCredential = {
  id: "credential-1",
  registry: "ghcr.io",
  username_label: "edo",
  status: "ready",
  created_at_ms: 1,
  updated_at_ms: 1,
  last_success_at_ms: null,
};

function pullJob(overrides: Partial<StudioJob> = {}): StudioJob {
  return {
    id: "job-1",
    kind: "image_pull",
    status: "running",
    project_id: "",
    runtime_profile_id: null,
    runtime_class: null,
    runtime_binding_source: "platform_default",
    service_name: null,
    actions: [],
    result: null,
    error: null,
    error_code: null,
    transfer_progress: [],
    created_at_ms: 1,
    updated_at_ms: 1,
    ...overrides,
  };
}

describe("pull authentication selection", () => {
  test("derives Docker Hub for familiar unqualified image references", () => {
    expect(deriveRegistryIdentity("alpine:latest")).toBe("docker.io");
    expect(deriveRegistryIdentity("library/nginx")).toBe("docker.io");
    expect(deriveRegistryIdentity("ghcr.io/susun/app:v1")).toBe("ghcr.io");
  });

  test("keeps public pulls anonymous when no credential is selected", () => {
    expect(
      resolvePullAuthSelection({
        registry: "ghcr.io",
        credentialId: null,
        credentials: [credential],
        authSupported: true,
      }),
    ).toEqual({ kind: "anonymous" });
  });

  test("accepts one ready credential for the derived registry", () => {
    expect(
      resolvePullAuthSelection({
        registry: "ghcr.io",
        credentialId: credential.id,
        credentials: [credential],
        authSupported: true,
      }),
    ).toEqual({ kind: "credential", credential });
  });

  test("blocks authenticated pulls when the engine does not support auth", () => {
    expect(
      resolvePullAuthSelection({
        registry: "ghcr.io",
        credentialId: credential.id,
        credentials: [credential],
        authSupported: false,
      }),
    ).toEqual({ kind: "blocked", reason: "unsupported_auth" });
  });
});

describe("durable transfer job state", () => {
  test("recognizes the transfer result without weakening build result narrowing", () => {
    expect(
      isArtifactTransferResult({
        image_reference: "docker.io/library/alpine:latest",
        requested_image: "alpine:latest",
        registry: "docker.io",
        engine_id: "platform-default",
        runtime_profile_id: null,
        authenticated: false,
      }),
    ).toBe(true);
    expect(
      isArtifactTransferResult({
        image_reference: "susun/app:latest",
        image_digest: null,
      }),
    ).toBe(false);
    expect(
      isJobExecutionResult({
        summary: {
          total_actions: 1,
          succeeded: 1,
          failed: 0,
          skipped: 0,
          cancelled: 0,
        },
      }),
    ).toBe(true);
    expect(isJobExecutionResult(null)).toBe(false);
  });

  test("treats queued and running pull jobs as active", () => {
    expect(isTransferJobActive(pullJob({ status: "queued" }))).toBe(true);
    expect(isTransferJobActive(pullJob({ status: "running" }))).toBe(true);
    expect(isTransferJobActive(pullJob({ status: "succeeded" }))).toBe(false);
  });

  test("surfaces cancellation with uncertain provider outcome", () => {
    expect(
      transferJobOutcome(
        pullJob({
          status: "cancelled",
          error_code: "transfer_cancelled_result_uncertain",
        }),
      ),
    ).toBe("uncertain");
  });

  test("recognizes a later provider result that corrected an uncertain job", () => {
    expect(
      transferJobOutcome(
        pullJob({
          status: "succeeded",
          result: {
            image_reference: "docker.io/library/alpine:latest",
            requested_image: "alpine:latest",
            registry: "docker.io",
            engine_id: "platform-default",
            runtime_profile_id: null,
            authenticated: false,
          },
        }),
      ),
    ).toBe("succeeded");
  });

  test("ignores a completion from the previous engine generation", () => {
    expect(isCurrentTransferRequest(4, 3, false)).toBe(false);
    expect(isCurrentTransferRequest(4, 4, true)).toBe(false);
    expect(isCurrentTransferRequest(4, 4, false)).toBe(true);
  });

  test("renders only the newest progress window without reordering entries", () => {
    const entries: TransferProgressEntry[] = Array.from({ length: 5 }, (_, sequence) => ({
      sequence,
      operation: "pull",
      stage: "downloading",
      current_units: sequence,
      total_units: 5,
      message: null,
      created_at_ms: sequence,
    }));
    expect(visibleTransferProgress(entries, 3)).toEqual({
      visible: entries.slice(2),
      hiddenCount: 2,
    });
  });
});
