<script lang="ts">
  import * as Dialog from "$lib/components/ui/dialog/index.js";
  import { Badge } from "$lib/components/ui/badge/index.js";
  import { Button } from "$lib/components/ui/button/index.js";
  import {
    commitRuntimeDestructiveOperation,
    previewRuntimeDestructiveOperation,
    type RuntimeDestructiveAction,
    type RuntimeDestructiveCommitResult,
    type RuntimeDestructivePreview,
    type RuntimeProfile,
  } from "$lib/daemon/client";
  import { RefreshCw, ShieldCheck } from "@lucide/svelte";

  let {
    profile,
    open = $bindable(false),
  }: {
    profile: RuntimeProfile | null;
    open?: boolean;
  } = $props();

  let action = $state<RuntimeDestructiveAction>("repair");
  let preview = $state<RuntimeDestructivePreview | null>(null);
  let commitResult = $state<RuntimeDestructiveCommitResult | null>(null);
  let busy = $state(false);
  let errorMessage = $state<string | null>(null);

  async function inspect() {
    if (!profile) return;
    busy = true;
    errorMessage = null;
    commitResult = null;
    try {
      preview = await previewRuntimeDestructiveOperation(profile.id, action);
    } catch (error) {
      errorMessage = error instanceof Error ? error.message : String(error);
    } finally {
      busy = false;
    }
  }

  async function commit() {
    if (!preview?.allowed || !preview.plan_id) return;
    busy = true;
    errorMessage = null;
    try {
      commitResult = await commitRuntimeDestructiveOperation(preview.plan_id);
      // The single-use plan is now spent; a fresh preview is required to retry.
      preview = null;
    } catch (error) {
      errorMessage = error instanceof Error ? error.message : String(error);
    } finally {
      busy = false;
    }
  }
</script>

<Dialog.Root bind:open>
  <Dialog.Content class="max-h-[85vh] overflow-y-auto sm:max-w-xl">
    <Dialog.Header>
      <Dialog.Title>Runtime data scope</Dialog.Title>
      <Dialog.Description>{profile?.display_name ?? "Runtime"}</Dialog.Description>
    </Dialog.Header>

    <div class="grid gap-4">
      <label class="grid gap-1 text-sm font-medium">
        Operation
        <select class="h-9 rounded-md border bg-background px-3 text-sm" bind:value={action} onchange={() => (preview = null)}>
          <option value="repair">Repair</option>
          <option value="reset_engine_data">Reset engine data</option>
          <option value="remove_built_in_runtime">Remove Susun Runtime</option>
        </select>
      </label>

      {#if preview}
        <div class="grid gap-3">
          <div class="flex flex-wrap items-center gap-2">
            <Badge variant={preview.allowed ? "default" : "destructive"}>
              {preview.allowed ? "Ownership verified" : "Blocked"}
            </Badge>
            <Badge variant="outline">Fresh preview required before execution</Badge>
          </div>
          {#if preview.blocker}<p class="text-sm text-destructive">{preview.blocker}</p>{/if}
          <div class="divide-y rounded-md border">
            {#each preview.affected as item (item.category)}
              <div class="grid grid-cols-[minmax(0,1fr)_auto] gap-3 p-3 text-sm">
                <div class="min-w-0">
                  <div class="font-medium">{item.category.replaceAll("_", " ")}</div>
                  <div class="text-xs text-muted-foreground">{item.effect.replaceAll("_", " ")}</div>
                </div>
                <Badge variant={item.exactness === "exact" ? "secondary" : "outline"}>
                  {item.count ?? "Unknown"}
                </Badge>
              </div>
            {/each}
          </div>
          <p class="text-xs text-muted-foreground">Preserved: {preview.preserved.join(", ")}.</p>
        </div>
      {/if}
      {#if commitResult}
        <div class="grid gap-2 rounded-md border border-emerald-500/40 bg-emerald-500/5 p-3 text-sm">
          <div class="flex items-center gap-2">
            <ShieldCheck class="size-4 text-emerald-600" />
            <span class="font-medium">Ownership and state verified</span>
          </div>
          <p class="text-muted-foreground">{commitResult.message}</p>
          {#each commitResult.next_steps as step}
            <p class="text-xs text-muted-foreground">{step}</p>
          {/each}
        </div>
      {/if}
      {#if errorMessage}<p class="text-sm text-destructive">{errorMessage}</p>{/if}
    </div>

    <Dialog.Footer>
      <Button variant="outline" onclick={() => (open = false)}>Close</Button>
      <Button variant="outline" disabled={busy || !profile} onclick={inspect}>
        <RefreshCw /> {busy ? "Inspecting" : "Preview scope"}
      </Button>
      {#if preview?.allowed && preview.plan_id}
        <Button variant="destructive" disabled={busy} onclick={commit}>
          Verify &amp; continue
        </Button>
      {/if}
    </Dialog.Footer>
  </Dialog.Content>
</Dialog.Root>
