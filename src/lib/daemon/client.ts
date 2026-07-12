export type DaemonHealth = {
  status: "ok";
  product: "susun-studio";
  version: string;
  api_version: string;
};

export type ProjectSummarySchemaVersion = {
  major: number;
  minor: number;
};

export type StudioProjectResource = {
  name: string;
  external: boolean;
};

export type StudioServicePort = {
  host_ip: string | null;
  published: string | null;
  target: number;
  protocol: string;
};

export type StudioServiceVolume = {
  kind: string;
  source: string | null;
  target: string;
  read_only: boolean;
};

export type StudioServiceSummary = {
  name: string;
  active: boolean;
  image: string | null;
  has_build: boolean;
  profile_count: number;
  profiles: string[];
  port_count: number;
  ports: StudioServicePort[];
  volume_count: number;
  volumes: StudioServiceVolume[];
  network_count: number;
  networks: string[];
  config_count: number;
  configs: string[];
  secret_count: number;
  secrets: string[];
  dependency_count: number;
  dependencies: string[];
};

export type StudioProjectSummary = {
  schema_version: ProjectSummarySchemaVersion;
  project_name: string | null;
  project_instance: string | null;
  service_count: number;
  active_service_count: number;
  network_count: number;
  volume_count: number;
  config_count: number;
  secret_count: number;
  networks: StudioProjectResource[];
  volumes: StudioProjectResource[];
  configs: StudioProjectResource[];
  secrets: StudioProjectResource[];
  has_errors: boolean;
  diagnostic_count: number;
  services: StudioServiceSummary[];
};

export type DiagnosticLabel = {
  primary: boolean;
  message: string;
  source: string | null;
  start: number;
  end: number;
  line: number | null;
  column: number | null;
};

export type Diagnostic = {
  code: string;
  severity: string;
  message: string;
  help: string | null;
  labels: DiagnosticLabel[];
};

export type DiagnosticsPayload = {
  diagnostics: Diagnostic[];
};

export type StudioProject = {
  id: string;
  name: string;
  path: string;
  created_at_ms: number;
  last_analyzed_at_ms: number | null;
  has_errors: boolean | null;
  summary: StudioProjectSummary | null;
  diagnostics: DiagnosticsPayload | null;
  runtime_profile_id: string | null;
};

export type StudioSettings = {
  default_project_root: string;
  last_project_id: string;
};

type ProjectListResponse = {
  projects: StudioProject[];
};

export type ImportProjectRequest = {
  files: string[];
  env_file?: string | null;
  project_name?: string | null;
  profiles?: string[];
  runtime_profile_id?: string | null;
};

export type ImportProjectResponse = {
  project: StudioProject | null;
  summary: StudioProjectSummary | null;
  diagnostics: DiagnosticsPayload;
  has_errors: boolean;
};

export type PlanActionSafety = "safe" | "caution" | "destructive";

export type PlanAction = {
  id: string;
  kind: string;
  resource: string;
  safety: PlanActionSafety;
  reason: string;
  dependencies: string[];
};

export type PlanSummary = {
  total_actions: number;
  safe_actions: number;
  caution_actions: number;
  destructive_actions: number;
};

export type StudioPlan = {
  id: string;
  project_id: string;
  operation: "up" | "down";
  summary: PlanSummary;
  actions: PlanAction[];
  blocked_diagnostics: DiagnosticsPayload | null;
  created_at_ms: number;
};

type PlanListResponse = {
  plans: StudioPlan[];
};

export type EngineHealth = {
  reachable: boolean;
  api_version: string | null;
  error: string | null;
};

export type EngineCapabilities = {
  api_version: string | null;
  supports_health: string;
  supports_named_volumes: string;
  supports_network_aliases: string;
  supports_log_follow: string;
  supports_build: string;
  supports_mount_types: string[];
  max_container_name_length: number | null;
};

export type StudioEngine = {
  id: string;
  provider_kind: string;
  display_name: string;
  enabled: boolean;
  is_default: boolean;
  last_health: EngineHealth | null;
  last_health_at_ms: number | null;
};

type EngineListResponse = {
  engines: StudioEngine[];
};

export type RuntimeDimension = {
  state: string;
  detail: string | null;
};

