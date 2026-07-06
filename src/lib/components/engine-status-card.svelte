<script lang="ts">
  import * as Card from "$lib/components/ui/card/index.js";
  import { Badge } from "$lib/components/ui/badge/index.js";
  import { Button } from "$lib/components/ui/button/index.js";
  import { RefreshCw, Trash2 } from "@lucide/svelte";
  import PruneDialog from "./prune-dialog.svelte";
  import {
    listEngines,
    readEngineCapabilities,
    readEngineHealth,
    type EngineCapabilities,
    type StudioEngine,
  } from "$lib/daemon/client";

  let engine = $state<StudioEngine | null>(null);
  let capabilities = $state<EngineCapabilities | null>(null);
  let checking = $state(false);
  let pruneDialogOpen = $state(false);

  $effect(() => {
    const controller = new AbortController();
    listEngines({ signal: controller.signal })
      .then((engines) => {
        engine = engines.find((item) => item.is_default) ?? engines[0] ?? null;
        if (engine) {
          void recheck(controller.signal);
        }
      })
      .catch(() => {
        engine = null;
      });
    return () => controller.abort();
  });

  async function recheck(signal?: AbortSignal) {
    if (!engine) {
      return;
    }
    checking = true;
    try {
      const health = await readEngineHealth(engine.id, { signal });
      engine = { ...engine, last_health: health };
      capabilities = health.reachable
        ? await readEngineCapabilities(engine.id, { signal })
        : null;
    } catch {
      capabilities = null;
    } finally {
      checking = false;
    }
  }

  function levelVariant(level: string): "default" | "outline" | "secondary" {
    if (level === "supported" || level === "supported_subset") return "default";
    if (level === "unsupported") return "outline";
    return "secondary";
  }

  const capabilityRows = $derived(
    capabilities
      ? [
          { label: "Healthchecks", level: capabilities.supports_health },
          { label: "Named volumes", level: capabilities.supports_named_volumes },
          { label: "Network aliases", level: capabilities.supports_network_aliases },
          { label: "Log follow", level: capabilities.supports_log_follow },
          { label: "Image build", level: capabilities.supports_build },
        ]
      : [],
  );
</script>

{#if engine}
  <Card.Root class="gap-3 p-4">
    <div class="flex items-center justify-between gap-2">
      <div class="flex items-center gap-2">
        <h3 class="text-sm font-semibold">{engine.display_name}</h3>
        {#if engine.last_health}
          <Badge variant={engine.last_health.reachable ? "default" : "destructive"}>
            {engine.last_health.reachable ? "Reachable" : "Unreachable"}
          </Badge>
        {:else}
          <Badge variant="outline">Checking…</Badge>
        {/if}
        {#if engine.last_health?.api_version}
          <span class="text-xs text-muted-foreground">
            Docker API {engine.last_health.api_version}
          </span>
        {/if}
      </div>
      <div class="flex items-center gap-2">
        <Button size="sm" variant="outline" disabled={checking} onclick={() => recheck()}>
          <RefreshCw />
          Recheck
        </Button>
        <Button size="sm" variant="destructive" onclick={() => (pruneDialogOpen = true)}>
          <Trash2 />
          Prune
        </Button>
      </div>
    </div>

    {#if engine.last_health?.error}
      <p class="text-xs text-destructive">{engine.last_health.error}</p>
    {/if}

    {#if capabilityRows.length > 0}
      <div class="flex flex-wrap gap-1.5">
        {#each capabilityRows as row (row.label)}
          <Badge variant={levelVariant(row.level)} class="text-xs">
            {row.label}: {row.level.replace("_", " ")}
          </Badge>
        {/each}
        {#if capabilities?.supports_mount_types.length}
          <Badge variant="secondary" class="text-xs">
            mounts: {capabilities.supports_mount_types.join(", ")}
          </Badge>
        {/if}
      </div>
    {/if}
  </Card.Root>
  <PruneDialog engineId={engine.id} bind:open={pruneDialogOpen} />
{/if}
