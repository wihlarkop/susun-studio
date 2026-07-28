import type {
  BuildProgressEntry,
  ImageBuildResult,
  JobExecutionResult,
  JobStatus,
} from "$lib/daemon/client";

/**
 * Narrows `StudioJob.result`'s union. Kept as one pure predicate rather than
 * duplicating the `"image_reference" in result` check (or a `kind ===
 * "image_build"` check, which requires threading the job's `kind` alongside
 * its `result` at every call site) wherever a component needs to read
 * either shape.
 */
export function isImageBuildResult(
  result: JobExecutionResult | ImageBuildResult | null,
): result is ImageBuildResult {
  return result !== null && "image_reference" in result;
}

/** Whether an `image_build` job's status means it may still produce more
 * progress — used to decide whether a poll loop should keep running for
 * this specific job, not the whole job list. */
export function isBuildJobActive(status: JobStatus): boolean {
  return status === "queued" || status === "running";
}

/**
 * The last `limit` progress entries, in their original (already-ordered)
 * sequence — never re-sorts, since the server already returns them ordered
 * by `sequence`. Exists so a very long-running build's full history (bounded
 * server-side at 500 rows) doesn't have to become 500 DOM nodes: the caller
 * renders only this window, with a way to know how many were hidden.
 */
export function visibleBuildProgress(
  entries: BuildProgressEntry[],
  limit: number,
): { visible: BuildProgressEntry[]; hiddenCount: number } {
  if (entries.length <= limit) {
    return { visible: entries, hiddenCount: 0 };
  }
  return {
    visible: entries.slice(entries.length - limit),
    hiddenCount: entries.length - limit,
  };
}
