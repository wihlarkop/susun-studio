export type DaemonHealth = {
  status: "ok";
  product: "susun-studio";
  version: string;
  api_version: string;
};

export type StudioProject = {
  id: string;
  name: string;
  path: string;
  created_at_ms: number;
};

export type StudioSettings = {
  default_project_root: string;
};

type ProjectListResponse = {
  projects: StudioProject[];
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