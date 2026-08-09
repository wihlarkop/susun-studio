import type {
  RuntimeMigrationInventory,
  RuntimeMigrationInventoryProfile,
  RuntimeMigrationHistory,
  RuntimeMigrationPreview,
  RuntimeMigrationResult,
  RuntimeMigrationRollbackPreview,
  RuntimeMigrationRollbackResult,
  RuntimeMigrationRequest,
} from "$lib/daemon/client";

export type MigrationWorkflow = "connect" | "migrate" | null;

export type MigrationPhase =
  | "idle"
  | "loading_inventory"
  | "editing"
  | "previewing"
  | "previewed"
  | "committing"
  | "committed"
  | "commit_failed"
  | "rollback_previewing"
  | "rollback_previewed"
  | "rollback_committing"
  | "rolled_back"
  | "rollback_failed"
  | "cancelled";

export type MigrationSelection = {
  sourceId: string | null;
  targetId: string | null;
  projectIds: string[];
  inventoryGeneration: number;
  profileRevision: number;
  compatibilityRevision: number;
};

export type MigrationState = {
  phase: MigrationPhase;
  generation: number;
  workflow: MigrationWorkflow;
  inventory: RuntimeMigrationInventory | null;
  selection: MigrationSelection;
  preview: RuntimeMigrationPreview | null;
  previewExpiresAtMs: number | null;
  result: RuntimeMigrationResult | null;
  rollbackPreview: RuntimeMigrationRollbackPreview | null;
  rollbackExpiresAtMs: number | null;
  rollbackConfirmed: boolean;
  rollbackResult: RuntimeMigrationRollbackResult | null;
  error: string | null;
};

export type MigrationSelectionUpdate = Partial<
  Pick<
    MigrationSelection,
    | "sourceId"
    | "targetId"
    | "projectIds"
    | "inventoryGeneration"
    | "profileRevision"
    | "compatibilityRevision"
  >
>;

export const migrationExcludedCategories = [
  "images",
  "containers",
  "volumes",
  "networks",
  "registry_credentials",
  "runtime_settings",
  "runtime_ownership",
  "project_files",
] as const;

export const externalRuntimeMigrationDocsPath = "docs/public/external-runtime-migration.md";

export type MigrationHistoryPresentationEntry = {
  migrationId: string;
  status: "completed" | "failed" | "rolled_back" | "unknown";
  recovery: "available" | "blocked" | "not_applicable";
  statusLabel: string;
  sourceLabel: "Recorded source runtime";
  targetLabel: "Recorded target runtime";
  projectCount: number;
  skippedLabels: string[];
  failureLabels: string[];
  canPrepareRollback: boolean;
  createdAtMs: number;
  completedAtMs: number;
  rolledBackAtMs: number | null;
};

export function createMigrationState(): MigrationState {
  return {
    phase: "idle",
    generation: 0,
    workflow: null,
    inventory: null,
    selection: emptySelection(),
    preview: null,
    previewExpiresAtMs: null,
    result: null,
    rollbackPreview: null,
    rollbackExpiresAtMs: null,
    rollbackConfirmed: false,
    rollbackResult: null,
    error: null,
  };
}

export function resetMigrationDialog(state: MigrationState): MigrationState {
  return { ...createMigrationState(), generation: state.generation + 1 };
}

export function chooseMigrationWorkflow(
  state: MigrationState,
  workflow: Exclude<MigrationWorkflow, null>,
): MigrationState {
  return {
    ...createMigrationState(),
    generation: state.generation + 1,
    workflow,
  };
}

export function beginMigrationInventory(
  state: MigrationState,
  workflow: Exclude<MigrationWorkflow, null> = state.workflow ?? "migrate",
): MigrationState {
  return {
    ...state,
    phase: "loading_inventory",
    generation: state.generation + 1,
    workflow,
    inventory: null,
    selection: emptySelection(),
    preview: null,
    previewExpiresAtMs: null,
    result: null,
    rollbackPreview: null,
    rollbackExpiresAtMs: null,
    rollbackConfirmed: false,
    rollbackResult: null,
    error: null,
  };
}

