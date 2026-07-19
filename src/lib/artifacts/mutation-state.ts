import type { ArtifactRequestError } from "./workspace-state";

/**
 * Generic preview/confirm state machine shared by every artifact mutation
 * dialog (image tag, image remove, single-scope prune). Kept as one pure,
 * engine-agnostic module — parameterized by the preview and result payload
 * types — so the confirmation flow's state transitions can be tested once,
 * directly, without mounting a component or a live daemon.
 */
export type MutationPhase = "idle" | "previewing" | "previewed" | "committing" | "succeeded";

export type MutationState<TPreview, TResult> = {
  phase: MutationPhase;
  preview: TPreview | null;
  result: TResult | null;
  error: ArtifactRequestError | null;
  /**
   * Bumped every time the dialog is (re)armed for a specific target (opened,
   * or switched to a different row). A preview/commit response is only
   * applied if its captured generation still matches — the same
   * belt-and-suspenders guard `scoped-fetch.ts` uses for list surfaces,
   * applied here to a single dialog's in-flight request instead of a whole
   * engine switch. This is what makes a response for a target the user has
   * since navigated away from (or a dialog they closed and reopened)
   * impossible to apply as a stale completion.
   */
  generation: number;
};

export function initialMutationState<TPreview, TResult>(): MutationState<TPreview, TResult> {
  return { phase: "idle", preview: null, result: null, error: null, generation: 0 };
}

/**
 * (Re)arms the dialog for a new target, from a plain generation number the
 * caller already advanced itself — never derived from a live `MutationState`
 * value, for the same reason `resetForNewEngine` takes a plain number: this
 * must be safe to call from an effect without becoming a dependency of its
 * own write.
 */
export function resetMutation<TPreview, TResult>(
  generation: number,
): MutationState<TPreview, TResult> {
  return { phase: "idle", preview: null, result: null, error: null, generation };
}

/** Marks a preview request as in flight. Safe to call from an event handler
 * (a button's `onclick`), which is the only place this is ever called. */
export function startPreviewing<TPreview, TResult>(
  state: MutationState<TPreview, TResult>,
): MutationState<TPreview, TResult> {
  return { ...state, phase: "previewing", preview: null, result: null, error: null };
}

/** Applies a successful preview, unless the dialog has since moved on to a
 * different target (or been closed and reopened). */
export function applyPreviewSuccess<TPreview, TResult>(
  state: MutationState<TPreview, TResult>,
  requestGeneration: number,
  preview: TPreview,
): MutationState<TPreview, TResult> {
  if (requestGeneration !== state.generation) return state;
  return { ...state, phase: "previewed", preview, error: null };
}

/** Applies a failed preview, unless superseded. Falls back to `idle` so the
 * user can retry from a clean slate. */
export function applyPreviewError<TPreview, TResult>(
  state: MutationState<TPreview, TResult>,
  requestGeneration: number,
  error: ArtifactRequestError,
): MutationState<TPreview, TResult> {
  if (requestGeneration !== state.generation) return state;
  return { ...state, phase: "idle", preview: null, error };
}

/** Marks a commit request as in flight, keeping the current preview visible
 * behind it. */
export function startCommitting<TPreview, TResult>(
  state: MutationState<TPreview, TResult>,
): MutationState<TPreview, TResult> {
  return { ...state, phase: "committing", error: null };
}

/** Applies a successful commit, unless superseded. */
export function applyCommitSuccess<TPreview, TResult>(
  state: MutationState<TPreview, TResult>,
  requestGeneration: number,
  result: TResult,
): MutationState<TPreview, TResult> {
  if (requestGeneration !== state.generation) return state;
  return { ...state, phase: "succeeded", result, error: null };
}

/** Applies a failed commit, unless superseded. Falls back to `previewed`
 * (not `idle`): the daemon rejects a stale/consumed/mismatched plan with a
 * specific reason, and the user should see that reason next to the preview
 * that produced it rather than have it silently discarded. The preview
 * itself is no longer valid for a retry — the caller must re-preview to get
 * a fresh plan — but keeping it on screen is what lets the user see what
 * they were trying to do. */
export function applyCommitError<TPreview, TResult>(
  state: MutationState<TPreview, TResult>,
  requestGeneration: number,
  error: ArtifactRequestError,
): MutationState<TPreview, TResult> {
  if (requestGeneration !== state.generation) return state;
  return { ...state, phase: "previewed", error };
}

/**
 * The reason a mutation preview's commit is disabled, or `"none"` when it
 * isn't. Every mutation preview (image tag, image remove, single-scope
 * prune) can be blocked for exactly these two reasons — the provider
 * doesn't support the operation, or something is actively running against
 * the engine — so this one pure function replaces the same branching that
 * would otherwise be duplicated inline in every dialog's markup.
 */
export type MutationBlocker =
  | { kind: "unsupported" }
  | { kind: "active_work"; jobs: number; watchSessions: number }
  | { kind: "none" };

export function describeMutationBlocker(input: {
  /** Whether the provider supports the operation at all — `isCapabilityUsable`
   * for tag/remove, `inventory_supported` for prune. */
  supported: boolean;
  /** Whether the preview actually minted a commit plan. */
  commitEnabled: boolean;
  activeJobs: number;
  activeWatchSessions: number;
}): MutationBlocker {
  if (!input.supported) {
    return { kind: "unsupported" };
  }
  if (!input.commitEnabled) {
    return {
      kind: "active_work",
      jobs: input.activeJobs,
      watchSessions: input.activeWatchSessions,
    };
  }
  return { kind: "none" };
}
