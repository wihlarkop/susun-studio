import { describe, expect, it } from "vitest";
import type {
  RuntimeMigrationInventory,
  RuntimeMigrationPreview,
  RuntimeMigrationRollbackPreview,
} from "$lib/daemon/client";
import {
  acceptMigrationCommit,
  acceptMigrationInventory,
  acceptMigrationPreview,
  acceptRollbackPreview,
  acknowledgeRollbackConfirmation,
  beginMigrationCommit,
  beginMigrationInventory,
  beginMigrationPreview,
  beginRollbackCommit,
  beginRollbackPreview,
  boundedMigrationError,
  boundedRollbackBlocker,
  canAcceptMigrationResponse,
  canCommitMigration,
  canCommitRollback,
  cancelMigration,
  chooseMigrationWorkflow,
  createMigrationState,
  failMigrationRequest,
  migrationEligibleSources,
  migrationEligibleTargets,
  migrationConfirmationDetails,
  migrationExcludedCategories,
  invalidateMigrationForRuntimeRefresh,
  migrationProjectsForSource,
  updateMigrationSelection,
} from "./migration-state";

const inventory = {
  global_binding: {
    source: "global_preference",
    state: "ready",
    profile_id: "podman",
    runtime_class: "external_local",
    display_name: "Existing Podman",
  },
  profiles: [
    {
      profile_id: "podman",
      provider_id: "windows-podman",
      display_name: "Existing Podman",
      runtime_class: "external_local",
      ownership_state: "external",
      availability_state: "available",
      reference_state: "present",
      selectable: true,
      is_preferred: true,
      explicitly_pinned_project_count: 2,
    },
    {
      profile_id: "missing",
      provider_id: null,
      display_name: "Missing runtime profile",
      runtime_class: null,
      ownership_state: null,
      availability_state: "missing",
      reference_state: "missing",
      selectable: false,
      is_preferred: false,
      explicitly_pinned_project_count: 1,
    },
    {
      profile_id: "docker",
      provider_id: "windows-docker-desktop",
      display_name: "Docker Desktop",
      runtime_class: "external_local",
      ownership_state: "external",
      availability_state: "available",
      reference_state: "present",
      selectable: true,
      is_preferred: false,
      explicitly_pinned_project_count: 0,
    },
  ],
  projects: [
    {
      project_id: "project-a",
      binding: {
        source: "project_pin",
        state: "ready",
        profile_id: "podman",
        runtime_class: "external_local",
        display_name: "Existing Podman",
      },
      explicitly_pinned: true,
      selectable: true,
    },
    {
      project_id: "project-b",
      binding: {
        source: "global_preference",
        state: "ready",
        profile_id: "podman",
        runtime_class: "external_local",
        display_name: "Existing Podman",
      },
      explicitly_pinned: false,
      selectable: false,
    },
    {
      project_id: "project-c",
      binding: {
        source: "project_pin",
        state: "missing",
        profile_id: "missing",
        runtime_class: null,
        display_name: "Missing runtime profile",
      },
      explicitly_pinned: true,
      selectable: true,
    },
  ],
} as RuntimeMigrationInventory;

const preview = {
  source: inventory.projects[0].binding,
  target: {
    source: "project_pin",
    state: "ready",
    profile_id: "docker",
    runtime_class: "external_local",
    display_name: "Docker Desktop",
  },
  projects: [{ project_id: "project-a", currently_bound_to_source: true }],
  can_migrate: true,
  blockers: [],
  unavailable_capabilities: ["volumes", "runtime_ownership"],
  artifact_policy: [],
  rollback_available: true,
  plan_id: "opaque-migration-plan",
  expires_in_seconds: 60,
} as RuntimeMigrationPreview;

const rollbackPreview = {
  migration_id: "migration-a",
  source: preview.source,
  target: preview.target,
  project_count: 1,
  excluded_categories: [...migrationExcludedCategories],
  restorable: true,
  blocker: null,
  plan_id: "opaque-rollback-plan",
  expires_in_seconds: 60,
} as RuntimeMigrationRollbackPreview;

function readyMigrationState() {
  const loading = beginMigrationInventory(createMigrationState());
  const editing = acceptMigrationInventory(loading, loading.generation, inventory);
  return updateMigrationSelection(editing, {
    sourceId: "podman",
    targetId: "docker",
    projectIds: ["project-a"],
  });
}

