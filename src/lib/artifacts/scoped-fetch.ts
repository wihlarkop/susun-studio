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
 * Constructs the state for a newly-selected engine (or an initial load),
 * from a plain generation number the caller already advanced itself -
 * never derived from a `ScopedFetchState` value.
 *
 * This must be callable from inside a Svelte `$effect` without reading the
 * component's live `fetchState`: an effect that reads a piece of `$state`
 * and then assigns that same `$state` variable becomes a dependency of its
 * own write, which re-triggers itself indefinitely
 * (`effect_update_depth_exceeded`). Callers should keep the generation
 * counter as a plain, non-reactive `let` bumped synchronously inside the
 * effect, and pass the new value here directly.
 */
export function resetForNewEngine<T>(generation: number, loading: boolean): ScopedFetchState<T> {
  return { data: null, loading, error: null, generation };
}

/**
 * Marks a manual refresh/retry as in flight, keeping any existing data and
 * error in place (the view-state resolver already treats "has data and an
 * error" as `stale`, ahead of `refreshing`, so a retry doesn't flash the
 * old error away before the new attempt actually completes). Reads the
 * live state, so this is only safe to call from an event handler (a
 * button's `onclick`) - never from inside an `$effect`.
 */
export function withLoading<T>(state: ScopedFetchState<T>): ScopedFetchState<T> {
  return { ...state, loading: true };
}

/** Applies a successful completion, unless a newer engine switch has
 * already superseded the request that produced it. Always called after an
 * `await`, in the async continuation - never synchronously inside an
 * `$effect` - so reading `state` here does not create an effect
 * dependency. */
export function applyLoadSuccess<T>(
  state: ScopedFetchState<T>,
  requestGeneration: number,
  data: T,
): ScopedFetchState<T> {
  if (requestGeneration !== state.generation) return state;
  return { ...state, data, error: null, loading: false };
}

/** Applies a failed completion, unless a newer engine switch has already
 * superseded the request that produced it. Same after-await-only
 * requirement as `applyLoadSuccess`. */
export function applyLoadError<T>(
  state: ScopedFetchState<T>,
  requestGeneration: number,
  error: ArtifactRequestError,
): ScopedFetchState<T> {
  if (requestGeneration !== state.generation) return state;
  return { ...state, error, loading: false };
}
