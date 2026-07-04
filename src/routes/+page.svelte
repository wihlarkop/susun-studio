<script lang="ts">
  import * as Sidebar from "$lib/components/ui/sidebar/index.js";
  import AppSidebar from "$lib/components/app-sidebar.svelte";
  import TopBar from "$lib/components/top-bar.svelte";
  import HeroPanel from "$lib/components/hero-panel.svelte";
  import ProjectsTable from "$lib/components/projects-table.svelte";
  import ProjectDetail from "$lib/components/project-detail.svelte";
  import PlanningPanel from "$lib/components/planning-panel.svelte";
  import JobPanel from "$lib/components/job-panel.svelte";
  import EngineStatusCard from "$lib/components/engine-status-card.svelte";
  import ImportProjectDialog from "$lib/components/import-project-dialog.svelte";
  import { createDaemonState } from "$lib/daemon/daemon-state.svelte";
  import type { ImportProjectRequest, ImportProjectResponse } from "$lib/daemon/client";

  const daemonState = createDaemonState();
  let importDialogOpen = $state(false);
  let selectedProjectId = $state<string | null>(null);
  const selectedProject = $derived(
    daemonState.projects.find((project) => project.id === selectedProjectId) ??
      daemonState.projects[0] ??
      null,
  );

  async function handleImport(request: ImportProjectRequest): Promise<ImportProjectResponse> {
    const response = await daemonState.importProject(request);
    if (response.project) {
      selectedProjectId = response.project.id;
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
</script>

<svelte:head>
  <title>Susun Studio</title>
</svelte:head>

<svelte:window onkeydown={handleShortcut} />

<Sidebar.Provider>
  <AppSidebar healthState={daemonState.healthState} settings={daemonState.settings} />
  <Sidebar.Inset>
    <div class="flex flex-col gap-6 p-6">
      <TopBar
        healthState={daemonState.healthState}
        onImportClick={() => (importDialogOpen = true)}
      />
      <HeroPanel healthState={daemonState.healthState} onRetry={daemonState.refresh} />
      <EngineStatusCard />
      <div class="grid items-start gap-6 xl:grid-cols-[3fr_2fr]">
        <ProjectsTable
          projects={daemonState.projects}
          workspaceDetail={daemonState.workspaceDetail}
          selectedId={selectedProject?.id ?? null}
          onSelect={(project) => (selectedProjectId = project.id)}
        />
        <ProjectDetail project={selectedProject} />
      </div>
      <PlanningPanel project={selectedProject} />
      <JobPanel project={selectedProject} />
    </div>
  </Sidebar.Inset>
</Sidebar.Provider>

<ImportProjectDialog
  bind:open={importDialogOpen}
  connected={daemonState.healthState.kind === "connected"}
  onImport={handleImport}
/>
