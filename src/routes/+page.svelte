<script lang="ts">
  import * as Sidebar from "$lib/components/ui/sidebar/index.js";
  import AppSidebar from "$lib/components/app-sidebar.svelte";
  import TopBar from "$lib/components/top-bar.svelte";
  import HeroPanel from "$lib/components/hero-panel.svelte";
  import ProjectsTable from "$lib/components/projects-table.svelte";
  import ProjectWorkspace from "$lib/components/project-workspace.svelte";
  import ActiveEngineStrip from "$lib/components/active-engine-strip.svelte";
  import JobsPage from "$lib/components/jobs-page.svelte";
  import RuntimePage from "$lib/components/runtime-page.svelte";
  import SettingsPage from "$lib/components/settings-page.svelte";
  import ImportProjectDialog from "$lib/components/import-project-dialog.svelte";
  import BetaOnboardingPanel from "$lib/components/beta-onboarding-panel.svelte";
  import { createDaemonState } from "$lib/daemon/daemon-state.svelte";
  import type { ImportProjectRequest, ImportProjectResponse } from "$lib/daemon/client";

  const daemonState = createDaemonState();
  let importDialogOpen = $state(false);
  let activeView = $state<"projects" | "jobs" | "runtime" | "settings">("projects");
  let selectedProjectId = $state<string | null>(null);
  const selectedProject = $derived(
    daemonState.projects.find((project) => project.id === selectedProjectId) ??
      daemonState.projects[0] ??
      null,
  );

  function selectProject(id: string) {
    selectedProjectId = id;
    void daemonState.setLastProjectId(id);
  }

  // Restore the last-viewed project once, the first time both settings and
  // the project list have loaded — not on every subsequent refresh, so it
  // never overrides a selection the user has since made.
  let restoredSelection = false;
  $effect(() => {
    if (restoredSelection || !daemonState.settings || daemonState.projects.length === 0) return;
    restoredSelection = true;
    const lastId = daemonState.settings.last_project_id;
    if (lastId && daemonState.projects.some((project) => project.id === lastId)) {
      selectedProjectId = lastId;
    }
  });

  function handleProjectRemoved(removedId: string) {
    if (selectedProjectId === removedId) {
      selectedProjectId = null;
    }
    void daemonState.refresh();
  }

  async function handleImport(request: ImportProjectRequest): Promise<ImportProjectResponse> {
    const response = await daemonState.importProject(request);
    if (response.project) {
      selectProject(response.project.id);
    }
    return response;
  }

  function handleShortcut(event: KeyboardEvent) {
    if ((event.ctrlKey || event.metaKey) && event.key.toLowerCase() === "i") {
      event.preventDefault();
      if (daemonState.healthState.kind === "connected") {
        importDialogOpen = true;
      }
    }
  }

  const viewTitle = $derived(
    activeView === "projects"
      ? "Projects"
      : activeView === "jobs"
        ? "Jobs"
        : activeView === "runtime"
          ? "Runtime"
          : "Settings",
  );
</script>

<svelte:head>
  <title>Susun Studio</title>
</svelte:head>

<svelte:window onkeydown={handleShortcut} />

<Sidebar.Provider>
  <AppSidebar
    healthState={daemonState.healthState}
    settings={daemonState.settings}
    {activeView}
    onNavigate={(view) => (activeView = view)}
  />
  <Sidebar.Inset>
    <div class="flex flex-col gap-6 p-6">
      <TopBar
        healthState={daemonState.healthState}
        title={viewTitle}
        onImportClick={() => (importDialogOpen = true)}
        onOpenSettings={() => (activeView = "settings")}
      />
      {#if activeView === "projects"}
        <HeroPanel healthState={daemonState.healthState} onRetry={daemonState.refresh} />
        <BetaOnboardingPanel
          healthState={daemonState.healthState}
          projectCount={daemonState.projects.length}
          runtimeProfiles={daemonState.runtimeProfiles}
          onImportClick={() => (importDialogOpen = true)}
          onRetry={daemonState.refresh}
          onSetupRuntime={() => (activeView = "runtime")}
        />
        <ActiveEngineStrip
          profiles={daemonState.runtimeProfiles}
          connected={daemonState.healthState.kind === "connected"}
          onManageRuntimes={() => (activeView = "runtime")}
          onChanged={() => daemonState.refresh()}
        />
        <ProjectsTable
          projects={daemonState.projects}
          profiles={daemonState.runtimeProfiles}
          workspaceDetail={daemonState.workspaceDetail}
          selectedId={selectedProject?.id ?? null}
          onSelect={(project) => selectProject(project.id)}
          onRemoved={handleProjectRemoved}
        />
        <ProjectWorkspace
          project={selectedProject}
          profiles={daemonState.runtimeProfiles}
          onEngineChanged={() => daemonState.refresh()}
        />
      {:else if activeView === "jobs"}
        <JobsPage projects={daemonState.projects} />
      {:else if activeView === "runtime"}
        <RuntimePage />
      {:else}
        <SettingsPage />
      {/if}
    </div>
  </Sidebar.Inset>
</Sidebar.Provider>

<ImportProjectDialog
  bind:open={importDialogOpen}
  connected={daemonState.healthState.kind === "connected"}
  runtimeProfiles={daemonState.runtimeProfiles}
  onImport={handleImport}
/>
