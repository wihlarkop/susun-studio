<script lang="ts">
  import * as Dialog from "$lib/components/ui/dialog/index.js";
  import { Badge } from "$lib/components/ui/badge/index.js";
  import { Button } from "$lib/components/ui/button/index.js";
  import {
    commitRuntimeMigration,
    commitRuntimeMigrationRollback,
    listProjects,
    prepareRuntimeMigrationRollback,
    previewRuntimeMigration,
    type RuntimeMigrationPreview,
    type RuntimeMigrationRequest,
    type RuntimeMigrationResult,
    type RuntimeMigrationRollbackResult,
    type RuntimeProfile,
    type StudioProject,
  } from "$lib/daemon/client";
  import { ArrowRight, RefreshCw } from "@lucide/svelte";

  let {
    profiles,
    open = $bindable(false),
    oncompleted,
  }: {
    profiles: RuntimeProfile[];
    open?: boolean;
    oncompleted?: () => void;
  } = $props();

  let projects = $state<StudioProject[]>([]);
  let sourceId = $state("");
  let targetId = $state("");
  let selectedProjectIds = $state<Set<string>>(new Set());
  let preview = $state<RuntimeMigrationPreview | null>(null);
  let result = $state<RuntimeMigrationResult | null>(null);
  let rollbackResult = $state<RuntimeMigrationRollbackResult | null>(null);
  let busy = $state(false);
  let errorMessage = $state<string | null>(null);

  const sourceProjects = $derived(projects.filter((project) => project.runtime_profile_id === sourceId));
  const request = $derived<RuntimeMigrationRequest>({
    source_profile_id: sourceId,
    target_profile_id: targetId,
    project_ids: [...selectedProjectIds],
  });

  $effect(() => {
    if (!open) return;
    void loadProjects();
  });

  async function loadProjects() {
    try {
      projects = await listProjects();
      errorMessage = null;
    } catch (error) {
      errorMessage = error instanceof Error ? error.message : String(error);
    }
  }

  function chooseSource(value: string) {
    sourceId = value;
    selectedProjectIds = new Set();
    preview = null;
    result = null;
    rollbackResult = null;
  }

  function toggleProject(id: string) {
    const next = new Set(selectedProjectIds);
    if (next.has(id)) next.delete(id);
    else next.add(id);
    selectedProjectIds = next;
    preview = null;
  }

  async function buildPreview() {
    busy = true;
    errorMessage = null;
    result = null;
    try {
      preview = await previewRuntimeMigration(request);
    } catch (error) {
      errorMessage = error instanceof Error ? error.message : String(error);
    } finally {
      busy = false;
    }
  }

  async function migrate() {
    if (!preview?.can_migrate || !preview.plan_id) return;
    busy = true;
    errorMessage = null;
    try {
      result = await commitRuntimeMigration(preview.plan_id);
      if (result.status === "completed") {
        preview = null;
        await loadProjects();
        oncompleted?.();
      }
    } catch (error) {
      errorMessage = error instanceof Error ? error.message : String(error);
    } finally {
      busy = false;
    }
  }

  async function rollback() {
    if (!result?.rollback_available) return;
    busy = true;
    errorMessage = null;
    try {
      const rollbackPlan = await prepareRuntimeMigrationRollback(result.migration_id);
      if (!rollbackPlan.restorable || !rollbackPlan.plan_id) {
        rollbackResult = {
          migration_id: result.migration_id,
          status: "unavailable",
          restored_project_count: 0,
        };
        return;
      }
      rollbackResult = await commitRuntimeMigrationRollback(rollbackPlan.plan_id);
      if (rollbackResult.status === "rolled_back") {
        await loadProjects();
        oncompleted?.();
      }
    } catch (error) {
      errorMessage = error instanceof Error ? error.message : String(error);
    } finally {
      busy = false;
    }
  }
</script>

