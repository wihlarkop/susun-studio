import type {
  RuntimeMigrationInventory,
  RuntimeMigrationInventoryProfile,
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

export function beginMigrationInventory(state: MigrationState): MigrationState {
  return {
    ...state,
    phase: "loading_inventory",
    generation: state.generation + 1,
    workflow: "migrate",
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