export function acceptMigrationInventory(
  state: MigrationState,
  generation: number,
  inventory: RuntimeMigrationInventory,
): MigrationState {
  if (state.phase !== "loading_inventory" || state.generation !== generation) return state;
  return { ...state, phase: "editing", inventory, error: null };
}

export function updateMigrationSelection(
  state: MigrationState,
  update: MigrationSelectionUpdate,
): MigrationState {
  if (!state.inventory || !canEditMigration(state.phase)) return state;
  const selection: MigrationSelection = {
    ...state.selection,
    ...update,
    projectIds: canonicalProjectIds(update.projectIds ?? state.selection.projectIds),
  };
  return {
    ...state,
    phase: "editing",
    generation: state.generation + 1,
    selection,
    preview: null,
    previewExpiresAtMs: null,
    rollbackPreview: null,
    rollbackExpiresAtMs: null,
    rollbackConfirmed: false,
    error: null,
  };
}

export function beginMigrationPreview(state: MigrationState, _nowMs: number): MigrationState {
  if (!canPreviewMigration(state)) return state;
  return {
    ...state,
    phase: "previewing",
    generation: state.generation + 1,
    preview: null,
    previewExpiresAtMs: null,
    error: null,
  };
}

export function acceptMigrationPreview(
  state: MigrationState,
  generation: number,
  preview: RuntimeMigrationPreview,
  nowMs: number,
): MigrationState {
  if (state.phase !== "previewing" || state.generation !== generation) return state;
  if (!previewMatchesSelection(preview, state.selection)) {
    return {
      ...state,
      phase: "editing",
      preview: null,
      previewExpiresAtMs: null,
      error: "The runtime context changed. Preview the migration again.",
    };
  }
  const planIsCurrent =
    preview.can_migrate && Boolean(preview.plan_id) && preview.expires_in_seconds;
  return {
    ...state,
    phase: "previewed",
    preview,
    previewExpiresAtMs: planIsCurrent ? nowMs + preview.expires_in_seconds! * 1_000 : null,
    error: null,
  };
}

export function canCommitMigration(state: MigrationState, nowMs: number): boolean {
  return (
    state.phase === "previewed" &&
    Boolean(state.preview?.can_migrate && state.preview.plan_id) &&
    state.previewExpiresAtMs !== null &&
    nowMs < state.previewExpiresAtMs
  );
}

export function beginMigrationCommit(state: MigrationState, nowMs: number): MigrationState {
  if (!canCommitMigration(state, nowMs)) return state;
  return { ...state, phase: "committing", generation: state.generation + 1, error: null };
}

export function acceptMigrationCommit(
  state: MigrationState,
  generation: number,
  result: RuntimeMigrationResult,
): MigrationState {
  if (state.phase !== "committing" || state.generation !== generation) return state;
  if (result.status !== "completed") {
    return {
      ...state,
      phase: "commit_failed",
      preview: null,
      previewExpiresAtMs: null,
      result: null,
      error: "The migration was not completed. Review the current bindings and preview again.",
    };
  }
  return {
    ...state,
    phase: "committed",
    preview: null,
    previewExpiresAtMs: null,
    result,
    error: null,
  };
}

export function beginRollbackPreview(state: MigrationState): MigrationState {
  if (state.phase !== "committed" || !state.result?.rollback_available) return state;
  return {
    ...state,
    phase: "rollback_previewing",
    generation: state.generation + 1,
    rollbackPreview: null,
    rollbackExpiresAtMs: null,
    rollbackConfirmed: false,
    error: null,
  };
}

export function beginHistoryRollback(state: MigrationState, migrationId: string): MigrationState {
  return {
    ...createMigrationState(),
    phase: "rollback_previewing",
    generation: state.generation + 1,
    workflow: "migrate",
    result: {
      migration_id: migrationId,
      status: "completed",
      source_profile_id: "",
      target_profile_id: "",
      project_count: 0,
      skipped_items: [],
      failures: [],
      rollback_available: true,
    },
  };
}

