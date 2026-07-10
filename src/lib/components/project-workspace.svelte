<script lang="ts">
  import * as Tabs from "$lib/components/ui/tabs/index.js";
  import { Button } from "$lib/components/ui/button/index.js";
  import { Badge } from "$lib/components/ui/badge/index.js";
  import ProjectDetail from "./project-detail.svelte";
  import PlanningPanel from "./planning-panel.svelte";
  import JobPanel from "./job-panel.svelte";
  import WatchPanel from "./watch-panel.svelte";
  import ServicesPanel from "./services-panel.svelte";
  import LogsViewer from "./logs-viewer.svelte";
  import EventsViewer from "./events-viewer.svelte";
  import { setProjectEngine, type RuntimeProfile, type StudioProject } from "$lib/daemon/client";
  import { ChevronDown } from "@lucide/svelte";

  let {
    project,
    profiles,
    onEngineChanged,
  }: {
    project: StudioProject | null;
    profiles: RuntimeProfile[];
    onEngineChanged: () => void;
  } = $props();

  let showLogs = $state(false);
  let logsAutoStartToken = $state(0);
  let bindingBusy = $state(false);

  const boundProfile = $derived(
    project?.runtime_profile_id
      ? (profiles.find((profile) => profile.id === project.runtime_profile_id) ?? null)
      : null,
  );
  const bindingBroken = $derived(
    project?.runtime_profile_id != null &&
      (boundProfile === null || boundProfile.connection.state !== "summarized"),
  );

  function handleJobFinished() {
    showLogs = true;
    logsAutoStartToken += 1;
  }

  async function changeBinding(event: Event) {
    if (!project) return;
    const value = (event.currentTarget as HTMLSelectElement).value;
    bindingBusy = true;
    try {
      await setProjectEngine(project.id, value || null);
      onEngineChanged();
    } finally {
      bindingBusy = false;
    }
  }
</script>

{#if project}
  <div class="flex flex-wrap items-center gap-2 text-sm">
    <span class="text-muted-foreground">Engine:</span>
    <div class="relative">
      <select
        class="h-8 appearance-none rounded-md border bg-background bg-none pr-8 pl-3 text-sm"
        disabled={bindingBusy}
        value={project.runtime_profile_id ?? ""}
        onchange={changeBinding}
        aria-label="Project engine binding"
      >
        <option value="">Use active engine</option>
        {#each profiles as profile (profile.id)}
          <option value={profile.id}>{profile.display_name}</option>
        {/each}
      </select>
      <ChevronDown
        class="pointer-events-none absolute top-1/2 right-2 size-4 -translate-y-1/2 text-muted-foreground"
      />
    </div>
    {#if bindingBroken}
      <Badge variant="destructive" class="text-xs">
        Bound engine unavailable, actions use the active engine
      </Badge>
    {/if}
  </div>

  <Tabs.Root value="overview" class="w-full">
    <Tabs.List>
      <Tabs.Trigger value="overview">Overview</Tabs.Trigger>
      <Tabs.Trigger value="services">Services</Tabs.Trigger>
      <Tabs.Trigger value="events">Events</Tabs.Trigger>
    </Tabs.List>
    <Tabs.Content value="overview" class="flex flex-col gap-6 pt-4">
      <ProjectDetail {project} />
      <PlanningPanel {project} />
    </Tabs.Content>
    <Tabs.Content value="services" class="flex flex-col gap-4 pt-4">
      <JobPanel {project} onJobFinished={handleJobFinished} />
      <Button size="sm" variant="ghost" class="self-start" onclick={() => (showLogs = !showLogs)}>
        {showLogs ? "Hide logs" : "Show logs"}
      </Button>
      {#if showLogs}
        <LogsViewer {project} autoStartToken={logsAutoStartToken} />
      {/if}
      <WatchPanel {project} />
      <ServicesPanel {project} />
    </Tabs.Content>
    <Tabs.Content value="events" class="pt-4">
      <EventsViewer {project} />
    </Tabs.Content>
  </Tabs.Root>
{/if}
