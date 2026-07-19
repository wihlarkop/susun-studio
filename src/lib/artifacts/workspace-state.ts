import { isCapabilityUsable } from "./capability";

export type ArtifactRequestError = {
  /** HTTP status from the daemon response, or `null` for a network-level
   * failure (fetch itself rejected — no response at all). */
  status: number | null;
  message: string;
};

/**
 * The one place that decides which of the required view states a fetched
 * artifact surface (container/image list or detail, build-cache status,
 * registry capability) is in right now. Kept as a pure function so every
 * combination of inputs can be exercised directly in tests, without
 * mounting a component or a live daemon.
 */
export type ArtifactViewState =
  | { kind: "disconnected" }
  | { kind: "loading" }
  | { kind: "refreshing" }
  | { kind: "unreachable"; error: ArtifactRequestError }
  | { kind: "request-error"; error: ArtifactRequestError }
  | { kind: "unsupported"; capability: string }
  | { kind: "empty" }
  | { kind: "stale"; error: ArtifactRequestError }
  | { kind: "ready" };

export type ArtifactViewStateInput = {
  /** Whether the daemon itself is reachable at all (from `HealthState`). */
  connected: boolean;
  /** A fetch for this surface is currently in flight. */
  loading: boolean;
  /** A successful response is currently held (possibly from a prior fetch,
   * even if the most recent one failed). */
  hasData: boolean;
  /** The most recent fetch's error, or `null` after a clean success. */
  error: ArtifactRequestError | null;
  /** The SDK support level for this surface, once known. `null` before the
   * first successful response, or for a surface with no single top-level
   * capability (e.g. registry, which exposes three independent flags). */
  capability: string | null;
  /** Number of items in a list surface, or `null` for a non-list surface
   * (a single object, like build-cache status or a detail view) — `null`
   * never triggers the "empty" state. */
  itemCount: number | null;
};

/** The HTTP status the daemon uses for "the provider engine could not be
 * reached" (`ApiError::EngineUnavailable`). Every other failure status is a
 * `request-error`. */
const UNREACHABLE_STATUS = 502;

export function resolveArtifactViewState(input: ArtifactViewStateInput): ArtifactViewState {
  if (!input.connected) {
    return { kind: "disconnected" };
  }

  if (input.hasData) {
    if (input.error) {
      return { kind: "stale", error: input.error };
    }
    if (input.loading) {
      return { kind: "refreshing" };
    }
    if (input.capability !== null && !isCapabilityUsable(input.capability)) {
      return { kind: "unsupported", capability: input.capability };
    }
    if (input.itemCount !== null && input.itemCount === 0) {
      return { kind: "empty" };
    }
    return { kind: "ready" };
  }

  if (input.error) {
    return input.error.status === UNREACHABLE_STATUS
      ? { kind: "unreachable", error: input.error }
      : { kind: "request-error", error: input.error };
  }

  return { kind: "loading" };
}
