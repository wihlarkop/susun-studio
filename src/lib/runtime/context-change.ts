import type {
  ProjectRuntimeImpactPreview,
  RuntimePreferenceImpactPreview,
} from "$lib/daemon/client";

export type RuntimeContextTarget =
  | { kind: "preference"; profileId: string | null }
  | { kind: "project"; projectId: string; profileId: string | null };

export type ContextImpactPreview = RuntimePreferenceImpactPreview | ProjectRuntimeImpactPreview;

export type ContextChangeState = {
  phase:
    | "idle"
    | "loading_preview"
    | "previewed"
    | "committing"
    | "committed"
    | "cancelled"
    | "failed";
  generation: number;
  target: RuntimeContextTarget | null;
  preview: ContextImpactPreview | null;
  message: string | null;
};

export function createContextChangeState(): ContextChangeState {
  return { phase: "idle", generation: 0, target: null, preview: null, message: null };
}

export function beginContextPreview(
  state: ContextChangeState,
  target: RuntimeContextTarget,
): ContextChangeState {
  return {
    phase: "loading_preview",
    generation: state.generation + 1,
    target,
    preview: null,
    message: null,
  };
}

export function acceptContextPreview(
  state: ContextChangeState,
  generation: number,
  target: RuntimeContextTarget,
  preview: ContextImpactPreview,
): ContextChangeState {
  if (
    state.phase !== "loading_preview" ||
    state.generation !== generation ||
    !sameTarget(state.target, target)
  ) {
    return state;
  }
  return { ...state, phase: "previewed", preview };
}

export function cancelContextChange(state: ContextChangeState): ContextChangeState {
  return {
    phase: "cancelled",
    generation: state.generation + 1,
    target: null,
    preview: null,
    message: null,
  };
}

export function beginContextCommit(state: ContextChangeState): ContextChangeState {
  if (!canCommitContextChange(state)) return state;
  return { ...state, phase: "committing", message: null };
}

export function resolveContextCommit(
  state: ContextChangeState,
  success: boolean,
  reasonCode?: string | null,
): ContextChangeState {
  if (state.phase !== "committing") return state;
  if (success) return { ...state, phase: "committed", preview: null, message: null };
  return { ...state, phase: "failed", preview: null, message: boundedContextReason(reasonCode) };
}

export function canCommitContextChange(state: ContextChangeState): boolean {
  return (
    state.phase === "previewed" &&
    Boolean(state.target && state.preview?.impact_fingerprint && state.preview.change_allowed)
  );
}

export function boundedContextReason(reasonCode: string | null | undefined): string {
  switch (reasonCode) {
    case "active_work":
      return "Stop the running work, then preview this change again.";
    case "target_missing":
      return "The selected runtime is no longer available. Refresh and choose a runtime again.";
    case "target_unavailable":
      return "The selected runtime is not ready. Recheck it and preview the change again.";
    default:
      return "Runtime context changed since preview. Review the current impact and preview again.";
  }
}

export function sameTarget(
  left: RuntimeContextTarget | null,
  right: RuntimeContextTarget,
): boolean {
  if (!left || left.kind !== right.kind || left.profileId !== right.profileId) return false;
  return (
    left.kind !== "project" || (right.kind === "project" && left.projectId === right.projectId)
  );
}

export function isProjectImpactPreview(
  preview: ContextImpactPreview,
): preview is ProjectRuntimeImpactPreview {
  return "project_id" in preview;
}