export type RuntimeAction = {
  id: "install" | "setup" | "start" | "stop" | "restart";
  label: string;
  destructive: boolean;
  enabled: boolean;
  reason: string;
};

export type RuntimeClass = "built_in" | "external_local" | "external_remote";

export type RuntimeOwnershipState =
  | "studio_managed"
  | "external"
  | "ownership_conflict"
  | "ownership_unknown";

export type RuntimeSource =
  | "studio_setup"
  | "provider_discovery"
  | "user_remote"
  | "restored_metadata";

export type RuntimeAvailabilityState = "available" | "missing" | "unknown";

export type RuntimeProfileError = {
  code: string;
  detail: string | null;
  at_ms: number;
};

export type RuntimeManagementCapabilities = {
  can_select: boolean;
  can_forget: boolean;
  can_adopt: boolean;
  requires_recovery: boolean;
  blocks_destructive_actions: boolean;
};

export type RuntimeProfile = {
  id: string;
  provider_id: string;
  provider_runtime_key: string;
  display_name: string;
  product: string;
  platform: string;
  runtime_class: RuntimeClass;
  ownership_state: RuntimeOwnershipState;
  source: RuntimeSource;
  installation: RuntimeDimension;
  process: RuntimeDimension;
  connection: RuntimeDimension;
  endpoint_summary: string | RuntimeEndpointSummary | null;
  availability_state: RuntimeAvailabilityState;
  last_seen_at_ms: number | null;
  missing_since_ms: number | null;
  last_error: RuntimeProfileError | null;
  is_selected: boolean;
  observation_revision: number;
  observed_at_ms: number;
  management: RuntimeManagementCapabilities;
  freshness: string;
};

export type RuntimeEndpointSummary = {
  kind: string;
  redacted: string;
};

export type RuntimeProviderStatus = {
  provider_id: string;
  display_name: string;
  product: string;
  platform: string;
  supported: boolean;
  installation: RuntimeDimension;
  process: RuntimeDimension;
  connection: RuntimeDimension;
  freshness: string;
  summary: string;
  remediation: string[];
  actions: RuntimeAction[];
  profiles: RuntimeProfile[];
};

export type RuntimeStatus = {
  providers: RuntimeProviderStatus[];
};

export type RuntimeActionResult = {
  action: string;
  status: string;
  message: string;
  next_steps: string[];
};

export type TrustedRuntimePlan = {
  plan_id: string;
  provider_id: string;
  action: string;
  label: string;
  destructive: boolean;
  consequence: string;
  elevation: "current_user" | "os_mediated_consent";
  command_summary: string;
  expires_in_seconds: number;
  state: "pending";
};

export type PrepareRuntimeActionResponse = {
  plan?: TrustedRuntimePlan;
  result?: RuntimeActionResult;
};

export type RuntimeLogLine = {
  level: string;
  message: string;
};

type RuntimeLogsResponse = {
  lines: RuntimeLogLine[];
};

export type JobStatus = "running" | "succeeded" | "failed" | "cancelled";

export type JobExecutionSummary = {
  total_actions: number;
  succeeded: number;
  failed: number;
  skipped: number;
  cancelled: number;
};

export type JobAction = {
  id: string;
  action: string;
  resource: string;
};

export type JobActionResult = {
  action_id: string;
  attempts: number;
  error: string | null;
  started_at: { secs_since_epoch: number; nanos_since_epoch: number } | null;
  finished_at: { secs_since_epoch: number; nanos_since_epoch: number } | null;
  status:
    | "pending"
    | "ready"
    | "running"
    | "succeeded"
    | "failed"
    | "skipped_dependency_failed"
    | "cancelled";
};

export type StudioJob = {
  id: string;
  kind: "up" | "down" | "build" | "clean";
  status: JobStatus;
  project_id: string;
  actions: JobAction[];
  result: {
    summary: JobExecutionSummary;
    partial?: boolean;
    plan_id?: string;
    actions?: Record<string, JobActionResult>;
  } | null;
  error: string | null;
  error_code: string | null;
  created_at_ms: number;
  updated_at_ms: number;
};

type JobListResponse = {
  jobs: StudioJob[];
};

export type SnapshotContainer = {
  id: string;
  name: string;
  service: string | null;
  replica: number | null;
  state: string;
  health: string | null;
  image: string | null;
};

