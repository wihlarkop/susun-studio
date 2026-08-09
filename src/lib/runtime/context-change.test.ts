import { describe, expect, it } from "vitest";
import type { RuntimePreferenceImpactPreview } from "$lib/daemon/client";
import {
  acceptContextPreview,
  beginContextCommit,
  beginContextPreview,
  boundedContextReason,
  canCommitContextChange,
  createContextChangeState,
  resolveContextCommit,
  isProjectImpactPreview,
} from "./context-change";

const preview: RuntimePreferenceImpactPreview = {
  current: {
    source: "global_preference",
    state: "ready",
    profile_id: "old",
    runtime_class: "external_local",
    display_name: "Old",
  },
  target: {
    source: "global_preference",
    state: "ready",
    profile_id: "new",
    runtime_class: "external_local",
    display_name: "New",
  },
  inheriting_project_count: 1,
  explicitly_pinned_project_count: 1,
  affected_active_jobs: 0,
  affected_active_watch_sessions: 0,
  change_allowed: true,
  reason_code: null,
  is_noop: false,
  impact_fingerprint: "opaque",
};

describe("runtime context change state", () => {
  it("cannot commit without a current exact preview", () => {
    expect(canCommitContextChange(createContextChangeState())).toBe(false);
  });

  it("invalidates stale previews and never reuses a failed plan", () => {
    const target = { kind: "preference" as const, profileId: "new" };
    const loading = beginContextPreview(createContextChangeState(), target);
    const stale = acceptContextPreview(loading, loading.generation - 1, target, preview);
    expect(stale.phase).toBe("loading_preview");

    const ready = acceptContextPreview(loading, loading.generation, target, preview);
    expect(canCommitContextChange(ready)).toBe(true);
    const failed = resolveContextCommit(beginContextCommit(ready), false, "active_work");
    expect(failed).toMatchObject({ phase: "failed", preview: null });
    expect(canCommitContextChange(failed)).toBe(false);
  });

  it("keeps bounded active-work guidance", () => {
    expect(boundedContextReason("active_work")).toBe(
      "Stop the running work, then preview this change again.",
    );
  });

  it("keeps global inherited impact separate from a single project pin", () => {
    expect(isProjectImpactPreview(preview)).toBe(false);
    const projectPreview = {
      ...preview,
      project_id: "project-a",
    };
    delete (projectPreview as Partial<RuntimePreferenceImpactPreview>).inheriting_project_count;
    delete (projectPreview as Partial<RuntimePreferenceImpactPreview>)
      .explicitly_pinned_project_count;
    expect(isProjectImpactPreview(projectPreview)).toBe(true);
  });

  it("disables commits for unavailable targets and active-work blockers", () => {
    const target = { kind: "preference" as const, profileId: "new" };
    const loading = beginContextPreview(createContextChangeState(), target);
    const unavailable = acceptContextPreview(loading, loading.generation, target, {
      ...preview,
      change_allowed: false,
      reason_code: "target_unavailable",
    });
    expect(canCommitContextChange(unavailable)).toBe(false);
  });
});
