<script lang="ts">
  import { onDestroy } from "svelte";
  import * as Dialog from "$lib/components/ui/dialog/index.js";
  import { Badge } from "$lib/components/ui/badge/index.js";
  import { Button } from "$lib/components/ui/button/index.js";
  import {
    commitRuntimeMigration,
    commitRuntimeMigrationRollback,
    prepareRuntimeMigrationRollback,
    previewRuntimeMigration,
    readRuntimeMigrationInventory,
    type RuntimeMigrationInventoryProfile,
    type RuntimeProfile,
  } from "$lib/daemon/client";
  import {
    acceptMigrationCommit,
    acceptMigrationInventory,
    acceptMigrationPreview,
    acceptRollbackCommit,
    acceptRollbackPreview,
    acknowledgeRollbackConfirmation,
    beginMigrationCommit,
    beginMigrationInventory,
    beginMigrationPreview,
    beginHistoryRollback,
    beginRollbackCommit,
    beginRollbackPreview,
    boundedMigrationError,
    boundedRollbackBlocker,
    canAcceptMigrationResponse,
    canCommitMigration,
    cancelMigration,
    chooseMigrationWorkflow,
    createMigrationState,
    failMigrationRequest,
    invalidateMigrationForRuntimeRefresh,
    migrationConfirmationDetails,
    migrationEligibleSources,
    migrationEligibleTargets,
    migrationExcludedCategories,
    migrationProjectsForSource,
    migrationRequest,
    resetMigrationDialog,
    updateMigrationSelection,
  } from "$lib/runtime/migration-state";
  import { ArrowRight, RefreshCw, RotateCcw, Server, Waypoints } from "@lucide/svelte";

  let {
    profiles,
    open = $bindable(false),
    oncompleted,
    onconnect,
    rollbackMigrationId = null,
    onhistoryrollbackhandled,
  }: {
    profiles: RuntimeProfile[];
    open?: boolean;
    oncompleted: () => void | Promise<void>;
    onconnect: (profileId: string) => void;
    rollbackMigrationId?: string | null;
    onhistoryrollbackhandled?: () => void;
  } = $props();

  let migrationState = $state(createMigrationState());
  let activeController: AbortController | null = null;
  let connectProfileId = $state("");
  let wasOpen = false;
  let previousProfileRevision = "";
  let hasProfileRevision = false;
  let lastHistoryRollbackId: string | null = null;

  const profileRevision = $derived(
    profiles
      .map((profile) => `${profile.id}:${profile.observation_revision}`)
      .sort()
      .join("|"),
  );
  const inventory = $derived(migrationState.inventory);
  const sourceProfiles = $derived(inventory ? migrationEligibleSources(inventory) : []);
  const targetProfiles = $derived(
    inventory ? migrationEligibleTargets(inventory, migrationState.selection.sourceId) : [],
  );
  const sourceProjects = $derived(
    inventory ? migrationProjectsForSource(inventory, migrationState.selection.sourceId) : [],
  );
  const externalProfiles = $derived(
    inventory
      ? inventory.profiles.filter(
          (profile) => profile.runtime_class !== "built_in" && profile.selectable,
        )
      : [],
  );
  const selectedExternalProfile = $derived(
    externalProfiles.find((profile) => profile.profile_id === connectProfileId) ?? null,
  );
  const confirmation = $derived(migrationState.preview ? migrationConfirmationDetails(migrationState.preview) : null);

  $effect(() => {
    if (open === wasOpen) return;
    wasOpen = open;
    abortActiveRequest();
    migrationState = open ? resetMigrationDialog(migrationState) : cancelMigration(migrationState);
    connectProfileId = "";
    if (!open) lastHistoryRollbackId = null;
  });

  $effect(() => {
    const migrationId = rollbackMigrationId;
    if (!open || !migrationId || migrationId === lastHistoryRollbackId) return;
    lastHistoryRollbackId = migrationId;
    onhistoryrollbackhandled?.();
    abortActiveRequest();
    migrationState = beginHistoryRollback(migrationState, migrationId);
    void prepareRollback();
  });

  // Shared daemon refreshes invalidate a preview, but do not create component polling.
  $effect(() => {
    const revision = profileRevision;
    if (!hasProfileRevision) {
      hasProfileRevision = true;
      previousProfileRevision = revision;
      return;
    }
    if (revision === previousProfileRevision) return;
    previousProfileRevision = revision;
    if (!open || migrationState.workflow === null) return;
    abortActiveRequest();
    migrationState = invalidateMigrationForRuntimeRefresh(migrationState);
  });

  onDestroy(abortActiveRequest);

  function setOpen(next: boolean) {
    open = next;
  }

  function abortActiveRequest() {
    activeController?.abort();
    activeController = null;
  }

  function requestIsCurrent(controller: AbortController, generation: number): boolean {
    return (
      activeController === controller &&
      !controller.signal.aborted &&
      canAcceptMigrationResponse(migrationState, generation, migrationState.generation)
    );
  }

  function beginWorkflow(workflow: "connect" | "migrate") {
    abortActiveRequest();
    migrationState = chooseMigrationWorkflow(migrationState, workflow);
    void loadInventory(workflow);
  }

  async function loadInventory(workflow: "connect" | "migrate") {
    abortActiveRequest();
    migrationState = beginMigrationInventory(migrationState, workflow);
    const generation = migrationState.generation;
    const controller = new AbortController();
    activeController = controller;
    try {
      const nextInventory = await readRuntimeMigrationInventory({ signal: controller.signal });
      if (requestIsCurrent(controller, generation)) {
        migrationState = acceptMigrationInventory(migrationState, generation, nextInventory);
      }
    } catch (error) {
      if (requestIsCurrent(controller, generation)) {
        migrationState = failMigrationRequest(migrationState, generation, error);
      }
    } finally {
      if (activeController === controller) activeController = null;
    }
  }

  function backToChoices() {
    abortActiveRequest();
    migrationState = resetMigrationDialog(migrationState);
    connectProfileId = "";
  }

  function chooseSource(sourceId: string) {
    abortActiveRequest();
    migrationState = updateMigrationSelection(migrationState, {
      sourceId: sourceId || null,
      targetId: null,
      projectIds: [],
    });
  }

  function chooseTarget(targetId: string) {
    abortActiveRequest();
    migrationState = updateMigrationSelection(migrationState, { targetId: targetId || null });
  }

  function toggleProject(projectId: string) {
    abortActiveRequest();
    const selected = new Set(migrationState.selection.projectIds);
    if (selected.has(projectId)) selected.delete(projectId);
    else selected.add(projectId);
    migrationState = updateMigrationSelection(migrationState, { projectIds: [...selected] });
  }

  async function previewMigration() {
    const request = migrationRequest(migrationState);
    if (!request) return;
    abortActiveRequest();
    migrationState = beginMigrationPreview(migrationState, Date.now());
    if (migrationState.phase !== "previewing") return;
    const generation = migrationState.generation;
    const controller = new AbortController();
    activeController = controller;
    try {
      const preview = await previewRuntimeMigration(request, { signal: controller.signal });
      if (requestIsCurrent(controller, generation)) {
        migrationState = acceptMigrationPreview(migrationState, generation, preview, Date.now());
      }
    } catch (error) {
      if (requestIsCurrent(controller, generation)) {
        migrationState = failMigrationRequest(migrationState, generation, error);
      }
    } finally {
      if (activeController === controller) activeController = null;
    }
  }

  async function commitMigration() {
    const planId = migrationState.preview?.plan_id;
    if (!planId) return;
    abortActiveRequest();
    migrationState = beginMigrationCommit(migrationState, Date.now());
    if (migrationState.phase !== "committing") return;
    const generation = migrationState.generation;
    const controller = new AbortController();
    activeController = controller;
    try {
      const result = await commitRuntimeMigration(planId, { signal: controller.signal });
      if (requestIsCurrent(controller, generation)) {
        migrationState = acceptMigrationCommit(migrationState, generation, result);
        if (migrationState.phase === "committed") await oncompleted();
      }
    } catch (error) {
      if (requestIsCurrent(controller, generation)) {
        migrationState = failMigrationRequest(migrationState, generation, error);
      }
    } finally {
      if (activeController === controller) activeController = null;
    }
  }

  async function prepareRollback() {
    const migrationId = migrationState.result?.migration_id;
    if (!migrationId) return;
    abortActiveRequest();
    migrationState = beginRollbackPreview(migrationState);
    if (migrationState.phase !== "rollback_previewing") return;
    const generation = migrationState.generation;
    const controller = new AbortController();
    activeController = controller;
    try {
      const preview = await prepareRuntimeMigrationRollback(migrationId, {
        signal: controller.signal,
      });
      if (requestIsCurrent(controller, generation)) {
        migrationState = acceptRollbackPreview(migrationState, generation, preview, Date.now());
      }
    } catch (error) {
      if (requestIsCurrent(controller, generation)) {
        migrationState = failMigrationRequest(migrationState, generation, error);
      }
    } finally {
      if (activeController === controller) activeController = null;
    }
  }

  async function confirmRollback() {
    const confirmed = acknowledgeRollbackConfirmation(migrationState);
    const planId = confirmed.rollbackPreview?.plan_id;
    if (!planId) return;
    abortActiveRequest();
    migrationState = beginRollbackCommit(confirmed, Date.now());
    if (migrationState.phase !== "rollback_committing") return;
    const generation = migrationState.generation;
    const controller = new AbortController();
    activeController = controller;
    try {
      const result = await commitRuntimeMigrationRollback(planId, { signal: controller.signal });
      if (requestIsCurrent(controller, generation)) {
        migrationState = acceptRollbackCommit(migrationState, generation, result);
        if (migrationState.phase === "rolled_back") await oncompleted();
      }
    } catch (error) {
      if (requestIsCurrent(controller, generation)) {
        migrationState = failMigrationRequest(migrationState, generation, error);
      }
    } finally {
      if (activeController === controller) activeController = null;
    }
  }

  function openContextChange() {
    const profile = selectedExternalProfile;
    if (!profile || profile.runtime_class === "built_in" || !profile.selectable) return;
    open = false;
    onconnect(profile.profile_id);
  }

  function profileLabel(profile: RuntimeMigrationInventoryProfile): string {
    return `${profile.display_name} (${profile.runtime_class === "built_in" ? "Built-in" : "External"})`;
  }
