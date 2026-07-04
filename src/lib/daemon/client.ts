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
};

export type StudioSettings = {
  default_project_root: string;
};

type ProjectListResponse = {
  projects: StudioProject[];
};

export type ImportProjectRequest = {
  files: string[];
  env_file?: string | null;
  project_name?: string | null;
  profiles?: string[];
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

export type StudioJob = {
  id: string;
  kind: "up" | "down" | "build";
  status: JobStatus;
  project_id: string;
  actions: JobAction[];
  result: { summary: JobExecutionSummary } | null;
  error: string | null;
  created_at_ms: number;
  updated_at_ms: number;
};

type JobListResponse = {
  jobs: StudioJob[];
};

type DaemonRequestOptions = {
  baseUrl?: string;
  token?: string;
  signal?: AbortSignal;
  method?: "GET" | "POST" | "PUT";
  body?: unknown;
  auth?: boolean;
};

export const defaultDaemonBaseUrl =
  import.meta.env.PUBLIC_SUSUN_STUDIO_DAEMON_URL ?? "http://127.0.0.1:7377";

export const defaultDaemonToken =
  import.meta.env.PUBLIC_SUSUN_STUDIO_DAEMON_TOKEN ?? "susun-studio-dev-token";

export async function readDaemonHealth(
  baseUrl: string = defaultDaemonBaseUrl,
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

export async function runAction(
  projectId: string,
  action: "up" | "down" | "build",
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

export async function readJob(
  jobId: string,
  options: DaemonRequestOptions = {},
): Promise<StudioJob> {
  return readJson(`/v1/jobs/${encodeURIComponent(jobId)}`, options);
}

// Native EventSource cannot send an Authorization header, so we first make an
// authenticated POST for a short-lived, single-use, job-scoped ticket and put
// only that ticket in the stream URL. The long-lived token never hits a URL.
export async function subscribeJobEvents(jobId: string): Promise<EventSource> {
  const { ticket } = await readJson<{ ticket: string; expires_at_ms: number }>(
    `/v1/jobs/${encodeURIComponent(jobId)}/events/ticket`,
    { method: "POST" },
  );
  const url = new URL(
    `/v1/jobs/${encodeURIComponent(jobId)}/events`,
    normalizeBaseUrl(defaultDaemonBaseUrl),
  );
  url.searchParams.set("ticket", ticket);
  return new EventSource(url);
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
  const baseUrl = options.baseUrl ?? defaultDaemonBaseUrl;
  const headers = new Headers({ accept: "application/json" });

  if (options.auth ?? true) {
    headers.set("authorization", `Bearer ${options.token ?? defaultDaemonToken}`);
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
