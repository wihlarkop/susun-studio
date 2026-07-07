<script lang="ts">
  import * as Sidebar from "$lib/components/ui/sidebar/index.js";
  import * as Tooltip from "$lib/components/ui/tooltip/index.js";
  import { Badge } from "$lib/components/ui/badge/index.js";
  import { Button } from "$lib/components/ui/button/index.js";
  import { Separator } from "$lib/components/ui/separator/index.js";
  import { Download, LifeBuoy, Moon, Plus, RefreshCw, Sun } from "@lucide/svelte";
  import { toggleMode } from "mode-watcher";
  import { invoke, isTauri } from "@tauri-apps/api/core";
  import { checkForUpdate, type UpdateCheckResult } from "$lib/tauri/updater";
  import type { HealthState } from "$lib/daemon/daemon-state.svelte";

  let {
    healthState,
    onImportClick,
  }: { healthState: HealthState; onImportClick: () => void } = $props();

  const connected = $derived(healthState.kind === "connected");

  async function exportDiagnostics() {
    try {
      await invoke("export_diagnostics_bundle");
    } catch (error) {
      console.error("failed to export diagnostics bundle", error);
    }
  }

  type UpdateUiState = "idle" | "checking" | "none" | "available" | "installing";

  let updateState = $state<UpdateUiState>("idle");
  let pendingUpdate = $state<UpdateCheckResult | null>(null);

  async function handleCheckForUpdate() {
    updateState = "checking";
    const result = await checkForUpdate();
    if (!result.available) {
      updateState = "none";
      pendingUpdate = null;
      return;
    }
    updateState = "available";
    pendingUpdate = result;
  }

  async function handleInstallUpdate() {
    if (!pendingUpdate?.available) {
      return;
    }
    updateState = "installing";
    await pendingUpdate.install();
  }
</script>

<header class="flex items-center justify-between gap-4">
  <div class="flex items-center gap-3">
    <Sidebar.Trigger />
    <Separator orientation="vertical" class="h-6" />
    <h2 class="text-2xl leading-tight font-semibold">Projects</h2>
    <Tooltip.Provider>
      <Tooltip.Root>
        <Tooltip.Trigger>
          <Badge variant={connected ? "default" : "outline"}>{healthState.label}</Badge>
        </Tooltip.Trigger>
        <Tooltip.Content>{healthState.detail}</Tooltip.Content>
      </Tooltip.Root>
    </Tooltip.Provider>
  </div>
  <div class="flex items-center gap-2">
    {#if isTauri()}
      {#if updateState === "available"}
        <Button
          variant="default"
          size="sm"
          aria-label={`Install update ${pendingUpdate?.available ? pendingUpdate.version : ""}`}
          onclick={handleInstallUpdate}
        >
          <Download />
          Install update
        </Button>
      {:else}
        <Button
          variant="ghost"
          size="icon"
          aria-label="Check for updates"
          title={updateState === "checking" ? "Checking for updates…" : "Check for updates"}
          disabled={updateState === "checking" || updateState === "installing"}
          onclick={handleCheckForUpdate}
        >
          <RefreshCw />
        </Button>
      {/if}
      <Button
        variant="ghost"
        size="icon"
        aria-label="Export diagnostics bundle"
        title="Export diagnostics bundle"
        onclick={exportDiagnostics}
      >
        <LifeBuoy />
      </Button>
    {/if}
    <Button variant="ghost" size="icon" aria-label="Toggle theme" onclick={toggleMode}>
      <Sun class="dark:hidden" />
      <Moon class="hidden dark:block" />
    </Button>
    <Button disabled={!connected} onclick={onImportClick} title="Ctrl+I">
      <Plus />
      Import Project
    </Button>
  </div>
</header>
