<script lang="ts">
  import { Badge } from "$lib/components/ui/badge/index.js";
  import { Button } from "$lib/components/ui/button/index.js";
  import { Skeleton } from "$lib/components/ui/skeleton/index.js";
  import type {
    RuntimeResourceMetric,
    RuntimeResourceSnapshot,
    RuntimeResourceSupport,
    RuntimeResourceText,
  } from "$lib/daemon/client";
  import { AlertCircle, RefreshCw } from "@lucide/svelte";

  let {
    snapshot,
    loading = false,
    error = null,
    onrefresh,
  }: {
    snapshot: RuntimeResourceSnapshot | null;
    loading?: boolean;
    error?: string | null;
    onrefresh: () => void;
  } = $props();

  function supportVariant(
    support: RuntimeResourceSupport,
  ): "default" | "secondary" | "outline" | "destructive" {
    if (support === "supported") return "default";
    if (support === "unavailable") return "destructive";
    if (support === "unknown") return "outline";
    return "secondary";
  }

  function bytes(value: number): string {
    if (value < 1024) return `${value} B`;
    const units = ["KiB", "MiB", "GiB", "TiB"];
    let next = value / 1024;
    let unit = units[0];
    for (let index = 1; index < units.length && next >= 1024; index += 1) {
      next /= 1024;
      unit = units[index];
    }
    return `${next >= 10 ? next.toFixed(0) : next.toFixed(1)} ${unit}`;
  }

  function metricValue(metric: RuntimeResourceMetric): string {
    if (metric.value === null) return metric.support.replaceAll("_", " ");
    if (metric.unit === "bytes") return bytes(metric.value);
    if (metric.unit === "cores") return `${metric.value} ${metric.value === 1 ? "core" : "cores"}`;
    return `${metric.value}`;
  }

  function textValue(field: RuntimeResourceText): string {
    if (!field.value) return field.support.replaceAll("_", " ");
    if (field.value === "provider_managed_user_scope") return "Current-user storage";
    if (field.value === "user_mode") return "User-mode networking";
    if (field.value === "wsl") return "WSL networking";
    return field.value.replaceAll("_", " ");
  }
</script>

<div class="mt-3 border-t pt-3">
  <div class="flex flex-wrap items-center justify-between gap-2">
    <div>
      <div class="text-xs font-semibold">Resources</div>
      <p class="text-xs text-muted-foreground">Observed provider allocation and capability.</p>
    </div>
    <Button size="icon-sm" variant="ghost" disabled={loading} title="Refresh resources" onclick={onrefresh}>
      <RefreshCw class={loading ? "animate-spin" : ""} />
      <span class="sr-only">Refresh resources</span>
    </Button>
  </div>

  {#if error}
    <div class="mt-3 flex gap-2 text-xs text-destructive">
      <AlertCircle class="mt-0.5 size-3.5 shrink-0" />
      <span>{error}</span>
    </div>
  {:else if !snapshot}
    {#if loading}
      <div class="mt-3 grid gap-3 sm:grid-cols-2 xl:grid-cols-3" aria-label="Reading provider resources">
        {#each Array(6) as _}
          <div class="space-y-2">
            <Skeleton class="h-3 w-20" />
            <Skeleton class="h-4 w-28" />
          </div>
        {/each}
      </div>
    {:else}
      <p class="mt-3 text-xs text-muted-foreground">Resource information is not loaded.</p>
    {/if}
  {:else}
    <dl class="mt-3 grid gap-x-5 gap-y-3 sm:grid-cols-2 xl:grid-cols-3">
      {#each [
        { label: "CPU", field: snapshot.cpu, value: metricValue(snapshot.cpu) },
        { label: "Memory", field: snapshot.memory, value: metricValue(snapshot.memory) },
        {
          label: "Disk allocation",
          field: snapshot.disk_allocation,
          value: metricValue(snapshot.disk_allocation),
        },
        { label: "Disk usage", field: snapshot.disk_usage, value: metricValue(snapshot.disk_usage) },
        { label: "Volumes", field: snapshot.volumes, value: metricValue(snapshot.volumes) },
      ] as item (item.label)}
        <div class="min-w-0">
          <dt class="flex items-center gap-2 text-xs text-muted-foreground">
            <span>{item.label}</span>
            <Badge variant={supportVariant(item.field.support)} class="px-1.5 py-0 text-[10px]">
              {item.field.support}
            </Badge>
          </dt>
          <dd class="mt-1 text-sm font-medium">{item.value}</dd>
          {#if item.field.detail}
            <p class="mt-1 text-xs leading-5 text-muted-foreground">{item.field.detail}</p>
          {/if}
        </div>
      {/each}
      {#each [
        { label: "Data location", field: snapshot.data_location },
        { label: "Network", field: snapshot.network },
      ] as item (item.label)}
        <div class="min-w-0">
          <dt class="flex items-center gap-2 text-xs text-muted-foreground">
            <span>{item.label}</span>
            <Badge variant={supportVariant(item.field.support)} class="px-1.5 py-0 text-[10px]">
              {item.field.support}
            </Badge>
          </dt>
          <dd class="mt-1 text-sm font-medium">{textValue(item.field)}</dd>
          {#if item.field.detail}
            <p class="mt-1 text-xs leading-5 text-muted-foreground">{item.field.detail}</p>
          {/if}
        </div>
      {/each}
    </dl>
  {/if}
</div>
