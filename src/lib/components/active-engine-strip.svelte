<script lang="ts">
  import * as Card from "$lib/components/ui/card/index.js";
  import { Badge } from "$lib/components/ui/badge/index.js";
  import { Button } from "$lib/components/ui/button/index.js";
  import { ChevronDown, RefreshCw, Settings2, Trash2 } from "@lucide/svelte";
  import PruneDialog from "./prune-dialog.svelte";
  import RuntimeIdentity from "./runtime-identity.svelte";
  import {
    readEngineHealth,
    setPreferredRuntime,
    type EngineHealth,
    type RuntimePreference,
    type RuntimeProfile,
  } from "$lib/daemon/client";
  import { resolveActiveEngineId } from "$lib/engine-identity";
  import { presentRuntimeBinding } from "$lib/runtime/presentation";

  let {
    profiles,
    runtimePreference,
    connected,
    onManageRuntimes,
    onChanged,
  }: {
    profiles: RuntimeProfile[];
    runtimePreference: RuntimePreference | undefined;
    connected: boolean;
    onManageRuntimes: () => void;
    onChanged: () => void | Promise<void>;
  } = $props();

  let health = $state<EngineHealth | null>(null);
  let checking = $state(false);
  let switching = $state(false);
  let pruneDialogOpen = $state(false);

  const binding = $derived(runtimePreference?.binding ?? null);
  const selected = $derived(
    binding?.profile_id
      ? (profiles.find((profile) => profile.id === binding.profile_id) ?? null)
      : null,
  );
  const activeEngineId = $derived(binding ? resolveActiveEngineId(binding) : null);

  $effect(() => {
    const engineId = activeEngineId;
    if (!connected || !engineId) {
      health = null;
      return;
    }
    const controller = new AbortController();
    void recheck(engineId, controller.signal);
    return () => controller.abort();
  });

  async function recheck(engineId: string | null = activeEngineId, signal?: AbortSignal) {
    if (!engineId) return;
    checking = true;
    try {
      health = await readEngineHealth(engineId, { signal });
    } catch {
      health = null;
    } finally {
      checking = false;
    }
  }

  async function switchProfile(event: Event) {
    const profileId = (event.currentTarget as HTMLSelectElement).value || null;
    if (profileId === runtimePreference?.preferred_profile_id) return;
    switching = true;
    try {
      await setPreferredRuntime(profileId);
      await onChanged();
    } finally {
      switching = false;
    }
  }
</script>

<Card.Root class="gap-3 p-4">
  <div class="flex flex-wrap items-center justify-between gap-3">
    <div class="flex flex-wrap items-center gap-2">
      <h3 class="text-sm font-semibold">Preferred runtime</h3>
      {#if binding}
        <RuntimeIdentity presentation={presentRuntimeBinding(binding)} compact />
        {#if health}
          <Badge variant={health.reachable ? "default" : "destructive"}>
            {health.reachable ? "Reachable" : "Unreachable"}
          </Badge>
        {/if}
        {#if health?.api_version}
          <span class="text-xs text-muted-foreground">Docker API {health.api_version}</span>
        {/if}
      {:else}
        <Badge variant="outline">Loading policy</Badge>
      {/if}
    </div>
    <div class="flex min-w-0 flex-wrap items-center justify-end gap-2">
      {#if profiles.length > 0}
        <div class="relative min-w-72 max-w-full flex-1 sm:flex-none">
          <select
            class="h-9 w-full appearance-none rounded-md border bg-background bg-none pr-9 pl-3 text-sm leading-5"
            disabled={switching || !connected}
            value={runtimePreference?.preferred_profile_id ?? ""}
            onchange={switchProfile}
            aria-label="Set preferred runtime"
          >
            <option value="">Use platform default</option>
            {#if binding?.state === "missing" && binding.profile_id}
              <option value={binding.profile_id} disabled>{binding.display_name}</option>
            {/if}
            {#each profiles as profile (profile.id)}
              <option value={profile.id} disabled={!profile.management.can_select}>
                {profile.display_name} ({profile.process.state.replace("_", " ")})
              </option>
            {/each}
          </select>
          <ChevronDown
            class="pointer-events-none absolute top-1/2 right-2 size-4 -translate-y-1/2 text-muted-foreground"
          />
        </div>
      {/if}
      <Button
        size="sm"
        variant="outline"
        disabled={checking || !connected || !activeEngineId}
        onclick={() => recheck()}
      >
        <RefreshCw />
        Recheck
      </Button>
      <Button size="sm" variant="outline" onclick={onManageRuntimes}>
        <Settings2 />
        Manage runtimes
      </Button>
      <Button
        size="sm"
        variant="destructive"
        disabled={!connected || !activeEngineId}
        onclick={() => (pruneDialogOpen = true)}
      >
        <Trash2 />
        Prune
      </Button>
    </div>
  </div>

  {#if health?.error}
    <p class="text-xs text-destructive">{health.error}</p>
  {/if}
  {#if binding && binding.state !== "ready" && binding.state !== "unconfigured"}
    <p class="text-xs text-destructive">
      This configured runtime is unavailable. Studio will not switch engines automatically.
    </p>
  {/if}
</Card.Root>

{#if activeEngineId}
  <PruneDialog
    engineId={activeEngineId}
    runtimeName={selected
      ? `${selected.display_name} (${selected.provider_runtime_key})`
      : binding?.display_name}
    bind:open={pruneDialogOpen}
    oncompleted={() => recheck()}
  />
{/if}