export type SnapshotResource = {
  id: string;
  name: string;
};

export type ProjectSnapshot = {
  observed_at_ms: number;
  containers: SnapshotContainer[];
  networks: SnapshotResource[];
  volumes: SnapshotResource[];
};

export type ServiceActionResult = {
  service: string;
  containers: { id: string; state: string }[];
};

export type PortBinding = {
  private_port: number;
  protocol: string;
  host_ip: string | null;
  host_port: string;
};

export type LogStreamLine = {
  service: string;
  source: string;
  line: string;
};

export type EngineEventPayload = {
  kind: string;
  action: string;
  resource_id?: string;
  attributes: Record<string, string>;
  time?: number;
};

export type ExecStreamEvent = {
  kind: "output" | "end" | "error" | "created" | "exited" | "removed";
  source?: string;
  line?: string;
  message?: string;
  exit_code?: number;
  container_id?: string;
};

type DaemonRequestOptions = {
  baseUrl?: string;
  token?: string;
  signal?: AbortSignal;
  method?: "GET" | "POST" | "PUT" | "DELETE";
  body?: unknown;
  auth?: boolean;
};

let daemonBaseUrl = import.meta.env.PUBLIC_SUSUN_STUDIO_DAEMON_URL ?? "http://127.0.0.1:7377";
let daemonToken = import.meta.env.PUBLIC_SUSUN_STUDIO_DAEMON_TOKEN ?? "susun-studio-dev-token";

export function setDaemonConnection(connection: { baseUrl: string; token: string }): void {
  daemonBaseUrl = connection.baseUrl;
  daemonToken = connection.token;
}

export function getDaemonBaseUrl(): string {
  return daemonBaseUrl;
}

export function getDaemonToken(): string {
  return daemonToken;
}

export async function readDaemonHealth(
  baseUrl: string = daemonBaseUrl,
  signal?: AbortSignal,
): Promise<DaemonHealth> {
  return readJson("/v1/health", { baseUrl, signal, auth: false });
}

export async function listProjects(options: DaemonRequestOptions = {}): Promise<StudioProject[]> {
  const response = await readJson<ProjectListResponse>("/v1/projects", options);
  return response.projects;
}

export async function createProject(
  project: Pick<StudioProject, "name" | "path">,
  options: DaemonRequestOptions = {},
): Promise<StudioProject> {
  return readJson("/v1/projects", {
    ...options,
    method: "POST",
    body: project,
  });
}

export async function importProject(
  request: ImportProjectRequest,
  options: DaemonRequestOptions = {},
): Promise<ImportProjectResponse> {
  return readJson("/v1/projects/import", {
    ...options,
    method: "POST",
    body: {
      files: request.files,
      env_file: request.env_file ?? null,
      project_name: request.project_name ?? null,
      profiles: request.profiles ?? [],
    },
  });
}

export async function deleteProject(
  projectId: string,
  options: DaemonRequestOptions = {},
): Promise<{ deleted: boolean }> {
  return readJson(`/v1/projects/${encodeURIComponent(projectId)}`, {
    ...options,
    method: "DELETE",
  });
}

export async function createUpPlan(
  projectId: string,
  options: DaemonRequestOptions = {},
): Promise<StudioPlan> {
  return readJson(`/v1/projects/${encodeURIComponent(projectId)}/plans/up`, {
    ...options,
    method: "POST",
  });
}

export async function createDownPlan(
  projectId: string,
  options: DaemonRequestOptions = {},
): Promise<StudioPlan> {
  return readJson(`/v1/projects/${encodeURIComponent(projectId)}/plans/down`, {
    ...options,
    method: "POST",
  });
}

export async function listProjectPlans(
  projectId: string,
  options: DaemonRequestOptions = {},
): Promise<StudioPlan[]> {
  const response = await readJson<PlanListResponse>(
    `/v1/projects/${encodeURIComponent(projectId)}/plans`,
    options,
  );
  return response.plans;
}

export async function readPlan(
  planId: string,
  options: DaemonRequestOptions = {},
): Promise<StudioPlan> {
  return readJson(`/v1/plans/${encodeURIComponent(planId)}`, options);
}

