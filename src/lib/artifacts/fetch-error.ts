import { DaemonRequestError } from "$lib/daemon/client";
import type { ArtifactRequestError } from "./workspace-state";

/** Normalizes anything a daemon client call can throw into the shape
 * `resolveArtifactViewState` expects, preserving the HTTP status from a
 * `DaemonRequestError` when there is one. */
export function toArtifactRequestError(error: unknown): ArtifactRequestError {
  if (error instanceof DaemonRequestError) {
    return { status: error.status, message: error.message };
  }
  if (error instanceof Error) {
    return { status: null, message: error.message };
  }
  return { status: null, message: String(error) };
}
