import { DaemonRequestError } from "$lib/daemon/client";
import type { ArtifactRequestError } from "./workspace-state";

/**
 * Fixed, display-safe messages keyed by HTTP status. The daemon's own error
 * body text is never shown here: some `ApiError` variants (engine
 * connection failures in particular) wrap raw provider/SDK error text that
 * can legitimately contain a socket path, named pipe, or other endpoint
 * detail — safe to log server-side, not safe to render verbatim in the
 * browser. This is an intentionally small, enumerable, testable set rather
 * than a passthrough.
 */
const STATUS_MESSAGES: Record<number, string> = {
  401: "You're not authorized to talk to the daemon.",
  404: "This resource is no longer visible on this engine.",
  422: "This request could not be completed right now.",
  500: "The daemon hit an internal error.",
  502: "The engine could not be reached.",
};

function messageForStatus(status: number): string {
  return STATUS_MESSAGES[status] ?? `The daemon request failed (HTTP ${status}).`;
}

/**
 * Normalizes anything a daemon client call can throw into the shape
 * `resolveArtifactViewState` expects. Deliberately never forwards the
 * daemon's own error body text (see `STATUS_MESSAGES`) or a raw
 * network-layer error message — only the HTTP status (or its absence) is
 * used to pick a fixed, display-safe message. This is the artifact UI's
 * redacted public error contract; other parts of the app may still choose
 * to show `DaemonRequestError.message` directly where that's safe.
 */
export function toArtifactRequestError(error: unknown): ArtifactRequestError {
  if (error instanceof DaemonRequestError) {
    return { status: error.status, message: messageForStatus(error.status) };
  }
  return { status: null, message: "Could not reach the daemon." };
}
