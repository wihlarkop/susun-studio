import { describe, expect, it } from "vitest";
import {
  applyCommitError,
  applyCommitSuccess,
  applyPreviewError,
  applyPreviewSuccess,
  describeMutationBlocker,
  initialMutationState,
  resetMutation,
  startCommitting,
  startPreviewing,
} from "./mutation-state";

type Preview = { commit_enabled: boolean; plan_id: string | null };
type Result = { deleted: string[] };

describe("resetMutation", () => {
  it("builds a cleared idle state from a plain generation number", () => {
    const reset = resetMutation<Preview, Result>(3);
    expect(reset).toEqual({
      phase: "idle",
      preview: null,
      result: null,
      error: null,
      generation: 3,
    });
  });
});

describe("preview flow", () => {
  it("moves to previewing, clearing any prior preview/result/error", () => {
    const state = applyPreviewError(startPreviewing(initialMutationState<Preview, Result>()), 0, {
      status: 422,
      message: "stale",
    });
    const next = startPreviewing(state);
    expect(next.phase).toBe("previewing");
    expect(next.preview).toBeNull();
    expect(next.error).toBeNull();
  });

  it("applies a same-generation preview success", () => {
    const state = startPreviewing(initialMutationState<Preview, Result>());
    const next = applyPreviewSuccess(state, state.generation, {
      commit_enabled: true,
      plan_id: "rap_1",
    });
    expect(next.phase).toBe("previewed");
    expect(next.preview).toEqual({ commit_enabled: true, plan_id: "rap_1" });
  });

  it("ignores a preview success from a superseded generation", () => {
    const armedForRowB = resetMutation<Preview, Result>(1);
    const result = applyPreviewSuccess(armedForRowB, 0 /* row A's generation */, {
      commit_enabled: true,
      plan_id: "rap_row_a",
    });
    expect(result).toBe(armedForRowB);
    expect(result.preview).toBeNull();
  });

  it("falls back to idle on a same-generation preview error", () => {
    const state = startPreviewing(initialMutationState<Preview, Result>());
    const next = applyPreviewError(state, state.generation, {
      status: 404,
      message: "gone",
    });
    expect(next.phase).toBe("idle");
    expect(next.preview).toBeNull();
    expect(next.error).toEqual({ status: 404, message: "gone" });
  });

  it("ignores a preview error from a superseded generation", () => {
    const armedForRowB = resetMutation<Preview, Result>(1);
    const result = applyPreviewError(armedForRowB, 0, { status: 502, message: "unreachable" });
    expect(result).toBe(armedForRowB);
    expect(result.error).toBeNull();
  });
});

describe("commit flow", () => {
  function previewed(): ReturnType<typeof applyPreviewSuccess<Preview, Result>> {
    const state = startPreviewing(initialMutationState<Preview, Result>());
    return applyPreviewSuccess(state, state.generation, { commit_enabled: true, plan_id: "rap_1" });
  }

  it("marks committing while keeping the preview visible", () => {
    const state = previewed();
    const next = startCommitting(state);
    expect(next.phase).toBe("committing");
    expect(next.preview).toEqual(state.preview);
  });

  it("applies a same-generation commit success", () => {
    const state = startCommitting(previewed());
    const next = applyCommitSuccess(state, state.generation, { deleted: ["sha256:abc"] });
    expect(next.phase).toBe("succeeded");
    expect(next.result).toEqual({ deleted: ["sha256:abc"] });
  });

  it("ignores a commit success from a superseded generation (dialog moved to a new target)", () => {
    const committing = startCommitting(previewed());
    const movedOn = resetMutation<Preview, Result>(committing.generation + 1);
    const result = applyCommitSuccess(movedOn, committing.generation, {
      deleted: ["sha256:stale"],
    });
    expect(result).toBe(movedOn);
    expect(result.result).toBeNull();
  });

  it("falls back to commit_failed (not previewed, not idle) on a same-generation commit error, keeping the plan's preview on screen", () => {
    const state = startCommitting(previewed());
    const next = applyCommitError(state, state.generation, {
      status: 422,
      message: "This request could not be completed right now.",
    });
    expect(next.phase).toBe("commit_failed");
    expect(next.preview).toEqual(state.preview);
    expect(next.error).toEqual({
      status: 422,
      message: "This request could not be completed right now.",
    });
  });

  it('commit_failed is distinct from previewed so a caller gating commit on phase === "previewed" can never replay a rejected plan', () => {
    const state = startCommitting(previewed());
    const next = applyCommitError(state, state.generation, {
      status: 422,
      message: "plan already consumed",
    });
    expect(next.phase).not.toBe("previewed");
    // The preview's own commit_enabled flag is still true (it reflects what
    // the daemon reported at preview time, not the plan's current spent
    // state) — callers must gate on `phase`, not on this field alone.
    expect(next.preview?.commit_enabled).toBe(true);
  });

  it("ignores a commit error from a superseded generation", () => {
    const committing = startCommitting(previewed());
    const movedOn = resetMutation<Preview, Result>(committing.generation + 1);
    const result = applyCommitError(movedOn, committing.generation, {
      status: 422,
      message: "stale",
    });
    expect(result).toBe(movedOn);
    expect(result.error).toBeNull();
  });
});

describe("describeMutationBlocker", () => {
  it("reports unsupported ahead of active work, even if both are true", () => {
    const blocker = describeMutationBlocker({
      supported: false,
      commitEnabled: false,
      activeJobs: 2,
      activeWatchSessions: 1,
    });
    expect(blocker).toEqual({ kind: "unsupported" });
  });

  it("reports active work with its counts when the capability is supported but nothing was minted", () => {
    const blocker = describeMutationBlocker({
      supported: true,
      commitEnabled: false,
      activeJobs: 2,
      activeWatchSessions: 1,
    });
    expect(blocker).toEqual({ kind: "active_work", jobs: 2, watchSessions: 1 });
  });

  it("reports no blocker when supported and a commit plan was minted", () => {
    const blocker = describeMutationBlocker({
      supported: true,
      commitEnabled: true,
      activeJobs: 0,
      activeWatchSessions: 0,
    });
    expect(blocker).toEqual({ kind: "none" });
  });
});
