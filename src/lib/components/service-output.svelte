<script lang="ts">
  import { untrack } from "svelte";
  import { openLogStream, type LogStreamLine, type StudioProject } from "$lib/daemon/client";

  let {
    project,
    service,
    open,
    autoStartToken = 0,
  }: {
    project: StudioProject;
    service: string;
    open: boolean;
    autoStartToken?: number;
  } = $props();

  const MAX_LINES = 300;

  let lines = $state<LogStreamLine[]>([]);
  let following = $state(false);
  let errorMessage = $state<string | null>(null);
  let viewport = $state<HTMLDivElement | null>(null);
  let source: EventSource | null = null;

  function append(line: LogStreamLine) {
    lines = [...lines.slice(-(MAX_LINES - 1)), line];
    if (viewport) {
      queueMicrotask(() => {
        if (viewport) viewport.scrollTop = viewport.scrollHeight;
      });
    }
  }

  async function start() {
    stop();
    lines = [];
    errorMessage = null;
    following = true;
    try {
      source = await openLogStream(project.id, { service, tail: 100 });
      source.onmessage = (message) => {
        try {
          append(JSON.parse(message.data) as LogStreamLine);
        } catch {
          // ignore malformed frames
        }
      };
      source.onerror = () => {
        errorMessage = "Log stream disconnected.";
        stop();
      };
    } catch (error) {
      errorMessage = error instanceof Error ? error.message : String(error);
      following = false;
    }
  }

  function stop() {
    source?.close();
    source = null;
    following = false;
  }

  // Opening the panel starts following; closing it stops the stream — no
  // point holding an SSE connection open for a panel nobody's looking at.
  let wasOpen = false;
  $effect(() => {
    if (open && !wasOpen) {
      wasOpen = true;
      untrack(() => start());
    } else if (!open && wasOpen) {
      wasOpen = false;
      untrack(() => stop());
    }
  });

  // A service action (Start/Stop/Restart) bumps this to force a fresh
  // stream even when the panel was already open and following. Starts at 0,
  // not `autoStartToken`'s current value — the parent's token map always
  // begins empty, so this component only ever mounts fresh with a 0 token.
  let lastToken = 0;
  $effect(() => {
    if (autoStartToken !== lastToken) {
      lastToken = autoStartToken;
      if (open) untrack(() => start());
    }
  });

  $effect(() => {
    return () => stop();
  });
</script>

{#if open}
  <div class="mt-2 flex flex-col gap-1">
    {#if errorMessage}
      <p class="text-destructive text-xs">{errorMessage}</p>
    {/if}
    <div
      bind:this={viewport}
      class="bg-muted/40 h-36 overflow-y-auto rounded-md border p-2 font-mono text-xs"
    >
      {#if lines.length === 0}
        <p class="text-muted-foreground">
          {following ? "Waiting for output…" : "No output yet."}
        </p>
      {:else}
        {#each lines as line, index (index)}
          <div class={line.source === "stderr" ? "text-destructive" : ""}>{line.line}</div>
        {/each}
      {/if}
    </div>
  </div>
{/if}