export function acceptRollbackPreview(
  state: MigrationState,
  generation: number,
  preview: RuntimeMigrationRollbackPreview,
  nowMs: number,
): MigrationState {
  if (
    state.phase !== "rollback_previewing" ||
    state.generation !== generation ||
    state.result?.migration_id !== preview.migration_id
  ) {
    return state;
  }
  const planIsCurrent =
    preview.restorable && Boolean(preview.plan_id) && preview.expires_in_seconds;
  return {
    ...state,
    phase: "rollback_previewed",
    rollbackPreview: preview,
    rollbackExpiresAtMs: planIsCurrent ? nowMs + preview.expires_in_seconds! * 1_000 : null,
    rollbackConfirmed: false,
    error: preview.restorable ? null : boundedMigrationError(preview.blocker),
  };
}

export function acknowledgeRollbackConfirmation(state: MigrationState): MigrationState {
  if (state.phase !== "rollback_previewed" || !state.rollbackPreview?.plan_id) return state;
  return { ...state, rollbackConfirmed: true, error: null };
}

export function canCommitRollback(state: MigrationState, nowMs: number): boolean {
  return (
    state.phase === "rollback_previewed" &&
    state.rollbackConfirmed &&
    Boolean(state.rollbackPreview?.restorable && state.rollbackPreview.plan_id) &&
    state.rollbackExpiresAtMs !== null &&
    nowMs < state.rollbackExpiresAtMs
  );
}

export function beginRollbackCommit(state: MigrationState, nowMs: number): MigrationState {
  if (!canCommitRollback(state, nowMs)) return state;
  return {
    ...state,
    phase: "rollback_committing",
    generation: state.generation + 1,
    error: null,
  };
}

export function acceptRollbackCommit(
  state: MigrationState,
  generation: number,
  result: RuntimeMigrationRollbackResult,
): MigrationState {
  if (
    state.phase !== "rollback_committing" ||
    state.generation !== generation ||
    state.rollbackPreview?.migration_id !== result.migration_id
  ) {
    return state;
  }
  if (result.status !== "rolled_back") {
    return {
      ...state,
      phase: "rollback_failed",
      rollbackPreview: null,
      rollbackExpiresAtMs: null,
      rollbackConfirmed: false,
      rollbackResult: null,
      error: "Rollback was not completed. Prepare a new rollback preview.",
    };
  }
  return {
    ...state,
    phase: "rolled_back",
    rollbackPreview: null,
    rollbackExpiresAtMs: null,
    rollbackConfirmed: false,
    rollbackResult: result,
    error: null,
  };
}

export function failMigrationRequest(
  state: MigrationState,
  generation: number,
  error: unknown,
): MigrationState {
  if (state.generation !== generation) return state;
  const message = boundedMigrationError(error);
  switch (state.phase) {
    case "loading_inventory":
      return { ...state, phase: "idle", inventory: null, error: message };
    case "previewing":
      return {
        ...state,
        phase: "editing",
        preview: null,
        previewExpiresAtMs: null,
        error: message,
      };
    case "committing":
      return {
        ...state,
        phase: "commit_failed",
        preview: null,
        previewExpiresAtMs: null,
        error: message,
      };
    case "rollback_previewing":
      return {
        ...state,
        phase: "committed",
        rollbackPreview: null,
        rollbackExpiresAtMs: null,
        rollbackConfirmed: false,
        error: message,
      };
    case "rollback_committing":
      return {
        ...state,
        phase: "rollback_failed",
        rollbackPreview: null,
        rollbackExpiresAtMs: null,
        rollbackConfirmed: false,
        error: message,
      };
    default:
      return state;
  }
}

export function invalidateMigrationForRuntimeRefresh(state: MigrationState): MigrationState {
  const base = {
    ...state,
    generation: state.generation + 1,
    selection: {
      ...state.selection,
      profileRevision: state.selection.profileRevision + 1,
    },
    preview: null,
    previewExpiresAtMs: null,
    rollbackPreview: null,
    rollbackExpiresAtMs: null,
    rollbackConfirmed: false,
  };
  switch (state.phase) {
    case "loading_inventory":
      return {
        ...base,
        phase: "idle",
        inventory: null,
        error: "Runtime state changed. Refresh the migration inventory.",
      };
    case "previewing":
    case "previewed":
    case "editing":
    case "commit_failed":
      return { ...base, phase: "editing", error: null };
    case "committing":
      return {
        ...base,
        phase: "commit_failed",
        result: null,
        error:
          "Runtime state changed while the migration was being confirmed. Refresh before continuing.",
      };
    case "rollback_previewing":
    case "rollback_previewed":
      return { ...base, phase: "committed", error: null };
    case "rollback_committing":
      return {
        ...base,
        phase: "rollback_failed",
        rollbackResult: null,
        error: "Runtime state changed while rollback was being confirmed. Prepare a new preview.",
      };
    default:
      return { ...base, error: null };
  }
}

