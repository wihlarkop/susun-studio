<script lang="ts">
  import { Skeleton } from "$lib/components/ui/skeleton/index.js";
  import { Button } from "$lib/components/ui/button/index.js";
  import { AlertCircle, PlugZap, RefreshCw, ShieldOff } from "@lucide/svelte";
  import { capabilityLabel } from "$lib/artifacts/capability";
  import type { ArtifactViewState } from "$lib/artifacts/workspace-state";

  // Handles only the "nothing to show alongside this" states. Callers own
  // rendering their own data for "ready", and a small inline notice for
  // "refreshing"/"stale" next to that data — this never replaces data that's
  // already on screen.
  let {
    state,
    itemNoun = "items",
    onRetry,
  }: {
    state: ArtifactViewState;
    itemNoun?: string;
    onRetry?: () => void;
  } = $props();
</script>

{#if state.kind === "disconnected"}
  <p class="text-sm text-muted-foreground">Connect to the daemon to load {itemNoun}.</p>
{:else if state.kind === "loading"}
  <div class="flex flex-col gap-2">
    <Skeleton class="h-9 w-full" />
    <Skeleton class="h-9 w-full" />
    <Skeleton class="h-9 w-full" />
  </div>
{:else if state.kind === "unreachable"}
  <div
    class="flex flex-col items-start gap-2 rounded-md border border-destructive/30 bg-destructive/5 p-3 text-sm"
  >
    <div class="flex items-center gap-2 text-destructive">
      <PlugZap class="size-4" />
      <span class="font-medium">Engine unreachable</span>
    </div>
    <p class="text-muted-foreground">{state.error.message}</p>
    {#if onRetry}
      <Button size="sm" variant="outline" onclick={onRetry}>
        <RefreshCw />
        Retry
      </Button>
    {/if}
  </div>
{:else if state.kind === "request-error"}
  <div
    class="flex flex-col items-start gap-2 rounded-md border border-destructive/30 bg-destructive/5 p-3 text-sm"
  >
    <div class="flex items-center gap-2 text-destructive">
      <AlertCircle class="size-4" />
      <span class="font-medium">Request failed</span>
    </div>
    <p class="text-muted-foreground">{state.error.message}</p>
    {#if onRetry}
      <Button size="sm" variant="outline" onclick={onRetry}>
        <RefreshCw />
        Retry
      </Button>
    {/if}
  </div>
{:else if state.kind === "unsupported"}
  <div class="flex items-center gap-2 rounded-md border p-3 text-sm text-muted-foreground">
    <ShieldOff class="size-4 shrink-0" />
    <span>{capabilityLabel(state.capability)} on this engine.</span>
  </div>
{:else if state.kind === "empty"}
  <p class="rounded-md border p-3 text-sm text-muted-foreground">
    No {itemNoun} found on this engine right now.
  </p>
{/if}
