import {
  getDaemonBaseUrl,
  importProject as importProjectRequest,
  listProjects,
  readDaemonHealth,
  readSettings,
  updateSettings as updateSettingsRequest,
  type DaemonHealth,
  type ImportProjectRequest,
  type ImportProjectResponse,
  type StudioProject,
  type StudioSettings,
} from "$lib/daemon/client";

export type HealthState =
  | { kind: "checking"; label: "Checking"; detail: string; health?: undefined }
  | { kind: "connected"; label: "Connected"; detail: string; health: DaemonHealth }
  | { kind: "disconnected"; label: "Disconnected"; detail: string; health?: undefined };

const healthPollIntervalMs = 5000;

export function createDaemonState() {
  let healthState = $state<HealthState>({
    kind: "checking",
    label: "Checking",
    detail: `Reading ${getDaemonBaseUrl()}/v1/health`,
  });
  let projects = $state<StudioProject[]>([]);
  let settings = $state<StudioSettings | undefined>(undefined);
  let workspaceDetail = $state(
    "Persisted projects will appear here after the daemon API is wired.",
  );

  function describeWorkspace(projectList: StudioProject[]): string {
    return projectList.length
      ? `${projectList.length} project${projectList.length === 1 ? "" : "s"} persisted by the local daemon.`
      : "No projects are stored yet. Import will write through the daemon API.";
  }

  async function refresh(signal?: AbortSignal) {
    try {
      const health = await readDaemonHealth(getDaemonBaseUrl(), signal);
      const [projectList, daemonSettings] = await Promise.all([
        listProjects({ signal }),
        readSettings({ signal }),
      ]);

      projects = projectList;
      settings = daemonSettings;
      workspaceDetail = describeWorkspace(projectList);
      healthState = {
        kind: "connected",
        label: "Connected",
        detail: `Daemon ${health.version} using API v${health.api_version}`,
        health,
      };
    } catch (error) {
      if (signal?.aborted) {
        return;
      }

      projects = [];
      settings = undefined;
      workspaceDetail = "Start the local daemon to load projects and settings.";
      healthState = {
        kind: "disconnected",
        label: "Disconnected",
        detail: error instanceof Error ? error.message : "Daemon health request failed",
      };
    }
  }

  $effect(() => {
    const controller = new AbortController();

    refresh(controller.signal);
    const interval = setInterval(() => refresh(controller.signal), healthPollIntervalMs);

    return () => {
      clearInterval(interval);
      controller.abort();
    };
  });

  async function importProject(request: ImportProjectRequest): Promise<ImportProjectResponse> {
    const response = await importProjectRequest(request);

    if (response.project) {
      const nextProjects = await listProjects();
      projects = nextProjects;
      workspaceDetail = describeWorkspace(nextProjects);
    }

    return response;
  }

  // Best-effort persistence: the selection already applies locally the
  // moment the caller sets it, so a failed write here (daemon restarting,
  // network blip) just means it won't be restored next launch — not worth
  // surfacing as an error.
  async function setLastProjectId(projectId: string): Promise<void> {
    const current = settings ?? { default_project_root: "", last_project_id: "" };
    try {
      settings = await updateSettingsRequest({ ...current, last_project_id: projectId });
    } catch {
      // ignore — see comment above
    }
  }

  return {
    get healthState() {
      return healthState;
    },
    get projects() {
      return projects;
    },
    get settings() {
      return settings;
    },
    get workspaceDetail() {
      return workspaceDetail;
    },
    importProject,
    refresh: () => refresh(),
    setLastProjectId,
  };
}
