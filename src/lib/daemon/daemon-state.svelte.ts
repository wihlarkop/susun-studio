import {
  defaultDaemonBaseUrl,
  importProject as importProjectRequest,
  listProjects,
  readDaemonHealth,
  readSettings,
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

export function createDaemonState() {
  let healthState = $state<HealthState>({
    kind: "checking",
    label: "Checking",
    detail: `Reading ${defaultDaemonBaseUrl}/v1/health`,
  });
  let projects = $state<StudioProject[]>([]);
  let settings = $state<StudioSettings | undefined>(undefined);
  let workspaceDetail = $state(
    "Persisted projects will appear here after the daemon API is wired.",
  );

  $effect(() => {
    const controller = new AbortController();

    async function refreshDaemonState() {
      try {
        const health = await readDaemonHealth(defaultDaemonBaseUrl, controller.signal);
        const [projectList, daemonSettings] = await Promise.all([
          listProjects({ signal: controller.signal }),
          readSettings({ signal: controller.signal }),
        ]);

        projects = projectList;
        settings = daemonSettings;
        workspaceDetail = projectList.length
          ? `${projectList.length} project${projectList.length === 1 ? "" : "s"} persisted by the local daemon.`
          : "No projects are stored yet. Import will write through the daemon API.";
        healthState = {
          kind: "connected",
          label: "Connected",
          detail: `Daemon ${health.version} using API v${health.api_version}`,
          health,
        };
      } catch (error) {
        if (controller.signal.aborted) {
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

    refreshDaemonState();

    return () => controller.abort();
  });

  async function importProject(request: ImportProjectRequest): Promise<ImportProjectResponse> {
    const response = await importProjectRequest(request);

    if (response.project) {
      const nextProjects = await listProjects();
      projects = nextProjects;
      workspaceDetail = nextProjects.length
        ? `${nextProjects.length} project${nextProjects.length === 1 ? "" : "s"} persisted by the local daemon.`
        : "No projects are stored yet. Import will write through the daemon API.";
    }

    return response;
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
  };
}