export async function listEngines(options: DaemonRequestOptions = {}): Promise<StudioEngine[]> {
  const response = await readJson<EngineListResponse>("/v1/engines", options);
  return response.engines;
}

export async function readEngineHealth(
  engineId: string,
  options: DaemonRequestOptions = {},
): Promise<EngineHealth> {
  return readJson(`/v1/engines/${encodeURIComponent(engineId)}/health`, options);
}

export async function readEngineCapabilities(
  engineId: string,
  options: DaemonRequestOptions = {},
): Promise<EngineCapabilities> {
  return readJson(`/v1/engines/${encodeURIComponent(engineId)}/capabilities`, options);
}

export async function readRuntimeStatus(
  options: DaemonRequestOptions = {},
): Promise<RuntimeStatus> {
  return readJson("/v1/runtime/status", options);
}

export async function prepareRuntimeAction(
  providerId: string,
  action: RuntimeAction["id"],
  options: DaemonRequestOptions = {},
): Promise<PrepareRuntimeActionResponse> {
  return readJson(
    `/v1/runtime/providers/${encodeURIComponent(providerId)}/actions/${encodeURIComponent(action)}/prepare`,
    {
      ...options,
      method: "POST",
    },
  );
}

export async function executeRuntimePlan(
  planId: string,
  options: DaemonRequestOptions = {},
): Promise<RuntimeActionResult> {
  return readJson(`/v1/runtime/plans/${encodeURIComponent(planId)}/execute`, {
    ...options,
    method: "POST",
  });
}

export async function cancelRuntimePlan(
  planId: string,
  options: DaemonRequestOptions = {},
): Promise<RuntimeActionResult> {
  return readJson(`/v1/runtime/plans/${encodeURIComponent(planId)}/cancel`, {
    ...options,
    method: "POST",
  });
}

export async function readRuntimeLogs(
  options: DaemonRequestOptions = {},
): Promise<RuntimeLogLine[]> {
  const response = await readJson<RuntimeLogsResponse>("/v1/runtime/logs", options);
  return response.lines;
}

type RuntimeProfilesResponse = {
  profiles: RuntimeProfile[];
};

export async function listRuntimeProfiles(
  options: DaemonRequestOptions = {},
): Promise<RuntimeProfile[]> {
  const response = await readJson<RuntimeProfilesResponse>("/v1/runtime/profiles", options);
  return response.profiles;
}

export type RuntimeMigrationProject = {
  id: string;
  name: string;
  currently_bound_to_source: boolean;
};

export type RuntimeArtifactPolicy = {
  category: string;
  disposition: string;
  exactness: string;
};

export type RuntimeMigrationPreview = {
  source: RuntimeProfile;
  target: RuntimeProfile;
  projects: RuntimeMigrationProject[];
  can_migrate: boolean;
  blockers: string[];
  unavailable_capabilities: string[];
  artifact_policy: RuntimeArtifactPolicy[];
  rollback_available: boolean;
};

export type RuntimeMigrationResult = {
  migration_id: string;
  status: "completed" | "failed";
  source_profile_id: string;
  target_profile_id: string;
  project_count: number;
  skipped_items: string[];
  failures: string[];
  rollback_available: boolean;
};

export type RuntimeMigrationRollbackResult = {
  migration_id: string;
  status: "rolled_back" | "failed" | "unavailable";
  restored_project_count: number;
};

export type RuntimeMigrationRequest = {
  source_profile_id: string;
  target_profile_id: string;
  project_ids: string[];
};

export async function previewRuntimeMigration(
  request: RuntimeMigrationRequest,
  options: DaemonRequestOptions = {},
): Promise<RuntimeMigrationPreview> {
  return readJson("/v1/runtime/migrations/preview", {
    ...options,
    method: "POST",
    body: request,
  });
}

export async function executeRuntimeMigration(
  request: RuntimeMigrationRequest,
  options: DaemonRequestOptions = {},
): Promise<RuntimeMigrationResult> {
  return readJson("/v1/runtime/migrations/execute", {
    ...options,
    method: "POST",
    body: request,
  });
}

export async function rollbackRuntimeMigration(
  migrationId: string,
  options: DaemonRequestOptions = {},
): Promise<RuntimeMigrationRollbackResult> {
  return readJson(`/v1/runtime/migrations/${encodeURIComponent(migrationId)}/rollback`, {
    ...options,
    method: "POST",
  });
}