describe("runtime migration state", () => {
  it("canonicalizes selected pins without mutating inventory and invalidates previews on context changes", () => {
    const editing = readyMigrationState();
    const before = inventory.projects.map((project) => project.project_id);
    const selected = updateMigrationSelection(editing, {
      projectIds: ["project-a", "project-a"],
    });
    expect(selected.selection.projectIds).toEqual(["project-a"]);
    expect(inventory.projects.map((project) => project.project_id)).toEqual(before);

    const previewing = beginMigrationPreview(selected, 1_000);
    const previewed = acceptMigrationPreview(previewing, previewing.generation, preview, 1_000);
    expect(canCommitMigration(previewed, 1_001)).toBe(true);

    for (const update of [
      { sourceId: "missing" },
      { targetId: "podman" },
      { projectIds: ["project-b"] },
      { inventoryGeneration: 1 },
      { profileRevision: 1 },
      { compatibilityRevision: 1 },
    ]) {
      const changed = updateMigrationSelection(previewed, update);
      expect(changed).toMatchObject({ phase: "editing", preview: null });
      expect(canCommitMigration(changed, 1_001)).toBe(false);
    }
  });

  it("requires a current unexpired opaque plan and never replays a failed migration commit", () => {
    const previewing = beginMigrationPreview(readyMigrationState(), 1_000);
    const previewed = acceptMigrationPreview(previewing, previewing.generation, preview, 1_000);
    expect(canCommitMigration(previewed, 60_999)).toBe(true);
    expect(canCommitMigration(previewed, 61_001)).toBe(false);

    const committing = beginMigrationCommit(previewed, 1_001);
    const failed = acceptMigrationCommit(committing, committing.generation, {
      migration_id: "migration-a",
      status: "failed",
      source_profile_id: "podman",
      target_profile_id: "docker",
      project_count: 1,
      skipped_items: [],
      failures: ["stale_preview"],
      rollback_available: false,
    });
    expect(failed).toMatchObject({ phase: "commit_failed", preview: null });
    expect(canCommitMigration(failed, 1_002)).toBe(false);
  });

  it("does not replay committed or cancelled plans and ignores stale completions", () => {
    const previewing = beginMigrationPreview(readyMigrationState(), 1_000);
    const previewed = acceptMigrationPreview(previewing, previewing.generation, preview, 1_000);
    const committing = beginMigrationCommit(previewed, 1_001);
    const committed = acceptMigrationCommit(committing, committing.generation, {
      migration_id: "migration-a",
      status: "completed",
      source_profile_id: "podman",
      target_profile_id: "docker",
      project_count: 1,
      skipped_items: [],
      failures: [],
      rollback_available: true,
    });
    expect(canCommitMigration(committed, 1_002)).toBe(false);
    const cancelled = cancelMigration(committed);
    expect(canCommitMigration(cancelled, 1_002)).toBe(false);
    expect(acceptMigrationCommit(cancelled, committing.generation, committed.result!)).toEqual(
      cancelled,
    );
  });

  it("requires a separately confirmed rollback preview before it can commit", () => {
    const previewing = beginMigrationPreview(readyMigrationState(), 1_000);
    const previewed = acceptMigrationPreview(previewing, previewing.generation, preview, 1_000);
    const committing = beginMigrationCommit(previewed, 1_001);
    const committed = acceptMigrationCommit(committing, committing.generation, {
      migration_id: "migration-a",
      status: "completed",
      source_profile_id: "podman",
      target_profile_id: "docker",
      project_count: 1,
      skipped_items: [],
      failures: [],
      rollback_available: true,
    });
    const rollbackLoading = beginRollbackPreview(committed);
    expect(rollbackLoading.phase).toBe("rollback_previewing");
    expect(canCommitRollback(rollbackLoading, 1_002)).toBe(false);

    const rollbackReady = acceptRollbackPreview(
      rollbackLoading,
      rollbackLoading.generation,
      rollbackPreview,
      1_002,
    );
    expect(canCommitRollback(rollbackReady, 1_003)).toBe(false);
    const confirmed = acknowledgeRollbackConfirmation(rollbackReady);
    expect(canCommitRollback(confirmed, 1_003)).toBe(true);
    expect(beginRollbackCommit(confirmed, 1_003).phase).toBe("rollback_committing");
  });

  it("ignores stale or aborted completions after the dialog generation changes", () => {
    const loading = beginMigrationInventory(createMigrationState());
    const cancelled = cancelMigration(loading);
    expect(acceptMigrationInventory(cancelled, loading.generation, inventory)).toEqual(cancelled);
    expect(canAcceptMigrationResponse(cancelled, loading.generation, cancelled.generation)).toBe(
      false,
    );
  });

  it("keeps connect-only separate from migration and preserves daemon-owned ownership and exclusions", () => {
    const connecting = chooseMigrationWorkflow(createMigrationState(), "connect");
    expect(connecting).toMatchObject({ workflow: "connect", phase: "idle" });
    expect(connecting.preview).toBeNull();

    const sources = migrationEligibleSources(inventory);
    expect(sources.map((profile) => profile.profile_id)).toEqual(["missing", "podman"]);
    expect(sources.find((profile) => profile.profile_id === "podman")?.ownership_state).toBe(
      "external",
    );
    expect(migrationExcludedCategories).toContain("project_files");
    expect(migrationExcludedCategories).toContain("runtime_ownership");
  });

  it("derives migration controls only from typed inventory and a daemon preview", () => {
    expect(
      migrationProjectsForSource(inventory, "podman").map((project) => project.project_id),
    ).toEqual(["project-a"]);
    expect(
      migrationProjectsForSource(inventory, "missing").map((project) => project.project_id),
    ).toEqual(["project-c"]);
    expect(
      migrationEligibleTargets(inventory, "podman").map((profile) => profile.profile_id),
    ).toEqual(["docker"]);

    expect(migrationConfirmationDetails(preview)).toMatchObject({
      source: preview.source,
      target: preview.target,
      projectCount: 1,
      rollbackAvailable: true,
      expiresInSeconds: 60,
    });
    expect(migrationConfirmationDetails(preview).excludedCategories).toEqual(
      migrationExcludedCategories,
    );
  });

  it("never uses raw daemon content in a user-visible migration error", () => {
    const sensitive = "connect \\.\\pipe\\docker_engine argv=podman token=secret";
    const message = boundedMigrationError({ status: 502, message: sensitive });
    expect(message).toBe("The selected runtime could not be reached.");
    expect(message).not.toContain("docker_engine");
    expect(message).not.toContain("secret");
  });

  it("maps rollback blockers to fixed public guidance", () => {
    expect(
      boundedRollbackBlocker(
        "Stop running work on the migrated runtime before preparing rollback.",
      ),
    ).toBe("Stop running work on the migrated runtime, then prepare rollback again.");
    const unknown = boundedRollbackBlocker("C:/secret podman.exe --host npipe");
    expect(unknown).toBe("Rollback is not available for the current migration state.");
    expect(unknown).not.toContain("secret");
  });

  it("keeps request failures bounded and ignores an older failure after selection changes", () => {
    const previewing = beginMigrationPreview(readyMigrationState(), 1_000);
    const failed = failMigrationRequest(previewing, previewing.generation, {
      status: 502,
      message: "podman.exe --host npipe:////./pipe/secret",
    });
    expect(failed).toMatchObject({
      phase: "editing",
      preview: null,
      error: "The selected runtime could not be reached.",
    });
    const changed = updateMigrationSelection(failed, { profileRevision: 1 });
    expect(failMigrationRequest(changed, previewing.generation, { status: 500 })).toEqual(changed);
  });

  it("invalidates preview and rollback plans when shared runtime state refreshes", () => {
    const previewing = beginMigrationPreview(readyMigrationState(), 1_000);
    const previewed = acceptMigrationPreview(previewing, previewing.generation, preview, 1_000);
    const refreshedPreview = invalidateMigrationForRuntimeRefresh(previewed);
    expect(refreshedPreview).toMatchObject({ phase: "editing", preview: null });
    expect(canCommitMigration(refreshedPreview, 1_001)).toBe(false);

    const committed = acceptMigrationCommit(
      beginMigrationCommit(previewed, 1_001),
      previewed.generation + 1,
      {
        migration_id: "migration-a",
        status: "completed",
        source_profile_id: "podman",
        target_profile_id: "docker",
        project_count: 1,
        skipped_items: [],
        failures: [],
        rollback_available: true,
      },
    );
    const rollbackLoading = beginRollbackPreview(committed);
    const rollbackReady = acceptRollbackPreview(
      rollbackLoading,
      rollbackLoading.generation,
      rollbackPreview,
      1_002,
    );
    const refreshedRollback = invalidateMigrationForRuntimeRefresh(rollbackReady);
    expect(refreshedRollback).toMatchObject({
      phase: "committed",
      rollbackPreview: null,
      rollbackConfirmed: false,
    });
    expect(canCommitRollback(refreshedRollback, 1_003)).toBe(false);
  });
});
