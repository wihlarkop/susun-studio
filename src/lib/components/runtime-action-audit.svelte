<script lang="ts">
  import * as Card from "$lib/components/ui/card/index.js";
  import { Badge } from "$lib/components/ui/badge/index.js";
  import { Button } from "$lib/components/ui/button/index.js";
  import {
    clearRuntimeActionAudit,
    listRuntimeActionAudit,
    type RuntimeActionAuditEntry,
  } from "$lib/daemon/client";
  import { History, Trash2 } from "@lucide/svelte";

  let entries = $state<RuntimeActionAuditEntry[]>([]);
  let loading = $state(false);
  let clearing = $state(false);
  let errorMessage = $state<string | null>(null);

  $effect(() => {
    void load();
  });

  async function load() {
    loading = true;
    try {
      entries = await listRuntimeActionAudit();
      errorMessage = null;
    } catch (error) {
      errorMessage = error instanceof Error ? error.message : String(error);
    } finally {
      loading = false;
    }
  }

  async function clearHistory() {
    clearing = true;
    try {
      await clearRuntimeActionAudit();
      await load();
    } catch (error) {
      errorMessage = error instanceof Error ? error.message : String(error);
    } finally {
      clearing = false;
    }
  }

  function statusVariant(
    status: string,
  ): "default" | "secondary" | "outline" | "destructive" {
    if (status === "completed") return "default";
    if (status === "rejected" || status === "failed") return "destructive";
    return "secondary";
  }

  function label(value: string): string {
    return value.replaceAll("_", " ");
  }

  function when(ms: number): string {
    return new Date(ms).toLocaleString();
  }
</script>

<Card.Root class="gap-0 overflow-hidden p-0">
  <div class="flex items-center justify-between border-b bg-muted/20 p-4">
    <div class="flex items-center gap-2">
      <History class="size-4 text-muted-foreground" />
      <div>
        <h4 class="text-sm font-semibold">Destructive action history</h4>
        <p class="text-xs text-muted-foreground">
          Redacted, secret-free audit of migrations, resets, prune, and restore. Clearing keeps
          runtime ownership evidence intact.
        </p>
      </div>
    </div>
    <div class="flex items-center gap-2">
      <Badge variant="outline">{entries.length}</Badge>
      <Button
        size="sm"
        variant="outline"
        disabled={clearing || loading || entries.length === 0}
        onclick={clearHistory}
      >
        <Trash2 />
        {clearing ? "Clearing…" : "Clear history"}
      </Button>
    </div>
  </div>

  {#if errorMessage}
    <p class="p-4 text-sm text-destructive">{errorMessage}</p>
  {:else if entries.length === 0}
    <p class="p-4 text-sm text-muted-foreground">No destructive actions recorded.</p>
  {:else}
    <ul class="max-h-72 divide-y overflow-auto">
      {#each entries as entry (entry.id)}
        <li class="grid gap-2 p-3 text-sm md:grid-cols-[minmax(0,1fr)_auto] md:items-start">
          <div class="min-w-0">
            <div class="flex flex-wrap items-center gap-2">
              <span class="font-medium">{label(entry.action_kind)}</span>
              <Badge variant={statusVariant(entry.terminal_status)} class="text-xs">
                {label(entry.terminal_status)}
              </Badge>
              <Badge variant="outline" class="text-xs">{label(entry.ownership_result)}</Badge>
              {#if entry.failure_code}
                <Badge variant="destructive" class="text-xs">{label(entry.failure_code)}</Badge>
              {/if}
            </div>
            <div class="mt-1 flex flex-wrap gap-x-4 gap-y-1 text-xs text-muted-foreground">
              {#each entry.affected as item (item.category)}
                <span>{label(item.category)}: {item.count}</span>
              {/each}
              {#if entry.command_kind}<span>{label(entry.command_kind)}</span>{/if}
            </div>
          </div>
          <span class="text-xs text-muted-foreground md:text-right">{when(entry.started_at_ms)}</span>
        </li>
      {/each}
    </ul>
  {/if}
</Card.Root>