export function cancelMigration(state: MigrationState): MigrationState {
  return {
    ...state,
    phase: "cancelled",
    generation: state.generation + 1,
    preview: null,
    previewExpiresAtMs: null,
    result: null,
    rollbackPreview: null,
    rollbackExpiresAtMs: null,
    rollbackConfirmed: false,
    rollbackResult: null,
    error: null,
  };
}

export function migrationRequest(state: MigrationState): RuntimeMigrationRequest | null {
  if (!canPreviewMigration(state)) return null;
  return {
    source_profile_id: state.selection.sourceId!,
    target_profile_id: state.selection.targetId!,
    project_ids: [...state.selection.projectIds],
  };
}

export function migrationEligibleSources(
  inventory: RuntimeMigrationInventory,
): RuntimeMigrationInventoryProfile[] {
  return inventory.profiles
    .filter((profile) => profile.explicitly_pinned_project_count > 0)
    .slice()
    .sort((left, right) => left.profile_id.localeCompare(right.profile_id));
}

export function migrationEligibleTargets(
  inventory: RuntimeMigrationInventory,
  sourceId: string | null,
): RuntimeMigrationInventoryProfile[] {
  return inventory.profiles
    .filter(
      (profile) =>
        profile.profile_id !== sourceId &&
        profile.reference_state === "present" &&
        profile.selectable,
    )
    .slice()
    .sort((left, right) => left.profile_id.localeCompare(right.profile_id));
}

export function migrationProjectsForSource(
  inventory: RuntimeMigrationInventory,
  sourceId: string | null,
): RuntimeMigrationInventory["projects"] {
  return inventory.projects.filter(
    (project) => project.explicitly_pinned && project.binding.profile_id === sourceId,
  );
}

export function migrationConfirmationDetails(preview: RuntimeMigrationPreview): {
  source: RuntimeMigrationPreview["source"];
  target: RuntimeMigrationPreview["target"];
  projectCount: number;
  excludedCategories: readonly string[];
  rollbackAvailable: boolean;
  expiresInSeconds: number | null;
} {
  return {
    source: preview.source,
    target: preview.target,
    projectCount: preview.projects.length,
    excludedCategories: migrationExcludedCategories,
    rollbackAvailable: preview.rollback_available,
    expiresInSeconds: preview.expires_in_seconds,
  };
}

export function migrationHistoryPresentation(
  history: RuntimeMigrationHistory,
): MigrationHistoryPresentationEntry[] {
  return history.entries
    .slice()
    .sort((left, right) => right.created_at_ms - left.created_at_ms)
    .slice(0, 50)
    .map((entry) => {
      const canPrepareRollback = entry.status === "completed" && entry.rollback_available;
      const recovery =
        entry.status === "completed"
          ? canPrepareRollback
            ? "available"
            : "blocked"
          : "not_applicable";
      return {
        migrationId: entry.migration_id,
        status: entry.status,
        recovery,
        statusLabel: migrationHistoryStatusLabel(entry.status, recovery),
        sourceLabel: "Recorded source runtime",
        targetLabel: "Recorded target runtime",
        projectCount: entry.project_count,
        skippedLabels: entry.skipped_categories.map(migrationCategoryLabel),
        failureLabels: entry.failure_codes.map(migrationFailureLabel),
        canPrepareRollback,
        createdAtMs: entry.created_at_ms,
        completedAtMs: entry.completed_at_ms,
        rolledBackAtMs: entry.rolled_back_at_ms,
      };
    });
}