export type RuntimeDestructiveAction =
  | "repair"
  | "reset_engine_data"
  | "remove_built_in_runtime";

export type RuntimeAffectedCategory = {
  category: string;
  count: number | null;
  exactness: string;
  effect: string;
};

export type RuntimeDestructivePreview = {
  operation_id: string;
  profile_id: string;
  action: RuntimeDestructiveAction;
  allowed: boolean;
  blocker: string | null;
  affected: RuntimeAffectedCategory[];
  preserved: string[];
  requires_fresh_preview: boolean;
};

export async function previewRuntimeDestructiveOperation(
  profileId: string,
  action: RuntimeDestructiveAction,
  options: DaemonRequestOptions = {},
): Promise<RuntimeDestructivePreview> {
  return readJson(
    `/v1/runtime/profiles/${encodeURIComponent(profileId)}/destructive-preview`,
    { ...options, method: "POST", body: { action } },
  );
}

export type RuntimeUninstallPolicy = {
  default_choice: string;
  choices: {
    id: string;
    label: string;
    mutates_external_runtimes: boolean;
    selected_by_default: boolean;
  }[];
  unattended_behavior: string;
  reinstall_rule: string;
};

export async function readRuntimeUninstallPolicy(
  options: DaemonRequestOptions = {},
): Promise<RuntimeUninstallPolicy> {
  return readJson("/v1/runtime/uninstall-policy", options);
}

export async function setProjectEngine(
  projectId: string,
  runtimeProfileId: string | null,
  options: DaemonRequestOptions = {},
): Promise<{ updated: boolean }> {
  return readJson(`/v1/projects/${encodeURIComponent(projectId)}/engine`, {
    ...options,
    method: "PUT",
    body: { runtime_profile_id: runtimeProfileId },
  });
}

export async function selectRuntimeProfile(
  profileId: string,
  options: DaemonRequestOptions = {},
): Promise<{ selected: boolean }> {
  return readJson(`/v1/runtime/profiles/${encodeURIComponent(profileId)}/select`, {
    ...options,
    method: "POST",
  });
}

export async function forgetRuntimeProfile(
  profileId: string,
  options: DaemonRequestOptions = {},
): Promise<{ forgotten: boolean }> {
  return readJson(`/v1/runtime/profiles/${encodeURIComponent(profileId)}/forget`, {
    ...options,
    method: "POST",
  });
}

export async function adoptRuntimeProfile(
  profileId: string,
  options: DaemonRequestOptions = {},
): Promise<{ adopted: boolean }> {
  return readJson(`/v1/runtime/profiles/${encodeURIComponent(profileId)}/adopt`, {
    ...options,
    method: "POST",
  });
}

export type PruneScope = "containers" | "networks" | "volumes" | "images";

export type PruneReport = {
  containers_removed: string[];
  networks_removed: string[];
  volumes_removed: string[];
  images_removed: string[];
  space_reclaimed_bytes: number;
};

export async function pruneEngine(
  engineId: string,
  scopes: PruneScope[],
  allImages: boolean = false,
  options: DaemonRequestOptions = {},
): Promise<PruneReport> {
  return readJson(`/v1/engines/${encodeURIComponent(engineId)}/prune`, {
    ...options,
    method: "POST",
    body: { scopes, all_images: allImages },
  });
}

export async function runAction(
  projectId: string,
  action: "up" | "down" | "build" | "clean",
  options: DaemonRequestOptions = {},
): Promise<StudioJob> {
  return readJson(`/v1/projects/${encodeURIComponent(projectId)}/actions/${action}`, {
    ...options,
    method: "POST",
  });
}

export async function cancelJob(
  jobId: string,
  options: DaemonRequestOptions = {},
): Promise<{ cancelled: boolean }> {
  return readJson(`/v1/jobs/${encodeURIComponent(jobId)}/cancel`, {
    ...options,
    method: "POST",
  });
}

export async function listJobs(options: DaemonRequestOptions = {}): Promise<StudioJob[]> {
  const response = await readJson<JobListResponse>("/v1/jobs", options);
  return response.jobs;
}

