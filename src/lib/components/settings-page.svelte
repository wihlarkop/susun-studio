<script lang="ts">
  import * as Card from "$lib/components/ui/card/index.js";
  import { Badge } from "$lib/components/ui/badge/index.js";
  import { Button } from "$lib/components/ui/button/index.js";
  import {
    ArchiveRestore,
    DatabaseBackup,
    Download,
    LifeBuoy,
    RefreshCw,
    ShieldCheck,
    Trash2,
  } from "@lucide/svelte";
  import { invoke, isTauri } from "@tauri-apps/api/core";
  import {
    readRuntimeUninstallPolicy,
    setDaemonConnection,
    type RuntimeUninstallPolicy,
  } from "$lib/daemon/client";
  import { checkForUpdate, type UpdateCheckResult } from "$lib/tauri/updater";

  type UpdateUiState = "idle" | "checking" | "none" | "available" | "installing" | "failed";

  type RestorePreview = {
    compatible: boolean;
    reason: string | null;
    manifest: {
      app_version: string;
      schema_migration_version: number;
      current_schema_migration_version: number;
      platform_os: string;
      platform_arch: string;
      created_at_epoch_seconds: number;
      project_count: number;
      runtime_profile_count: number;
      job_count: number;
      content_classes: string[];
      excluded: string[];
    };
    replacement_scope: string[];
    reenter_after_restore: string[];
  };

  type RestorePreviewOutcome =
    | { outcome: "cancelled" }
    | { outcome: "previewed"; preview: RestorePreview; archive_path: string };

  type DaemonConnectionPayload = { base_url: string; token: string };

  type RestoreSummary = {
    app_version: string;
    schema_migration_version: number;
    current_schema_migration_version: number;
    project_count: number;
    runtime_profile_count: number;
    job_count: number;
  };

  type RestoreOutcome =
    | { outcome: "restored"; summary: RestoreSummary; connection: DaemonConnectionPayload }
    | { outcome: "rolled_back"; reason: string; connection: DaemonConnectionPayload };

  const backupIncludes = [
    "Projects, preferences, and runtime profile bindings",
    "Runtime profiles, plans, jobs, and history",
    "A consistent database snapshot with a versioned, checksummed manifest",
  ];
  const backupExcludes = [
    "Registry credentials and auth tokens",
    "Updater keys and raw endpoint secrets",
    "Engine images, containers, and volumes",
  ];

  let updateState = $state<UpdateUiState>("idle");
  let pendingUpdate = $state<UpdateCheckResult | null>(null);
  let updateMessage = $state("Check whether a newer Susun Studio build is available.");
  let diagnosticsState = $state<"idle" | "exporting" | "done" | "cancelled" | "failed">("idle");
  let backupState = $state<"idle" | "backing-up" | "saved" | "cancelled" | "failed">("idle");
  let backupError = $state<string | null>(null);
  let restoreState = $state<"idle" | "validating" | "previewed" | "cancelled" | "failed">("idle");
  let restorePreview = $state<RestorePreview | null>(null);
  let restoreArchivePath = $state<string | null>(null);
  let restoreError = $state<string | null>(null);
  let applyState = $state<"idle" | "applying" | "restored" | "rolled_back" | "failed">("idle");
  let applyMessage = $state<string | null>(null);
  let uninstallPolicy = $state<RuntimeUninstallPolicy | null>(null);
  let uninstallPolicyError = $state<string | null>(null);

  const canUseDesktopFeatures = $derived(isTauri());

  $effect(() => {
    const controller = new AbortController();
    void readRuntimeUninstallPolicy({ signal: controller.signal })
      .then((policy) => {
        uninstallPolicy = policy;
        uninstallPolicyError = null;
      })
      .catch((error) => {
        if (!controller.signal.aborted) {
          uninstallPolicyError = error instanceof Error ? error.message : String(error);
        }
      });
    return () => controller.abort();
  });

  async function backUpStudioData() {
    backupState = "backing-up";
    backupError = null;
    try {
      const outcome = await invoke<"saved" | "cancelled">("backup_studio_data");
      backupState = outcome === "cancelled" ? "cancelled" : "saved";
    } catch (error) {
      backupState = "failed";
      backupError = error instanceof Error ? error.message : String(error);
    }
  }

  async function previewRestore() {
    restoreState = "validating";
    restoreError = null;
    restorePreview = null;
    restoreArchivePath = null;
    applyState = "idle";
    applyMessage = null;
    try {
      const outcome = await invoke<RestorePreviewOutcome>("preview_restore_studio_data");
      if (outcome.outcome === "cancelled") {
        restoreState = "cancelled";
        return;
      }
      restorePreview = outcome.preview;
      restoreArchivePath = outcome.archive_path;
      restoreState = "previewed";
    } catch (error) {
      restoreState = "failed";
      restoreError = error instanceof Error ? error.message : String(error);
    }
  }

  async function applyRestore() {
    if (!restoreArchivePath || !restorePreview?.compatible) return;
    applyState = "applying";
    applyMessage = null;
    try {
      const outcome = await invoke<RestoreOutcome>("apply_restore_studio_data", {
        archivePath: restoreArchivePath,
      });
      // The daemon restarted on a new port/token — re-point the client at it.
      setDaemonConnection({
        baseUrl: outcome.connection.base_url,
        token: outcome.connection.token,
      });
      if (outcome.outcome === "restored") {
        applyState = "restored";
        applyMessage = `Restored ${outcome.summary.project_count} projects. Reloading…`;
        setTimeout(() => location.reload(), 1200);
      } else {
        applyState = "rolled_back";
        applyMessage = `Restore failed and your previous data was kept: ${outcome.reason}`;
      }
    } catch (error) {
      applyState = "failed";
      applyMessage = error instanceof Error ? error.message : String(error);
    }
  }

  function formatEpochSeconds(seconds: number): string {
    return new Date(seconds * 1000).toLocaleString();
  }

  async function handleCheckForUpdate() {
    updateState = "checking";
    updateMessage = "Checking for updates...";
    try {
      const result = await checkForUpdate();
      if (!result.available) {
        updateState = "none";
        pendingUpdate = null;
        updateMessage =
          result.reason === "unpublished"
            ? "No published update feed was found for this build yet."
            : "You are running the latest available build.";
        return;
      }
      updateState = "available";
      pendingUpdate = result;
      updateMessage = `Version ${result.version} is ready to install.`;
    } catch (error) {
      updateState = "failed";
      pendingUpdate = null;
      updateMessage = error instanceof Error ? error.message : String(error);
    }
  }

  async function handleInstallUpdate() {
    if (!pendingUpdate?.available) return;
    updateState = "installing";
    updateMessage = `Installing version ${pendingUpdate.version}...`;
    try {
      await pendingUpdate.install();
    } catch (error) {
      updateState = "failed";
      updateMessage = error instanceof Error ? error.message : String(error);
    }
  }

  async function exportDiagnostics() {
    diagnosticsState = "exporting";
    try {
      const outcome = await invoke<"exported" | "cancelled">("export_diagnostics_bundle");
      diagnosticsState = outcome === "cancelled" ? "cancelled" : "done";
    } catch (error) {
      diagnosticsState = "failed";
      console.error("failed to export diagnostics bundle", error);
    }
  }

  function updateBadgeVariant(): "default" | "secondary" | "outline" | "destructive" {
    if (updateState === "available") return "default";
    if (updateState === "failed") return "destructive";
    if (updateState === "checking" || updateState === "installing") return "secondary";
    return "outline";
  }

  function updateBadgeLabel(): string {
    if (updateState === "available") return "Update available";
    if (updateState === "checking") return "Checking";
    if (updateState === "installing") return "Installing";
    if (updateState === "none") return "Up to date";
    if (updateState === "failed") return "Failed";
    return "Not checked";
  }