export function canAcceptMigrationResponse(
  state: MigrationState,
  responseGeneration: number,
  currentGeneration: number,
): boolean {
  return state.generation === responseGeneration && responseGeneration === currentGeneration;
}

export function boundedMigrationError(error: unknown): string {
  const status =
    typeof error === "object" && error !== null && "status" in error
      ? (error as { status?: unknown }).status
      : undefined;
  if (status === 401) return "Studio could not authorize the migration request.";
  if (status === 404) return "The migration context is no longer available. Refresh and try again.";
  if (status === 422) return "The runtime context changed. Preview the migration again.";
  if (status === 502) return "The selected runtime could not be reached.";
  if (typeof status === "number" && status >= 500)
    return "Studio could not complete the migration request.";
  return "Studio could not reach the daemon. Try again.";
}

export function boundedRollbackBlocker(blocker: string | null): string {
  switch (blocker) {
    case "Stop running work on the migrated runtime before preparing rollback.":
      return "Stop running work on the migrated runtime, then prepare rollback again.";
    case "This migration can no longer be rolled back.":
      return "Rollback is no longer available because the recorded bindings changed.";
    default:
      return "Rollback is not available for the current migration state.";
  }
}

function migrationHistoryStatusLabel(
  status: MigrationHistoryPresentationEntry["status"],
  recovery: MigrationHistoryPresentationEntry["recovery"],
): string {
  if (status === "completed" && recovery === "available") return "Completed - rollback available";
  if (status === "completed" && recovery === "blocked") return "Completed - rollback unavailable";
  if (status === "rolled_back") return "Rolled back";
  if (status === "failed") return "Failed";
  return "Unknown historical state";
}

function migrationCategoryLabel(category: string): string {
  switch (category) {
    case "images":
      return "Images were not copied.";
    case "containers":
      return "Containers were not copied.";
    case "volumes":
      return "Volumes were not copied.";
    case "networks":
      return "Networks were not copied.";
    case "registry_credentials":
      return "Registry credentials were not copied.";
    case "runtime_settings":
      return "Runtime settings were not copied.";
    case "runtime_ownership":
      return "Runtime ownership was not transferred.";
    case "project_files":
      return "Project files were not copied.";
    default:
      return "Additional runtime data was not copied.";
  }
}

function migrationFailureLabel(code: string): string {
  switch (code) {
    case "stale_preview":
      return "The migration context changed before commit.";
    case "active_work":
      return "Active work prevented the migration.";
    case "target_missing":
      return "The target runtime was no longer available.";
    case "target_incompatible":
      return "The target runtime was no longer compatible.";
    case "binding_race":
      return "Project bindings changed before commit.";
    default:
      return "The migration failed before bindings changed.";
  }
}

function emptySelection(): MigrationSelection {
  return {
    sourceId: null,
    targetId: null,
    projectIds: [],
    inventoryGeneration: 0,
    profileRevision: 0,
    compatibilityRevision: 0,
  };
}

function canEditMigration(phase: MigrationPhase): boolean {
  return phase === "editing" || phase === "previewed" || phase === "commit_failed";
}

function canPreviewMigration(state: MigrationState): boolean {
  return (
    canEditMigration(state.phase) &&
    Boolean(
      state.selection.sourceId &&
      state.selection.targetId &&
      state.selection.sourceId !== state.selection.targetId &&
      state.selection.projectIds.length > 0,
    )
  );
}

function canonicalProjectIds(projectIds: string[]): string[] {
  return [...new Set(projectIds)].sort((left, right) => left.localeCompare(right));
}

function previewMatchesSelection(
  preview: RuntimeMigrationPreview,
  selection: MigrationSelection,
): boolean {
  if (
    preview.source.profile_id !== selection.sourceId ||
    preview.target.profile_id !== selection.targetId
  ) {
    return false;
  }
  const returnedProjects = canonicalProjectIds(
    preview.projects.map((project) => project.project_id),
  );
  return (
    returnedProjects.length === selection.projectIds.length &&
    returnedProjects.every((projectId, index) => projectId === selection.projectIds[index]) &&
    preview.projects.every((project) => project.currently_bound_to_source)
  );
}
