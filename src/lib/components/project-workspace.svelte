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
  import { type RuntimeProfile, type StudioProject } from "$lib/daemon/client";
  import RuntimeIdentity from "./runtime-identity.svelte";
  import { presentRuntimeBinding } from "$lib/runtime/presentation";
  import { presentProjectBinding } from "$lib/runtime/project-binding-state";
  import { ChevronDown } from "@lucide/svelte";
  import type { RuntimeContextTarget } from "$lib/runtime/context-change";

  let {
    project,
    profiles,
    onContextChange,
  }: {
    project: StudioProject | null;
    profiles: RuntimeProfile[];
    onContextChange: (target: RuntimeContextTarget) => void;
  } = $props();

  let showLogs = $state(false);
  let logsAutoStartToken = $state(0);
  let bindingSelect = $state<HTMLSelectElement | null>(null);

  const bindingView = $derived(
    project ? presentProjectBinding(project.runtime_binding) : null,
  );
  const bindingBlocked = $derived(bindingView?.blocked ?? false);
  const selectableProfiles = $derived(
    profiles.filter(
      (profile) => profile.availability_state === "available" && profile.management.can_select,
    ),
  );
  const builtInProfiles = $derived(
    selectableProfiles.filter((profile) => profile.runtime_class === "built_in"),
  );
  const externalProfiles = $derived(
    selectableProfiles.filter((profile) => profile.runtime_class !== "built_in"),
  );

  function handleJobFinished() {
    showLogs = true;
    logsAutoStartToken += 1;
  }

  function changeBinding(event: Event) {
    if (!project) return;
    const value = (event.currentTarget as HTMLSelectElement).value;
    onContextChange({ kind: "project", projectId: project.id, profileId: value || null });
  }
</script>

{#if project && bindingView}
  <div class="flex flex-col gap-3 rounded-md border p-3 text-sm">
    <div class="flex flex-wrap items-center gap-2">
      <span class="text-muted-foreground">Runtime:</span>
      <RuntimeIdentity presentation={presentRuntimeBinding(project.runtime_binding)} compact />
    </div>
    <div class="flex flex-wrap items-center gap-2">
      <span class="text-muted-foreground">{bindingView.selectorLabel}:</span>
      <div class="relative">
      <select
        bind:this={bindingSelect}
        class="h-8 appearance-none rounded-md border bg-background bg-none pr-8 pl-3 text-sm"
        value={project.runtime_profile_id ?? ""}
        onchange={changeBinding}
        aria-label="Project runtime binding"
      >
        <option value="">Use preferred runtime</option>
        {#if builtInProfiles.length > 0}
          <optgroup label="Susun Runtime">
            {#each builtInProfiles as profile (profile.id)}
              <option value={profile.id}>{profile.display_name}</option>
            {/each}
          </optgroup>
        {/if}
        {#if externalProfiles.length > 0}
          <optgroup label="Existing runtimes">
            {#each externalProfiles as profile (profile.id)}
              <option value={profile.id}>{profile.display_name}</option>
            {/each}
          </optgroup>
        {/if}
      </select>
      <ChevronDown
        class="pointer-events-none absolute top-1/2 right-2 size-4 -translate-y-1/2 text-muted-foreground"
      />
      </div>
      {#if bindingView.canClear}
        <Button size="sm" variant="ghost" onclick={() => bindingSelect?.focus()}>
          Change runtime
        </Button>
      {/if}
    </div>
    {#if bindingBlocked}
      <div class="flex flex-wrap items-center gap-2 text-destructive">
        <Badge variant="destructive">Actions blocked</Badge>
        <span>{bindingView.blockedReason}</span>
      </div>
    {/if}
  </div>

  <Tabs.Root value="overview" class="w-full">
    <Tabs.List>
      <Tabs.Trigger value="overview">Overview</Tabs.Trigger>
      <Tabs.Trigger value="services" disabled={bindingBlocked}>Services</Tabs.Trigger>
      <Tabs.Trigger value="events">Events</Tabs.Trigger>
    </Tabs.List>
    <Tabs.Content value="overview" class="flex flex-col gap-6 pt-4">
      <ProjectDetail {project} />
      {#if bindingBlocked}
        <p class="rounded-md border border-destructive/30 bg-destructive/5 p-3 text-sm text-destructive">
          Runtime-backed planning is unavailable until you change or recover this runtime binding.
        </p>
      {:else}
        <PlanningPanel {project} />
      {/if}
    </Tabs.Content>
    <Tabs.Content value="services" class="flex flex-col gap-4 pt-4">
      {#if bindingBlocked}
        <p class="rounded-md border border-destructive/30 bg-destructive/5 p-3 text-sm text-destructive">
          Service actions are blocked until you change or recover this runtime binding.
        </p>
      {:else}
        <JobPanel {project} onJobFinished={handleJobFinished} />
        <Button size="sm" variant="ghost" class="self-start" onclick={() => (showLogs = !showLogs)}>
          {showLogs ? "Hide logs" : "Show logs"}
        </Button>
        {#if showLogs}
          <LogsViewer {project} autoStartToken={logsAutoStartToken} />
        {/if}
        <WatchPanel {project} />
        <ServicesPanel {project} />
      {/if}
    </Tabs.Content>
    <Tabs.Content value="events" class="pt-4">
      <EventsViewer {project} />
    </Tabs.Content>
  </Tabs.Root>
{/if}
