<script lang="ts">
  import * as Card from "$lib/components/ui/card/index.js";
  import { Badge } from "$lib/components/ui/badge/index.js";
  import { Button } from "$lib/components/ui/button/index.js";
  import { Download, LifeBuoy, RefreshCw, ShieldCheck } from "@lucide/svelte";
  import { invoke, isTauri } from "@tauri-apps/api/core";
  import { checkForUpdate, type UpdateCheckResult } from "$lib/tauri/updater";

  type UpdateUiState = "idle" | "checking" | "none" | "available" | "installing" | "failed";

  let updateState = $state<UpdateUiState>("idle");
  let pendingUpdate = $state<UpdateCheckResult | null>(null);
  let updateMessage = $state("Check whether a newer Susun Studio build is available.");
  let diagnosticsState = $state<"idle" | "exporting" | "done" | "cancelled" | "failed">("idle");

  const canUseDesktopFeatures = $derived(isTauri());

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
</div>