<Dialog.Root bind:open>
  <Dialog.Content class="max-h-[85vh] overflow-y-auto sm:max-w-2xl">
    <Dialog.Header>
      <Dialog.Title>Migrate project runtime</Dialog.Title>
      <Dialog.Description>
        Move selected project bindings. Runtime ownership, volume data, and credentials stay unchanged.
      </Dialog.Description>
    </Dialog.Header>

    <div class="grid gap-4">
      <div class="grid gap-3 sm:grid-cols-[1fr_auto_1fr] sm:items-end">
        <label class="grid gap-1 text-sm font-medium">
          Source
          <select class="h-9 min-w-0 rounded-md border bg-background px-3 text-sm" value={sourceId} onchange={(event) => chooseSource(event.currentTarget.value)}>
            <option value="">Select runtime</option>
            {#each profiles as profile (profile.id)}
              <option value={profile.id}>{profile.display_name}</option>
            {/each}
          </select>
        </label>
        <ArrowRight class="mb-2 hidden size-4 text-muted-foreground sm:block" />
        <label class="grid gap-1 text-sm font-medium">
          Target
          <select class="h-9 min-w-0 rounded-md border bg-background px-3 text-sm" bind:value={targetId} onchange={() => (preview = null)}>
            <option value="">Select runtime</option>
            {#each profiles.filter((profile) => profile.id !== sourceId) as profile (profile.id)}
              <option value={profile.id}>{profile.display_name}</option>
            {/each}
          </select>
        </label>
      </div>

      <div>
        <div class="mb-2 flex items-center justify-between gap-2">
          <span class="text-sm font-medium">Projects</span>
          <Badge variant="outline">{selectedProjectIds.size} selected</Badge>
        </div>
        {#if !sourceId}
          <p class="rounded-md border p-3 text-sm text-muted-foreground">Select a source runtime.</p>
        {:else if sourceProjects.length === 0}
          <p class="rounded-md border p-3 text-sm text-muted-foreground">No projects are bound to this runtime.</p>
        {:else}
          <div class="max-h-44 divide-y overflow-y-auto rounded-md border">
            {#each sourceProjects as project (project.id)}
              <label class="flex items-center gap-3 p-3 text-sm">
                <input type="checkbox" checked={selectedProjectIds.has(project.id)} onchange={() => toggleProject(project.id)} />
                <span class="min-w-0 truncate">{project.name}</span>
              </label>
            {/each}
          </div>
        {/if}
      </div>

      {#if preview}
        <div class="grid gap-3 border-y py-3 text-sm">
          <div class="flex flex-wrap items-center gap-2">
            <Badge variant={preview.can_migrate ? "default" : "destructive"}>
              {preview.can_migrate ? "Ready" : "Blocked"}
            </Badge>
            <span>{preview.projects.length} project bindings</span>
            <Badge variant="outline">Rollback metadata retained</Badge>
          </div>
          {#if preview.blockers.length > 0}
            <ul class="grid gap-1 text-destructive">
              {#each preview.blockers as blocker}<li>{blocker}</li>{/each}
            </ul>
          {/if}
          <div class="grid gap-1 text-muted-foreground">
            {#each preview.artifact_policy as item (item.category)}
              <div class="flex flex-wrap justify-between gap-2">
                <span>{item.category.replaceAll("_", " ")}</span>
                <span>{item.disposition.replaceAll("_", " ")}</span>
              </div>
            {/each}
          </div>
        </div>
      {/if}

      {#if result}
        <div class="rounded-md border p-3 text-sm">
          <div class="flex items-center gap-2">
            <Badge variant={result.status === "completed" ? "default" : "destructive"}>{result.status}</Badge>
            <span>{result.project_count} projects migrated</span>
          </div>
          {#if result.failures.length > 0}<p class="mt-2 text-destructive">{result.failures.join(" ")}</p>{/if}
          {#if result.status === "completed" && !rollbackResult}
            <Button class="mt-3" size="sm" variant="outline" disabled={busy} onclick={rollback}>
              Roll back bindings
            </Button>
          {/if}
          {#if rollbackResult}
            <p class="mt-2 text-muted-foreground">
              {rollbackResult.status === "rolled_back"
                ? `${rollbackResult.restored_project_count} project bindings restored.`
                : "Rollback is no longer available because one or more bindings changed."}
            </p>
          {/if}
        </div>
      {/if}

      {#if errorMessage}<p class="text-sm text-destructive">{errorMessage}</p>{/if}
    </div>

    <Dialog.Footer>
      <Button variant="outline" onclick={() => (open = false)}>Close</Button>
      <Button variant="outline" disabled={busy || !sourceId || !targetId || selectedProjectIds.size === 0} onclick={buildPreview}>
        <RefreshCw /> Preview
      </Button>
      <Button disabled={busy || !preview?.can_migrate} onclick={migrate}>Migrate projects</Button>
    </Dialog.Footer>
  </Dialog.Content>
</Dialog.Root>
