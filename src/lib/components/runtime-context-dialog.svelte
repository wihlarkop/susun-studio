<script lang="ts">
  import * as Dialog from "$lib/components/ui/dialog/index.js";
  import { Button } from "$lib/components/ui/button/index.js";
  import {
    previewPreferredRuntime,
    previewProjectEngine,
    setPreferredRuntime,
    setProjectEngine,
  } from "$lib/daemon/client";
  import {
    acceptContextPreview,
    beginContextCommit,
    beginContextPreview,
    boundedContextReason,
    canCommitContextChange,
    cancelContextChange,
    createContextChangeState,
    isProjectImpactPreview,
    resolveContextCommit,
    type RuntimeContextTarget,
  } from "$lib/runtime/context-change";

  let {
    intent = null,
    open = $bindable(false),
    oncompleted,
  }: {
    intent?: RuntimeContextTarget | null;
    open?: boolean;
    oncompleted: () => void | Promise<void>;
  } = $props();

  let state = $state(createContextChangeState());
  let previewGeneration = 0;

  $effect(() => {
    const target = intent;
    if (!open || !target) return;
    const next = beginContextPreview(
      { ...createContextChangeState(), generation: previewGeneration },
      target,
    );
    previewGeneration = next.generation;
    state = next;
    const controller = new AbortController();
    const request = target.kind === "preference"
      ? previewPreferredRuntime(target.profileId, { signal: controller.signal })
      : previewProjectEngine(target.projectId, target.profileId, { signal: controller.signal });
    void request
      .then((preview) => {
        if (!controller.signal.aborted) {
          state = acceptContextPreview(state, next.generation, target, preview);
        }
      })
      .catch(() => {
        if (!controller.signal.aborted) {
          state = {
            ...state,
            phase: "failed",
            preview: null,
            message: boundedContextReason(null),
          };
        }
      });
    return () => controller.abort();
  });

  function setOpen(next: boolean) {
    open = next;
    if (!next) state = cancelContextChange(state);
  }

  async function commit() {
    const target = state.target;
    const preview = state.preview;
    if (!target || !preview || !canCommitContextChange(state)) return;
    state = beginContextCommit(state);
    try {
      if (target.kind === "preference") {
        await setPreferredRuntime(target.profileId, preview.impact_fingerprint);
      } else {
        await setProjectEngine(target.projectId, target.profileId, preview.impact_fingerprint);
      }
      state = resolveContextCommit(state, true);
      await oncompleted();
      open = false;
    } catch {
      state = resolveContextCommit(state, false);
    }
  }
</script>

<Dialog.Root bind:open={() => open, setOpen}>
  <Dialog.Content class="sm:max-w-xl">
    <Dialog.Header>
      <Dialog.Title>Change runtime context</Dialog.Title>
      <Dialog.Description>
        Review the affected projects and running work before applying this metadata change.
      </Dialog.Description>
    </Dialog.Header>
    <div class="grid gap-3 text-sm">
      {#if state.phase === "loading_preview"}
        <p class="text-muted-foreground">Preparing impact preview...</p>
      {:else if state.preview}
        <div class="grid gap-1 rounded-md border p-3">
            <span>Current: {state.preview.current.display_name}</span>
            <span>Target: {state.preview.target.display_name}</span>
          {#if !isProjectImpactPreview(state.preview)}
            <span class="text-muted-foreground">
              {state.preview.inheriting_project_count} inheriting project(s),
              {state.preview.explicitly_pinned_project_count} explicit pin(s).
            </span>
          {:else}
            <span class="text-muted-foreground">Only this project's explicit binding will change.</span>
          {/if}
          {#if state.preview.affected_active_jobs || state.preview.affected_active_watch_sessions}
            <span class="text-destructive">
              {state.preview.affected_active_jobs} running job(s) and
              {state.preview.affected_active_watch_sessions} watch session(s) would be redirected.
            </span>
          {/if}
          {#if !state.preview.change_allowed}
            <span class="text-destructive">{boundedContextReason(state.preview.reason_code)}</span>
          {/if}
        </div>
      {:else if state.message}
        <p class="text-destructive">{state.message}</p>
      {/if}
    </div>
    <Dialog.Footer>
      <Button variant="outline" onclick={() => setOpen(false)}>Cancel</Button>
      <Button disabled={!canCommitContextChange(state) || state.phase === "committing"} onclick={commit}>
        {state.phase === "committing" ? "Applying..." : "Apply change"}
      </Button>
    </Dialog.Footer>
  </Dialog.Content>
</Dialog.Root>