export async function listProjectJobs(
  projectId: string,
  options: DaemonRequestOptions = {},
): Promise<StudioJob[]> {
  const response = await readJson<JobListResponse>(
    `/v1/projects/${encodeURIComponent(projectId)}/jobs`,
    options,
  );
  return response.jobs;
}

export async function readJob(
  jobId: string,
  options: DaemonRequestOptions = {},
): Promise<StudioJob> {
  return readJson(`/v1/jobs/${encodeURIComponent(jobId)}`, options);
}

// Native EventSource cannot send an Authorization header, so we first make an
// authenticated POST for a short-lived, single-use, scope-bound ticket and put
// only that ticket in the stream URL. The long-lived token never hits a URL.
async function openTicketStream(
  ticketPath: string,
  streamPath: string = ticketPath,
  body?: unknown,
): Promise<EventSource> {
  const { ticket } = await readJson<{ ticket: string; expires_at_ms: number }>(ticketPath, {
    method: "POST",
    body: body ?? {},
  });
  const url = new URL(streamPath, normalizeBaseUrl(daemonBaseUrl));
  url.searchParams.set("ticket", ticket);
  return new EventSource(url);
}

export async function subscribeJobEvents(jobId: string): Promise<EventSource> {
  return openTicketStream(
    `/v1/jobs/${encodeURIComponent(jobId)}/events/ticket`,
    `/v1/jobs/${encodeURIComponent(jobId)}/events`,
  );
}

export type WatchAction = "rebuild" | "restart" | "sync" | "sync_restart";

export type SyncSpec = {
  service: string;
  host_path: string;
  container_path: string;
};

export type StudioWatchSession = {
  id: string;
  project_id: string;
  status: "running" | "stopped" | "failed";
  action: WatchAction;
  services: string[];
  sync: SyncSpec[];
  watch_paths: string[];
  debounce_ms: number;
  track_restart_as_job: boolean;
  last_action_status: string | null;
  last_action_error: string | null;
  error: string | null;
  created_at_ms: number;
  updated_at_ms: number;
};

type WatchListResponse = {
  sessions: StudioWatchSession[];
};

export type StartWatchRequest = {
  action: WatchAction;
  services?: string[];
  sync?: SyncSpec[];
  watch_paths?: string[];
  debounce_ms?: number;
  track_restart_as_job?: boolean;
};

export async function startWatch(
  projectId: string,
  request: StartWatchRequest,
  options: DaemonRequestOptions = {},
): Promise<StudioWatchSession> {
  return readJson(`/v1/projects/${encodeURIComponent(projectId)}/watch`, {
    ...options,
    method: "POST",
    body: request,
  });
}

export async function listProjectWatchSessions(
  projectId: string,
  options: DaemonRequestOptions = {},
): Promise<StudioWatchSession[]> {
  const response = await readJson<WatchListResponse>(
    `/v1/projects/${encodeURIComponent(projectId)}/watch`,
    options,
  );
  return response.sessions;
}

export async function readWatchSession(
  watchId: string,
  options: DaemonRequestOptions = {},
): Promise<StudioWatchSession> {
  return readJson(`/v1/watch/${encodeURIComponent(watchId)}`, options);
}

export async function stopWatchSession(
  watchId: string,
  options: DaemonRequestOptions = {},
): Promise<{ stopped: boolean }> {
  return readJson(`/v1/watch/${encodeURIComponent(watchId)}/stop`, {
    ...options,
    method: "POST",
  });
}

export async function subscribeWatchEvents(watchId: string): Promise<EventSource> {
  return openTicketStream(
    `/v1/watch/${encodeURIComponent(watchId)}/events/ticket`,
    `/v1/watch/${encodeURIComponent(watchId)}/events`,
  );
}

export async function readProjectSnapshot(
  projectId: string,
  options: DaemonRequestOptions = {},
): Promise<ProjectSnapshot> {
  return readJson(`/v1/projects/${encodeURIComponent(projectId)}/snapshot`, options);
}

export async function serviceAction(
  projectId: string,
  service: string,
  action: "start" | "stop" | "restart",
  options: DaemonRequestOptions = {},
): Promise<ServiceActionResult> {
  return readJson(
    `/v1/projects/${encodeURIComponent(projectId)}/services/${encodeURIComponent(service)}/${action}`,
    { ...options, method: "POST" },
  );
}

