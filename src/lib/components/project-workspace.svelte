<script lang="ts">
  import * as Tabs from "$lib/components/ui/tabs/index.js";
  import { Button } from "$lib/components/ui/button/index.js";
  import ProjectDetail from "./project-detail.svelte";
  import PlanningPanel from "./planning-panel.svelte";
  import JobPanel from "./job-panel.svelte";
  import WatchPanel from "./watch-panel.svelte";
  import ServicesPanel from "./services-panel.svelte";
  import LogsViewer from "./logs-viewer.svelte";
  import EventsViewer from "./events-viewer.svelte";
  import type { StudioProject } from "$lib/daemon/client";

  let { project }: { project: StudioProject | null } = $props();

  let showLogs = $state(false);
  let logsAutoStartToken = $state(0);

  function handleJobFinished() {
    showLogs = true;
    logsAutoStartToken += 1;
  }
</script>

{#if project}
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
