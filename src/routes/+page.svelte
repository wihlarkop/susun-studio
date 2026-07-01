<script lang="ts">
  import * as Sidebar from "$lib/components/ui/sidebar/index.js";
  import AppSidebar from "$lib/components/app-sidebar.svelte";
  import TopBar from "$lib/components/top-bar.svelte";
  import HeroPanel from "$lib/components/hero-panel.svelte";
  import ProjectsTable from "$lib/components/projects-table.svelte";
  import ProjectDetail from "$lib/components/project-detail.svelte";
  import SettingsStrip from "$lib/components/settings-strip.svelte";
  import ImportProjectDialog from "$lib/components/import-project-dialog.svelte";
  import { createDaemonState } from "$lib/daemon/daemon-state.svelte";

  const daemonState = createDaemonState();
  let importDialogOpen = $state(false);
  let selectedProjectId = $state<string | null>(null);
  const selectedProject = $derived(
    daemonState.projects.find((project) => project.id === selectedProjectId) ?? null,
  );
</script>

<svelte:head>
  <title>Susun Studio</title>
</svelte:head>

<Sidebar.Provider>
  <AppSidebar healthState={daemonState.healthState} />
  <Sidebar.Inset>
    <div class="flex flex-col gap-6 p-6">
      <TopBar
        connected={daemonState.healthState.kind === "connected"}
        onImportClick={() => (importDialogOpen = true)}
      />
      <HeroPanel healthState={daemonState.healthState} />
      <ProjectsTable
        projects={daemonState.projects}
        workspaceDetail={daemonState.workspaceDetail}
        selectedId={selectedProjectId}
        onSelect={(project) => (selectedProjectId = project.id)}
      />
      <ProjectDetail project={selectedProject} />
      <SettingsStrip settings={daemonState.settings} />
    </div>
  </Sidebar.Inset>
</Sidebar.Provider>

<ImportProjectDialog
  bind:open={importDialogOpen}
  connected={daemonState.healthState.kind === "connected"}
  onImport={daemonState.importProject}
/>