export async function waitService(
  projectId: string,
  service: string,
  options: DaemonRequestOptions = {},
): Promise<{ exit_code: number }> {
  return readJson(
    `/v1/projects/${encodeURIComponent(projectId)}/services/${encodeURIComponent(service)}/wait`,
    { ...options, method: "POST" },
  );
}

export async function readServicePorts(
  projectId: string,
  service: string,
  options: DaemonRequestOptions = {},
): Promise<{ bindings: PortBinding[] }> {
  return readJson(
    `/v1/projects/${encodeURIComponent(projectId)}/services/${encodeURIComponent(service)}/ports`,
    options,
  );
}

export async function copyService(
  projectId: string,
  service: string,
  body: { direction: "to_container" | "from_container"; host_path: string; container_path: string },
  options: DaemonRequestOptions = {},
): Promise<{ copied: boolean }> {
  return readJson(
    `/v1/projects/${encodeURIComponent(projectId)}/services/${encodeURIComponent(service)}/cp`,
    { ...options, method: "POST", body },
  );
}

export async function openLogStream(
  projectId: string,
  opts: { service?: string; tail?: number } = {},
): Promise<EventSource> {
  const path = `/v1/projects/${encodeURIComponent(projectId)}/streams/logs`;
  return openTicketStream(path, path, opts);
}

export async function openEventStream(projectId: string): Promise<EventSource> {
  const path = `/v1/projects/${encodeURIComponent(projectId)}/streams/events`;
  return openTicketStream(path, path);
}

export async function openExecStream(
  projectId: string,
  service: string,
  body: { command: string[]; user?: string; working_dir?: string },
): Promise<EventSource> {
  const path = `/v1/projects/${encodeURIComponent(projectId)}/services/${encodeURIComponent(service)}/streams/exec`;
  return openTicketStream(path, path, body);
}

export async function openRunStream(
  projectId: string,
  service: string,
  body: { command?: string[] } = {},
): Promise<EventSource> {
  const path = `/v1/projects/${encodeURIComponent(projectId)}/services/${encodeURIComponent(service)}/streams/run`;
  return openTicketStream(path, path, body);
}

export type DiagnosticsJobError = {
  id: string;
  kind: string;
  error: string | null;
  error_code: string | null;
  created_at_ms: number;
};

export type DiagnosticsEngineStatus = {
  id: string;
  display_name: string;
  reachable: boolean;
};

export type DiagnosticsReport = {
  daemon_version: string;
  api_version: string;
  os: string;
  arch: string;
  db_file_name: string;
  db_size_bytes: number | null;
  project_count: number;
  recent_job_errors: DiagnosticsJobError[];
  engines: DiagnosticsEngineStatus[];
};

export async function readDiagnostics(
  options: DaemonRequestOptions = {},
): Promise<DiagnosticsReport> {
  return readJson("/v1/diagnostics", options);
}

export async function readSettings(options: DaemonRequestOptions = {}): Promise<StudioSettings> {
  return readJson("/v1/settings", options);
}

export async function updateSettings(
  settings: StudioSettings,
  options: DaemonRequestOptions = {},
): Promise<StudioSettings> {
  return readJson("/v1/settings", {
    ...options,
    method: "PUT",
    body: settings,
  });
}

async function readJson<T>(path: string, options: DaemonRequestOptions = {}): Promise<T> {
  const baseUrl = options.baseUrl ?? daemonBaseUrl;
  const headers = new Headers({ accept: "application/json" });

  if (options.auth ?? true) {
    headers.set("authorization", `Bearer ${options.token ?? daemonToken}`);
  }

  if (options.body !== undefined) {
    headers.set("content-type", "application/json");
  }

  const response = await fetch(new URL(path, normalizeBaseUrl(baseUrl)), {
    body: options.body === undefined ? undefined : JSON.stringify(options.body),
    headers,
    method: options.method ?? "GET",
    signal: options.signal,
  });

  if (!response.ok) {
    throw new Error(`Daemon request to ${path} failed with HTTP ${response.status}`);
  }

  return (await response.json()) as T;
}

function normalizeBaseUrl(baseUrl: string): string {
  return baseUrl.endsWith("/") ? baseUrl : `${baseUrl}/`;
}