</script>

<Dialog.Root bind:open={() => open, setOpen}>
  <Dialog.Content class="max-h-[calc(100vh-2rem)] overflow-y-auto sm:max-w-2xl" showCloseButton={false}>
    <Dialog.Header>
      <Dialog.Title>Runtime migration and recovery</Dialog.Title>
      <Dialog.Description>
        Choose a default runtime or move only explicit project bindings. Neither option copies engine data.
      </Dialog.Description>
    </Dialog.Header>

    <div class="grid gap-4 text-sm">
      {#if migrationState.phase === "idle" && migrationState.workflow === null}
        <div class="grid gap-3 sm:grid-cols-2">
          <button
            type="button"
            class="grid gap-2 rounded-md border p-4 text-left transition-colors hover:bg-muted focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
            onclick={() => beginWorkflow("connect")}
          >
            <Server class="size-5" />
            <span class="font-medium">Use an external runtime</span>
            <span class="text-muted-foreground">Change the global default. Explicit project pins stay unchanged.</span>
          </button>
          <button
            type="button"
            class="grid gap-2 rounded-md border p-4 text-left transition-colors hover:bg-muted focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
            onclick={() => beginWorkflow("migrate")}
          >
            <Waypoints class="size-5" />
            <span class="font-medium">Move explicit project bindings</span>
            <span class="text-muted-foreground">Prepare a guarded metadata migration with rollback limits.</span>
          </button>
        </div>
      {:else if migrationState.phase === "loading_inventory"}
        <p class="text-muted-foreground">Loading runtime migration inventory...</p>
      {:else if migrationState.workflow === "connect" && inventory}
        <div class="grid gap-3">
          <p class="text-muted-foreground">
            This opens the shared runtime-context preview. It changes only the global default and never creates project pins or a migration plan.
          </p>
          <label class="grid gap-1 font-medium">
            External runtime
            <select
              class="h-9 rounded-md border bg-background px-3 text-sm"
              bind:value={connectProfileId}
            >
              <option value="">Select an external runtime</option>
              {#each externalProfiles as profile (profile.profile_id)}
                <option value={profile.profile_id}>{profileLabel(profile)}</option>
              {/each}
            </select>
          </label>
          {#if externalProfiles.length === 0}
            <p class="rounded-md border p-3 text-muted-foreground">No selectable external runtime is available.</p>
          {/if}
        </div>
      {:else if migrationState.workflow === "migrate"}
        <div class="grid gap-4">
          {#if inventory}
          <div class="grid gap-3 sm:grid-cols-[1fr_auto_1fr] sm:items-end">
            <label class="grid gap-1 font-medium">
              Source runtime
              <select
                class="h-9 min-w-0 rounded-md border bg-background px-3 text-sm"
                value={migrationState.selection.sourceId ?? ""}
                onchange={(event) => chooseSource(event.currentTarget.value)}
              >
                <option value="">Select the pinned source</option>
                {#each sourceProfiles as profile (profile.profile_id)}
                  <option value={profile.profile_id}>{profileLabel(profile)}</option>
                {/each}
              </select>
            </label>
            <ArrowRight class="mb-2 hidden size-4 text-muted-foreground sm:block" />
            <label class="grid gap-1 font-medium">
              Target runtime
              <select
                class="h-9 min-w-0 rounded-md border bg-background px-3 text-sm"
                value={migrationState.selection.targetId ?? ""}
                onchange={(event) => chooseTarget(event.currentTarget.value)}
              >
                <option value="">Select a live target</option>
                {#each targetProfiles as profile (profile.profile_id)}
                  <option value={profile.profile_id}>{profileLabel(profile)}</option>
                {/each}
              </select>
            </label>
          </div>

          <div class="grid gap-2">
            <div class="flex flex-wrap items-center justify-between gap-2">
              <span class="font-medium">Explicitly pinned projects</span>
              <Badge variant="outline">{migrationState.selection.projectIds.length} selected</Badge>
            </div>
            {#if !migrationState.selection.sourceId}
              <p class="rounded-md border p-3 text-muted-foreground">Select a source runtime to review its explicit project pins.</p>
            {:else if sourceProjects.length === 0}
              <p class="rounded-md border p-3 text-muted-foreground">No explicit project pins reference this runtime.</p>
            {:else}
              <div class="max-h-44 divide-y overflow-y-auto rounded-md border">
                {#each sourceProjects as project (project.project_id)}
                  <label class="flex items-center gap-3 p-3">
                    <input
                      type="checkbox"
                      checked={migrationState.selection.projectIds.includes(project.project_id)}
                      onchange={() => toggleProject(project.project_id)}
                    />
                    <span>Project {project.project_id}</span>
                  </label>
                {/each}
              </div>
            {/if}
          </div>

          {#if migrationState.phase === "previewing"}
            <p class="text-muted-foreground">Preparing trusted migration preview...</p>
          {:else if confirmation && migrationState.preview}
            <div class="grid gap-3 rounded-md border p-3">
              <div class="flex flex-wrap items-center gap-2">
                <Badge variant={migrationState.preview.can_migrate ? "default" : "destructive"}>
                  {migrationState.preview.can_migrate ? "Ready for confirmation" : "Blocked"}
                </Badge>
                <span>{confirmation.projectCount} explicit project binding(s)</span>
              </div>
              <div class="grid gap-1 text-muted-foreground">
                <span>Source: {confirmation.source.display_name}</span>
                <span>Target: {confirmation.target.display_name}</span>
                <span>
                  Rollback metadata: {confirmation.rollbackAvailable ? "retained" : "not available"}
                  {confirmation.expiresInSeconds ? `; preview expires in ${confirmation.expiresInSeconds}s` : ""}
                </span>
              </div>
              {#if migrationState.preview.blockers.length > 0}
                <ul class="grid gap-1 text-destructive">
                  {#each migrationState.preview.blockers as blocker}<li>{blocker}</li>{/each}
                </ul>
              {/if}
              <div class="grid gap-1 text-muted-foreground">
                <span class="font-medium text-foreground">Not copied</span>
                <span>{migrationExcludedCategories.map((category) => category.replaceAll("_", " ")).join(", ")}</span>
              </div>
            </div>
          {/if}
          {/if}

          {#if migrationState.phase === "committed" && migrationState.result}
            <div class="grid gap-2 rounded-md border p-3">
              <div class="flex items-center gap-2">
                <Badge>{migrationState.result.project_count} bindings moved</Badge>
                <span>Historical jobs keep their original runtime attribution.</span>
              </div>
              <Button size="sm" variant="outline" onclick={prepareRollback}>
                <RotateCcw /> Prepare rollback preview
              </Button>
            </div>
          {:else if migrationState.phase === "rollback_previewing"}
            <p class="text-muted-foreground">Preparing rollback preview...</p>
          {:else if migrationState.phase === "rollback_previewed" && migrationState.rollbackPreview}
            <div class="grid gap-3 rounded-md border p-3">
              <div class="flex flex-wrap items-center gap-2">
                <Badge variant={migrationState.rollbackPreview.restorable ? "default" : "destructive"}>
                  {migrationState.rollbackPreview.restorable ? "Rollback ready for confirmation" : "Rollback blocked"}
                </Badge>
                <span>{migrationState.rollbackPreview.project_count} binding(s)</span>
              </div>
              <div class="grid gap-1 text-muted-foreground">
                <span>Restore source: {migrationState.rollbackPreview.source.display_name}</span>
                <span>From target: {migrationState.rollbackPreview.target.display_name}</span>
                <span>{migrationState.rollbackPreview.expires_in_seconds ? `Preview expires in ${migrationState.rollbackPreview.expires_in_seconds}s.` : "Prepare a new preview when ready."}</span>
              </div>
              <p class="text-muted-foreground">
                Rollback restores only recorded explicit project pins. It does not restore engine data, ownership, settings, credentials, or project files.
              </p>
              {#if migrationState.rollbackPreview.blocker}<p class="text-destructive">{boundedRollbackBlocker(migrationState.rollbackPreview.blocker)}</p>{/if}
              <Button
                size="sm"
                variant="destructive"
                disabled={!migrationState.rollbackPreview.restorable || !migrationState.rollbackPreview.plan_id}
                onclick={confirmRollback}
              >
                Confirm rollback
              </Button>
            </div>
          {:else if migrationState.phase === "rolled_back" && migrationState.rollbackResult}
            <p class="rounded-md border p-3 text-muted-foreground">
              {migrationState.rollbackResult.restored_project_count} explicit project binding(s) restored.
            </p>
          {/if}
        </div>
      {/if}

      {#if migrationState.error}<p class="text-destructive">{migrationState.error}</p>{/if}
    </div>

    <Dialog.Footer>
      {#if migrationState.phase !== "idle" || migrationState.workflow !== null}
        <Button variant="outline" onclick={backToChoices}>Back</Button>
      {/if}
      <Button variant="outline" onclick={() => setOpen(false)}>Close</Button>
      {#if migrationState.workflow === "connect" && inventory}
        <Button disabled={!selectedExternalProfile} onclick={openContextChange}>Review default runtime</Button>
      {:else if migrationState.workflow === "migrate" && inventory}
        <Button
          variant="outline"
          disabled={migrationState.phase === "loading_inventory" || migrationState.phase === "previewing" || migrationState.phase === "committing"}
          onclick={() => loadInventory("migrate")}
        >
          <RefreshCw /> Refresh inventory
        </Button>
        <Button
          variant="outline"
          disabled={!migrationRequest(migrationState) || migrationState.phase === "previewing" || migrationState.phase === "committing"}
          onclick={previewMigration}
        >
          <RefreshCw /> Preview
        </Button>
        <Button
          disabled={!canCommitMigration(migrationState, Date.now()) || migrationState.phase === "committing"}
          onclick={commitMigration}
        >
          {migrationState.phase === "committing" ? "Migrating..." : "Confirm migration"}
        </Button>
      {/if}
    </Dialog.Footer>
  </Dialog.Content>
</Dialog.Root>
