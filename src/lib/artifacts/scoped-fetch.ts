import type { ArtifactRequestError } from "./workspace-state";

/**
 * Fetch state for one artifact surface (a container/image list, or a
 * single-object surface like build-cache status or registry capability),
 * scoped to whichever engine it was last loaded for. `generation` is bumped
 * every time the engine changes; a completion is only applied if its
 * captured generation still matches, so a response that arrives after the
 * user has already switched engines can never overwrite the new engine's
 * state — this is the second, belt-and-suspenders guard alongside aborting
 * the underlying request.
 */
export type ScopedFetchState<T> = {
  data: T | null;
  loading: boolean;
  error: ArtifactRequestError | null;
  generation: number;
};

export function initialScopedFetchState<T>(): ScopedFetchState<T> {
  return { data: null, loading: false, error: null, generation: 0 };
}

/**
 * Produces the state for a newly-selected engine: every piece of the
 * previous engine's data, loading flag, and error is cleared immediately —
 * never shown, even briefly, underneath the new engine's header — and the
 * generation is bumped so any still-in-flight request from the old engine
 * is recognized as stale once it completes.
 */
export function resetForNewEngine<T>(previous: ScopedFetchState<T>): ScopedFetchState<T> {
  return { data: null, loading: false, error: null, generation: previous.generation + 1 };
}

export function withLoading<T>(state: ScopedFetchState<T>): ScopedFetchState<T> {
  return { ...state, loading: true };
}

/** Applies a successful completion, unless a newer engine switch has
 * already superseded the request that produced it. */
export function applyLoadSuccess<T>(
  state: ScopedFetchState<T>,
  requestGeneration: number,
  data: T,
): ScopedFetchState<T> {
  if (requestGeneration !== state.generation) return state;
  return { ...state, data, error: null, loading: false };
}

/** Applies a failed completion, unless a newer engine switch has already
 * superseded the request that produced it. */
export function applyLoadError<T>(
  state: ScopedFetchState<T>,
  requestGeneration: number,
  error: ArtifactRequestError,
): ScopedFetchState<T> {
  if (requestGeneration !== state.generation) return state;
  return { ...state, error, loading: false };
}

/** For the "daemon not connected" branch: only clears the loading flag if
 * this request is still the current one for its engine. */
export function applyNotConnected<T>(
  state: ScopedFetchState<T>,
  requestGeneration: number,
): ScopedFetchState<T> {
  if (requestGeneration !== state.generation) return state;
  return { ...state, loading: false };
}
