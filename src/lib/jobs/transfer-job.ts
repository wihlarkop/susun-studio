import type {
  ArtifactTransferResult,
  ImageBuildResult,
  JobExecutionResult,
  RegistryCredential,
  StudioJob,
  TransferProgressEntry,
} from "$lib/daemon/client";

type PullAuthSelection =
  | { kind: "anonymous" }
  | { kind: "credential"; credential: RegistryCredential }
  | {
      kind: "blocked";
      reason: "unsupported_auth" | "credential_not_found" | "credential_not_ready";
    };

export function deriveRegistryIdentity(imageReference: string): string | null {
  const reference = imageReference.trim();
  if (!reference || reference.includes("://") || /\s/.test(reference)) return null;
  const first = reference.split("/", 1)[0];
  if (
    reference.includes("/") &&
    (first.includes(".") || first.includes(":") || first === "localhost")
  ) {
    return first.toLowerCase();
  }
  return "docker.io";
}

export function resolvePullAuthSelection({
  registry,
  credentialId,
  credentials,
  authSupported,
}: {
  registry: string | null;
  credentialId: string | null;
  credentials: RegistryCredential[];
  authSupported: boolean;
}): PullAuthSelection {
  if (!credentialId) return { kind: "anonymous" };
  if (!authSupported) return { kind: "blocked", reason: "unsupported_auth" };
  const credential = credentials.find(
    (candidate) => candidate.id === credentialId && candidate.registry === registry,
  );
  if (!credential) return { kind: "blocked", reason: "credential_not_found" };
  if (credential.status !== "ready") {
    return { kind: "blocked", reason: "credential_not_ready" };
  }
  return { kind: "credential", credential };
}

export function isTransferJobActive(job: StudioJob): boolean {
  return (
    (job.kind === "image_pull" || job.kind === "image_push") &&
    (job.status === "queued" || job.status === "running")
  );
}

export function isArtifactTransferResult(
  result: JobExecutionResult | ImageBuildResult | ArtifactTransferResult | null,
): result is ArtifactTransferResult {
  return result !== null && "registry" in result && "engine_id" in result;
}

export function isJobExecutionResult(
  result: JobExecutionResult | ImageBuildResult | ArtifactTransferResult | null,
): result is JobExecutionResult {
  return result !== null && "summary" in result;
}

export function transferJobOutcome(
  job: StudioJob,
): "active" | "uncertain" | "succeeded" | "failed" | "cancelled" {
  if (isTransferJobActive(job)) return "active";
  if (job.error_code?.endsWith("_result_uncertain")) return "uncertain";
  if (job.status === "queued" || job.status === "running") return "active";
  return job.status;
}

export function isCurrentTransferRequest(
  currentGeneration: number,
  requestGeneration: number,
  aborted: boolean,
): boolean {
  return !aborted && currentGeneration === requestGeneration;
}

export function visibleTransferProgress(
  entries: TransferProgressEntry[],
  limit: number,
): { visible: TransferProgressEntry[]; hiddenCount: number } {
  if (entries.length <= limit) return { visible: entries, hiddenCount: 0 };
  return {
    visible: entries.slice(entries.length - limit),
    hiddenCount: entries.length - limit,
  };
}
