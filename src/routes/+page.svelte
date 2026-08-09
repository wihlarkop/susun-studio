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
  import ArtifactsPage from "$lib/components/artifacts-page.svelte";
  import SettingsPage from "$lib/components/settings-page.svelte";
  import ImportProjectDialog from "$lib/components/import-project-dialog.svelte";
  import BetaOnboardingPanel from "$lib/components/beta-onboarding-panel.svelte";
  import RuntimeOnboardingDialog from "$lib/components/runtime-onboarding-dialog.svelte";
  import { createDaemonState } from "$lib/daemon/daemon-state.svelte";
  import {
    reopenRuntimeOnboarding,
    type ImportProjectRequest,
    type ImportProjectResponse,
  } from "$lib/daemon/client";
  import { resolveOnboardingView } from "$lib/runtime/onboarding-state";

  const daemonState = createDaemonState();
  let importDialogOpen = $state(false);
  let activeView = $state<"projects" | "jobs" | "runtime" | "artifacts" | "settings">("projects");
  let selectedProjectId = $state<string | null>(null);
  let runtimeOnboardingOpen = $state(false);
  let runtimeOnboardingReopened = $state(false);
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
          : activeView === "artifacts"
            ? "Artifacts"
            : "Settings",
  );
  const onboardingView = $derived(
    resolveOnboardingView({
      connected: daemonState.healthState.kind === "connected",
      onboarding: daemonState.runtimeOnboarding,
      binding: daemonState.runtimePreference?.binding,
    }),
  );

  $effect(() => {
    if (runtimeOnboardingReopened) return;
    runtimeOnboardingOpen = onboardingView.kind === "chooser";
  });

  async function openRuntimeSetup() {
    if (daemonState.healthState.kind !== "connected" || !daemonState.runtimeOnboarding) return;
    try {
      if (daemonState.runtimeOnboarding.state === "dismissed") {
        await reopenRuntimeOnboarding();
        await daemonState.refresh();
      }
      runtimeOnboardingReopened = true;
      runtimeOnboardingOpen = true;
    } catch {
      // The existing Runtime recovery UI remains available if reopening cannot
      // reach the local daemon; no policy or completion state is changed.
    }
  }

  function finishRuntimeSetup() {
    runtimeOnboardingOpen = false;
    runtimeOnboardingReopened = false;
  }
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
          runtimePreference={daemonState.runtimePreference}
          onImportClick={() => (importDialogOpen = true)}
          onRetry={daemonState.refresh}
          onManageRuntime={() => (activeView = "runtime")}
        />
        <ActiveEngineStrip
          profiles={daemonState.runtimeProfiles}
          runtimePreference={daemonState.runtimePreference}
          connected={daemonState.healthState.kind === "connected"}
          onManageRuntimes={() => (activeView = "runtime")}
          onChanged={() => daemonState.refresh()}
        />
        <ProjectsTable
          projects={daemonState.projects}
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
        <RuntimePage
          runtimeStatus={daemonState.runtimeStatus}
          refreshing={daemonState.refreshing}
          onRecheck={daemonState.refresh}
          onChooseRuntime={openRuntimeSetup}
        />
      {:else if activeView === "artifacts"}
        <ArtifactsPage
          profiles={daemonState.runtimeProfiles}
          runtimePreference={daemonState.runtimePreference}
          connected={daemonState.healthState.kind === "connected"}
          projects={daemonState.projects}
        />
      {:else}
        <SettingsPage
          onboarding={daemonState.runtimeOnboarding}
          onRunRuntimeSetup={openRuntimeSetup}
        />
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

{#if runtimeOnboardingOpen && daemonState.runtimeStatus && daemonState.runtimeOnboarding}
  <RuntimeOnboardingDialog
    bind:open={runtimeOnboardingOpen}
    status={daemonState.runtimeStatus}
    onboarding={daemonState.runtimeOnboarding}
    reopened={runtimeOnboardingReopened}
    onchanged={daemonState.refresh}
    onfinished={finishRuntimeSetup}
  />
{/if}
