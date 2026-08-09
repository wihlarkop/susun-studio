<script lang="ts">
  import { onDestroy } from "svelte";
  import { Badge } from "$lib/components/ui/badge/index.js";
  import { Button } from "$lib/components/ui/button/index.js";
  import { readRuntimeMigrationHistory } from "$lib/daemon/client";
  import {
    boundedMigrationError,
    migrationHistoryPresentation,
    type MigrationHistoryPresentationEntry,
  } from "$lib/runtime/migration-state";
  import { ChevronDown, History, RefreshCw, RotateCcw } from "@lucide/svelte";

  let {
    profileRevision,
    onprepareRollback,
  }: {
    profileRevision: string;
    onprepareRollback: (migrationId: string) => void;
  } = $props();

  let expanded = $state(false);
  let loading = $state(false);
  let entries = $state<MigrationHistoryPresentationEntry[]>([]);
  let error = $state<string | null>(null);
  let activeController: AbortController | null = null;
  let generation = 0;
  let lastProfileRevision = "";
  let hasProfileRevision = false;

  $effect(() => {
    const revision = profileRevision;
    if (!hasProfileRevision) {
      hasProfileRevision = true;
      lastProfileRevision = revision;
      return;
    }
    if (revision === lastProfileRevision) return;
    lastProfileRevision = revision;
    abortActiveRequest();
    generation += 1;
    entries = [];
    loading = false;
    error = null;
  });

  onDestroy(abortActiveRequest);

  function abortActiveRequest() {
    activeController?.abort();
    activeController = null;
  }

  async function loadHistory() {
    abortActiveRequest();
    generation += 1;
    const requestGeneration = generation;
    const controller = new AbortController();
    activeController = controller;
    loading = true;
    error = null;
    try {
      const history = await readRuntimeMigrationHistory({ signal: controller.signal });
      if (activeController === controller && !controller.signal.aborted && generation === requestGeneration) {
        entries = migrationHistoryPresentation(history);
      }
    } catch (requestError) {
      if (activeController === controller && !controller.signal.aborted && generation === requestGeneration) {
        error = boundedMigrationError(requestError);
      }
    } finally {
      if (activeController === controller) activeController = null;
      if (generation === requestGeneration) loading = false;
    }
  }

  function toggleExpanded() {
    expanded = !expanded;
    if (expanded) void loadHistory();
    else {
      abortActiveRequest();
      generation += 1;
      loading = false;
    }
  }

  function badgeVariant(entry: MigrationHistoryPresentationEntry): "default" | "destructive" | "outline" {
    if (entry.status === "failed") return "destructive";
    if (entry.status === "unknown") return "outline";
    return "default";
  }
</script>

<section class="border-t pt-5" aria-labelledby="runtime-migration-history-heading">
  <div class="flex flex-wrap items-center justify-between gap-3">
    <div>
      <h2 id="runtime-migration-history-heading" class="text-base font-semibold">Migration history</h2>
      <p class="text-sm text-muted-foreground">Recorded metadata migrations remain independent of the current runtime preference.</p>
    </div>
    <Button size="sm" variant="outline" aria-expanded={expanded} onclick={toggleExpanded}>
      <History />
      {expanded ? "Hide history" : "Show history"}
      <ChevronDown class={expanded ? "rotate-180" : ""} />
    </Button>
  </div>

  {#if expanded}
    <div class="mt-4 grid gap-3">
      <div class="flex justify-end">
        <Button size="sm" variant="outline" disabled={loading} onclick={loadHistory}>
          <RefreshCw />
          Refresh history
        </Button>
      </div>
      {#if loading}
        <p class="text-sm text-muted-foreground">Loading recorded migrations...</p>
      {:else if error}
        <p class="text-sm text-destructive">{error}</p>
      {:else if entries.length === 0}
        <p class="rounded-md border p-3 text-sm text-muted-foreground">No metadata migrations have been recorded.</p>
      {:else}
        <div class="grid gap-3">
          {#each entries as entry (entry.migrationId)}
            <article class="grid gap-3 rounded-md border p-3 text-sm">
              <div class="flex flex-wrap items-center justify-between gap-2">
                <Badge variant={badgeVariant(entry)}>{entry.statusLabel}</Badge>
                <span class="text-muted-foreground">{entry.projectCount} explicit binding(s)</span>
              </div>
              <div class="grid gap-1 text-muted-foreground">
                <span>{entry.sourceLabel} to {entry.targetLabel}</span>
                <span>{new Date(entry.completedAtMs || entry.createdAtMs).toLocaleString()}</span>
              </div>
              {#if entry.skippedLabels.length > 0}
                <ul class="grid gap-1 text-muted-foreground">
                  {#each entry.skippedLabels as label}<li>{label}</li>{/each}
                </ul>
              {/if}
              {#if entry.failureLabels.length > 0}
                <ul class="grid gap-1 text-destructive">
                  {#each entry.failureLabels as label}<li>{label}</li>{/each}
                </ul>
              {/if}
              {#if entry.canPrepareRollback}
                <div>
                  <Button size="sm" variant="outline" onclick={() => onprepareRollback(entry.migrationId)}>
                    <RotateCcw /> Prepare rollback
                  </Button>
                </div>
              {/if}
            </article>
          {/each}
        </div>
      {/if}
    </div>
  {/if}
</section>