</script>

<div class="flex flex-col gap-4">
  <div class="max-w-3xl">
    <h3 class="text-lg font-semibold">Settings</h3>
    <p class="text-sm text-muted-foreground">
      Manage app maintenance actions that are outside a single project or runtime.
    </p>
  </div>

  <Card.Root class="gap-0 overflow-hidden p-0">
    <div class="border-b bg-muted/20 p-4">
      <div class="flex min-w-0 gap-3">
        <div class="flex size-9 shrink-0 items-center justify-center rounded-md border bg-background">
          <RefreshCw class="size-4 text-muted-foreground" />
        </div>
        <div class="min-w-0">
          <div class="flex flex-wrap items-center gap-2">
            <h4 class="text-base font-semibold">Software update</h4>
            <Badge variant={updateBadgeVariant()}>{updateBadgeLabel()}</Badge>
          </div>
          <p class="mt-1 max-w-2xl text-sm text-muted-foreground">{updateMessage}</p>
        </div>
      </div>
    </div>

    <div class="grid gap-4 p-4 md:grid-cols-[minmax(0,1fr)_auto] md:items-center">
      <div class="flex min-w-0 items-start gap-3">
        <ShieldCheck class="mt-0.5 size-4 shrink-0 text-muted-foreground" />
        <div class="min-w-0">
          <p class="text-sm font-medium">Updater channel</p>
          <p class="text-xs text-muted-foreground">
            Updates use the signed Tauri updater configured for this app build.
          </p>
        </div>
      </div>
      <div class="flex flex-wrap gap-2 md:justify-end">
        <Button
          variant="outline"
          disabled={!canUseDesktopFeatures || updateState === "checking" || updateState === "installing"}
          onclick={handleCheckForUpdate}
        >
          <RefreshCw />
          {updateState === "checking" ? "Checking" : "Check for update"}
        </Button>
        <Button
          disabled={!pendingUpdate?.available || updateState === "installing"}
          onclick={handleInstallUpdate}
        >
          <Download />
          {updateState === "installing" ? "Installing" : "Install update"}
        </Button>
      </div>
    </div>

    {#if !canUseDesktopFeatures}
      <div class="border-t p-4 text-sm text-muted-foreground">
        Update checks are available in the desktop app build.
      </div>
    {/if}
  </Card.Root>

  <Card.Root class="gap-0 overflow-hidden p-0">
    <div class="grid gap-4 p-4 md:grid-cols-[minmax(0,1fr)_auto] md:items-center">
      <div class="flex min-w-0 gap-3">
        <div class="flex size-9 shrink-0 items-center justify-center rounded-md border bg-background">
          <LifeBuoy class="size-4 text-muted-foreground" />
        </div>
        <div class="min-w-0">
          <div class="flex flex-wrap items-center gap-2">
            <h4 class="text-base font-semibold">Diagnostics bundle</h4>
            {#if diagnosticsState !== "idle"}
              <Badge
                variant={diagnosticsState === "failed"
                  ? "destructive"
                  : diagnosticsState === "cancelled"
                    ? "outline"
                    : "secondary"}
              >
                {diagnosticsState === "done" ? "exported" : diagnosticsState}
              </Badge>
            {/if}
          </div>
          <p class="mt-1 max-w-2xl text-sm text-muted-foreground">
            Export app diagnostics with sensitive values redacted.
          </p>
        </div>
      </div>
      <Button
        variant="outline"
        disabled={!canUseDesktopFeatures || diagnosticsState === "exporting"}
        onclick={exportDiagnostics}
      >
        <LifeBuoy />
        {diagnosticsState === "exporting" ? "Exporting" : "Export diagnostics"}
      </Button>
    </div>
  </Card.Root>

  <Card.Root class="gap-0 overflow-hidden p-0">
    <div class="border-b bg-muted/20 p-4">
      <div class="flex min-w-0 items-start justify-between gap-3">
        <div class="flex min-w-0 gap-3">
          <div class="flex size-9 shrink-0 items-center justify-center rounded-md border bg-background">
            <DatabaseBackup class="size-4 text-muted-foreground" />
          </div>
          <div class="min-w-0">
            <div class="flex flex-wrap items-center gap-2">
              <h4 class="text-base font-semibold">Back up Studio data</h4>
              {#if backupState !== "idle"}
                <Badge
                  variant={backupState === "failed"
                    ? "destructive"
                    : backupState === "cancelled"
                      ? "outline"
                      : backupState === "saved"
                        ? "default"
                        : "secondary"}
                >
                  {backupState === "backing-up" ? "backing up" : backupState}
                </Badge>
              {/if}
            </div>
            <p class="mt-1 max-w-2xl text-sm text-muted-foreground">
              Save a snapshot of Studio metadata. This is separate from the diagnostics bundle.
            </p>
          </div>
        </div>
        <Button
          variant="outline"
          disabled={!canUseDesktopFeatures || backupState === "backing-up"}
          onclick={backUpStudioData}
        >
          <DatabaseBackup />
          {backupState === "backing-up" ? "Backing up" : "Back up"}
        </Button>
      </div>
    </div>

    <div class="grid gap-4 p-4 md:grid-cols-2">
      <div class="rounded-md border p-3">
        <div class="text-xs font-medium text-muted-foreground">Included</div>
        <ul class="mt-2 space-y-1 text-sm">
          {#each backupIncludes as item (item)}
            <li class="leading-5">{item}</li>
          {/each}
        </ul>
      </div>
      <div class="rounded-md border p-3">
        <div class="text-xs font-medium text-muted-foreground">Not included</div>
        <ul class="mt-2 space-y-1 text-sm text-muted-foreground">
          {#each backupExcludes as item (item)}
            <li class="leading-5">{item}</li>
          {/each}
        </ul>
      </div>
    </div>

    {#if backupError}
      <div class="border-t p-4 text-sm text-destructive">{backupError}</div>
    {/if}
  </Card.Root>

  <Card.Root class="gap-0 overflow-hidden p-0">
    <div class="border-b bg-muted/20 p-4">
      <div class="flex min-w-0 items-start justify-between gap-3">
        <div class="flex min-w-0 gap-3">
          <div class="flex size-9 shrink-0 items-center justify-center rounded-md border bg-background">
            <ArchiveRestore class="size-4 text-muted-foreground" />
          </div>
          <div class="min-w-0">
            <div class="flex flex-wrap items-center gap-2">
              <h4 class="text-base font-semibold">Restore Studio data</h4>
              {#if restoreState === "cancelled"}
                <Badge variant="outline">cancelled</Badge>
              {:else if restoreState === "failed"}
                <Badge variant="destructive">invalid</Badge>
              {:else if restorePreview}
                <Badge variant={restorePreview.compatible ? "default" : "destructive"}>
                  {restorePreview.compatible ? "Compatible" : "Incompatible"}
                </Badge>
              {/if}
            </div>
            <p class="mt-1 max-w-2xl text-sm text-muted-foreground">
              Check a backup archive, then restore it. Restoring replaces all Studio data and
              restarts the daemon; your current data is backed up first and kept if anything fails.
            </p>
          </div>
        </div>
        <Button
          variant="outline"
          disabled={!canUseDesktopFeatures || restoreState === "validating" || applyState === "applying"}
          onclick={previewRestore}
        >
          <ArchiveRestore />
          {restoreState === "validating" ? "Checking" : "Check a backup"}
        </Button>
      </div>
    </div>

    {#if restoreError}
      <div class="p-4 text-sm text-destructive">{restoreError}</div>
    {:else if restorePreview}
      {@const preview = restorePreview}
      <div class="grid gap-4 p-4">
        {#if !preview.compatible && preview.reason}
          <div class="rounded-md border border-destructive/30 bg-destructive/5 p-3 text-sm text-destructive">
            {preview.reason}
          </div>
        {/if}

        <div class="grid gap-2 text-sm md:grid-cols-2">
          <div class="rounded-md border p-3">
            <div class="text-xs font-medium text-muted-foreground">Backup</div>
            <p class="mt-1">Created {formatEpochSeconds(preview.manifest.created_at_epoch_seconds)}</p>
            <p class="text-xs text-muted-foreground">
              App {preview.manifest.app_version} · {preview.manifest.platform_os}/{preview.manifest
                .platform_arch} · schema v{preview.manifest.schema_migration_version} (this app v{preview
                .manifest.current_schema_migration_version})
            </p>
          </div>
          <div class="rounded-md border p-3">
            <div class="text-xs font-medium text-muted-foreground">Contents</div>
            <p class="mt-1">
              {preview.manifest.project_count} projects · {preview.manifest.runtime_profile_count} runtime
              profiles · {preview.manifest.job_count} jobs
            </p>
          </div>
        </div>

        <div class="rounded-md border p-3">
          <div class="text-xs font-medium text-muted-foreground">A restore would replace</div>
          <ul class="mt-2 space-y-1 text-sm">
            {#each preview.replacement_scope as item (item)}
              <li class="leading-5">{item}</li>
            {/each}
          </ul>
        </div>

        <div class="rounded-md border p-3">
          <div class="text-xs font-medium text-muted-foreground">You must re-enter after restore</div>
          <ul class="mt-2 space-y-1 text-sm text-muted-foreground">
            {#each preview.reenter_after_restore as item (item)}
              <li class="leading-5">{item}</li>
            {/each}
          </ul>
        </div>

        {#if applyMessage}
          <div
            class="rounded-md border p-3 text-sm {applyState === 'failed' || applyState === 'rolled_back'
              ? 'border-destructive/30 bg-destructive/5 text-destructive'
              : 'text-muted-foreground'}"
          >
            {applyMessage}
          </div>
        {/if}

        {#if preview.compatible && applyState !== "restored"}
          <div class="flex flex-wrap items-center justify-between gap-3 rounded-md border border-destructive/30 bg-destructive/5 p-3">
            <p class="text-sm text-destructive">
              This replaces all current Studio data and restarts the daemon. This can't be undone
              except from your automatic pre-restore backup.
            </p>
            <Button variant="destructive" disabled={applyState === "applying"} onclick={applyRestore}>
              <ArchiveRestore />
              {applyState === "applying" ? "Restoring…" : "Restore now"}
            </Button>
          </div>
        {/if}
      </div>
    {/if}
  </Card.Root>

  <Card.Root class="gap-0 overflow-hidden p-0">
    <div class="border-b bg-muted/20 p-4">
      <div class="flex min-w-0 gap-3">
        <div class="flex size-9 shrink-0 items-center justify-center rounded-md border bg-background">
          <Trash2 class="size-4 text-muted-foreground" />
        </div>
        <div class="min-w-0">
          <div class="flex flex-wrap items-center gap-2">
            <h4 class="text-base font-semibold">Removal and data retention</h4>
            <Badge variant="secondary">Preserve by default</Badge>
          </div>
          <p class="mt-1 max-w-2xl text-sm text-muted-foreground">
            Back up Studio data before uninstalling. External runtimes are never changed by app removal.
          </p>
        </div>
      </div>
    </div>
    {#if uninstallPolicy}
      <div class="grid gap-2 p-4 sm:grid-cols-2">
        {#each uninstallPolicy.choices as choice (choice.id)}
          <div class="flex items-start justify-between gap-3 rounded-md border p-3">
            <div>
              <div class="text-sm font-medium">{choice.label}</div>
              <div class="mt-1 text-xs text-muted-foreground">
                {choice.id === "app_binaries_only"
                  ? "Conservative unattended behavior"
                  : "Requires an explicit user choice"}
              </div>
            </div>
            <Badge variant={choice.selected_by_default ? "default" : "outline"}>
              {choice.selected_by_default ? "Default" : "Optional"}
            </Badge>
          </div>
        {/each}
      </div>
      <div class="border-t p-4 text-xs text-muted-foreground">
        Preserved runtime metadata is revalidated after reinstall. A runtime name alone never restores ownership.
      </div>
    {:else if uninstallPolicyError}
      <p class="p-4 text-sm text-destructive">{uninstallPolicyError}</p>
    {:else}
      <p class="p-4 text-sm text-muted-foreground">Loading retention policy...</p>
    {/if}
  </Card.Root>
</div>
