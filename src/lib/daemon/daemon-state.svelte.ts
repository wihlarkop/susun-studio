import {
  getDaemonBaseUrl,
  importProject as importProjectRequest,
  listProjects,
  readDaemonHealth,
  readRuntimeOnboarding,
  readRuntimeStatus,
  readSettings,
  updateSettings as updateSettingsRequest,
  type DaemonHealth,
  type ImportProjectRequest,
  type ImportProjectResponse,
  type RuntimeProfile,
  type RuntimePreference,
  type RuntimeOnboardingState,
  type RuntimeStatus,
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
  let runtimeProfiles = $state<RuntimeProfile[]>([]);
  let runtimeStatus = $state<RuntimeStatus | undefined>(undefined);
  let runtimeOnboarding = $state<RuntimeOnboardingState | undefined>(undefined);
  let settings = $state<StudioSettings | undefined>(undefined);
  let refreshing = $state(false);
  let refreshGeneration = 0;
  let workspaceDetail = $state(
    "Persisted projects will appear here after the daemon API is wired.",
  );

  function describeWorkspace(projectList: StudioProject[]): string {
    return projectList.length
      ? `${projectList.length} project${projectList.length === 1 ? "" : "s"} persisted by the local daemon.`
      : "No projects are stored yet. Import will write through the daemon API.";
  }

  async function refresh(signal?: AbortSignal) {
    const generation = ++refreshGeneration;
    refreshing = true;

    try {
      const health = await readDaemonHealth(getDaemonBaseUrl(), signal);
      const [projectList, daemonSettings, nextRuntimeStatus, nextRuntimeOnboarding] =
        await Promise.all([
          listProjects({ signal }),
          readSettings({ signal }),
          readRuntimeStatus({ signal }),
          readRuntimeOnboarding({ signal }),
        ]);

      if (signal?.aborted || generation !== refreshGeneration) {
        return;
      }

      projects = projectList;
      settings = daemonSettings;
      runtimeStatus = nextRuntimeStatus;
      runtimeOnboarding = nextRuntimeOnboarding;
      runtimeProfiles = nextRuntimeStatus.providers.flatMap((provider) => provider.profiles);
      workspaceDetail = describeWorkspace(projectList);
      healthState = {
        kind: "connected",
        label: "Connected",
        detail: `Daemon ${health.version} using API v${health.api_version}`,
        health,
      };
    } catch (error) {
      if (signal?.aborted || generation !== refreshGeneration) {
        return;
      }

      projects = [];
      runtimeProfiles = [];
      runtimeStatus = undefined;
      runtimeOnboarding = undefined;
      settings = undefined;
      workspaceDetail = "Start the local daemon to load projects and settings.";
      healthState = {
        kind: "disconnected",
        label: "Disconnected",
        detail: error instanceof Error ? error.message : "Daemon health request failed",
      };
    } finally {
      if (generation === refreshGeneration) {
        refreshing = false;
      }
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
    get runtimeProfiles() {
      return runtimeProfiles;
    },
    get runtimeStatus() {
      return runtimeStatus;
    },
    get runtimePreference(): RuntimePreference | undefined {
      return runtimeStatus?.policy;
    },
    get runtimeOnboarding() {
      return runtimeOnboarding;
    },
    get refreshing() {
      return refreshing;
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
