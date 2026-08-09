<script lang="ts">
  import { Button } from "$lib/components/ui/button/index.js";
  import { RefreshCw } from "@lucide/svelte";
  import {
    readRuntimeCompatibility,
    type RuntimeCompatibilityReport,
    type RuntimeProfile,
  } from "$lib/daemon/client";
  import {
    acceptsCompatibilityResult,
    boundedCompatibilityError,
    compatibilityGroups,
    compatibilityLevelLabel,
    shouldRequestCompatibility,
  } from "$lib/runtime/compatibility";

  let { profile }: { profile: RuntimeProfile } = $props();
  let open = $state(false);
  let report = $state<RuntimeCompatibilityReport | null>(null);
  let loading = $state(false);
  let message = $state<string | null>(null);
  let generation = 0;
  let refreshRevision = $state(0);

  $effect(() => {
    const profileId = profile.id;
    const requestRevision = refreshRevision;
    if (!shouldRequestCompatibility(open, profileId, profile.id)) return;
    const requestGeneration = ++generation;
    const controller = new AbortController();
    loading = true;
    report = null;
    message = null;
    void readRuntimeCompatibility(profileId, { signal: controller.signal })
      .then((next) => {
        if (
          controller.signal.aborted ||
          requestRevision !== refreshRevision ||
          !acceptsCompatibilityResult(profileId, requestGeneration, profile.id, generation)
        ) {
          return;
        }
        report = next;
      })
      .catch((error) => {
        if (
          controller.signal.aborted ||
          requestRevision !== refreshRevision ||
          !acceptsCompatibilityResult(profileId, requestGeneration, profile.id, generation)
        ) {
          return;
        }
        report = null;
        message = boundedCompatibilityError(error);
      })
      .finally(() => {
        if (
          requestRevision === refreshRevision &&
          acceptsCompatibilityResult(profileId, requestGeneration, profile.id, generation)
        ) {
          loading = false;
        }
      });
    return () => controller.abort();
  });

  function toggle() {
    open = !open;
    if (!open) {
      generation += 1;
      report = null;
      message = null;
    }
  }

  function refresh() {
    refreshRevision += 1;
  }
</script>

<div class="mt-3 border-t pt-3">
  <div class="flex flex-wrap items-center gap-2">
    <Button size="sm" variant="outline" onclick={toggle} aria-expanded={open}>
      {open ? "Hide compatibility" : "Compatibility"}
    </Button>
    {#if open}
      <Button size="sm" variant="ghost" onclick={refresh} disabled={loading}>
        <RefreshCw class={loading ? "animate-spin" : undefined} />
        Refresh
      </Button>
    {/if}
  </div>
  {#if open}
    <div class="mt-3 grid gap-3 text-sm">
      {#if loading}
        <p class="text-muted-foreground">Checking compatibility...</p>
      {:else if message}
        <p class="text-destructive">{message}</p>
      {:else if report}
        <p class="text-muted-foreground">
          Probe-based compatibility{report.observed_api_version ? `, Docker API ${report.observed_api_version}` : ""}.
        </p>
        <p class="text-xs text-muted-foreground">
          Checked {new Date(report.observed_at_ms).toLocaleTimeString()}.
        </p>
        {#each compatibilityGroups(report.workflows) as group (group.id)}
          <div class="grid gap-1">
            <span class="font-medium">{group.label}</span>
            {#each group.workflows as workflow (workflow.id)}
              <div class="grid gap-1 border-l-2 pl-2 sm:grid-cols-[minmax(0,1fr)_auto] sm:items-center">
                <span>{workflow.label}</span>
                <span class="text-muted-foreground">{compatibilityLevelLabel(workflow.level)}</span>
                <span class="text-xs text-muted-foreground sm:col-span-2">{workflow.detail}</span>
              </div>
            {/each}
          </div>
        {/each}
      {/if}
    </div>
  {/if}
</div>
