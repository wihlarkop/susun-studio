<script lang="ts">
  import { untrack } from "svelte";
  import { Button } from "$lib/components/ui/button/index.js";
  import { Input } from "$lib/components/ui/input/index.js";
  import { ChevronDown } from "@lucide/svelte";
  import { openLogStream, type LogStreamLine, type StudioProject } from "$lib/daemon/client";

  let {
    project,
    autoStartToken = 0,
  }: { project: StudioProject | null; autoStartToken?: number } = $props();

  const MAX_LINES = 2000;
  const CHART_TONE_CLASSES = [
    "text-chart-1",
    "text-chart-2",
    "text-chart-3",
    "text-chart-4",
    "text-chart-5",
  ];

  let following = $state(false);
  let paused = $state(false);
  let lines = $state<LogStreamLine[]>([]);
  let pending = $state<LogStreamLine[]>([]);
  let filter = $state("");
  let service = $state<string>("");
  let tail = $state(200);
  let source: EventSource | null = null;
  let viewport = $state<HTMLDivElement | null>(null);
  let pinned = $state(true);
  let errorMessage = $state<string | null>(null);

  const services = $derived((project?.summary?.services ?? []).map((s) => s.name));

  function append(line: LogStreamLine) {
    if (paused) {
      pending = [...pending.slice(-(MAX_LINES - 1)), line];
      return;
    }
    lines = [...lines.slice(-(MAX_LINES - 1)), line];
    if (pinned && viewport) {
      queueMicrotask(() => {
        if (viewport) viewport.scrollTop = viewport.scrollHeight;
      });
    }
  }

  function resume() {
    paused = false;
    lines = [...lines, ...pending].slice(-MAX_LINES);
    pending = [];
  }

  async function start() {
    if (!project || following) return;
    stop();
    lines = [];
    pending = [];
    errorMessage = null;
    following = true;
    try {
      source = await openLogStream(project.id, {
        service: service || undefined,
        tail,
      });
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

  function handleScroll() {
    if (!viewport) return;
    pinned = viewport.scrollTop + viewport.clientHeight >= viewport.scrollHeight - 24;
  }

  const visible = $derived(
    filter.trim() === ""
      ? lines
      : lines.filter((line) => line.line.toLowerCase().includes(filter.toLowerCase())),
  );

  function toneFor(serviceName: string): string {
    let hash = 0;
    for (const char of serviceName) hash = (hash * 31 + char.charCodeAt(0)) >>> 0;
    return CHART_TONE_CLASSES[hash % CHART_TONE_CLASSES.length];
  }

  function exportText(): string {
    return visible.map((line) => `[${line.service}] ${line.line}`).join("\n");
  }

  function copyLogs() {
    void navigator.clipboard.writeText(exportText());
  }

  function downloadLogs() {
    const blob = new Blob([exportText()], { type: "text/plain" });
    const url = URL.createObjectURL(blob);
    const anchor = document.createElement("a");
    anchor.href = url;
    anchor.download = `${project?.name ?? "project"}-logs.txt`;
    anchor.click();
    URL.revokeObjectURL(url);
  }

  $effect(() => {
    return () => stop();
  });

  // Forces an immediate all-services follow when the parent bumps this —
  // e.g. right after a whole-project job finishes — without requiring a
  // manual click on Follow. `untrack` keeps this effect keyed only on
  // `autoStartToken` changing, not on start()'s internal state reads.
  // `lastToken` starts at a literal 0, not `autoStartToken`'s current value:
  // this component is destroyed/remounted whenever the parent's `{#if
  // showLogs}` toggles closed then open again, and if a job finished while
  // hidden the fresh instance mounts with autoStartToken already bumped —
  // reading the prop here would capture that bumped value as the "last
  // seen" one and silently skip the auto-start this remount is supposed to
  // trigger.
  let lastToken = 0;
  $effect(() => {
    if (autoStartToken !== lastToken) {
      lastToken = autoStartToken;
      untrack(() => {
        service = "";
        start();
      });
    }
  });
</script>

{#if !project}
  <p class="text-muted-foreground text-sm">Select a project to view logs.</p>
{:else}
  <div class="flex flex-col gap-3">
    <div class="flex flex-wrap items-center gap-2">
      <div class="relative">
        <select
          bind:value={service}
          class="border-input rounded-md border bg-transparent bg-none py-1 pr-7 pl-2 text-sm appearance-none"
        >
          <option value="">All services</option>
          {#each services as name (name)}
            <option value={name}>{name}</option>
          {/each}
        </select>
        <ChevronDown
          class="text-muted-foreground pointer-events-none absolute top-1/2 right-2 size-3.5 -translate-y-1/2"
        />
      </div>
      <div class="relative">
        <select
          bind:value={tail}
          class="border-input rounded-md border bg-transparent bg-none py-1 pr-7 pl-2 text-sm appearance-none"
        >
          <option value={100}>tail 100</option>
          <option value={200}>tail 200</option>
          <option value={1000}>tail 1000</option>
        </select>
        <ChevronDown
          class="text-muted-foreground pointer-events-none absolute top-1/2 right-2 size-3.5 -translate-y-1/2"
        />
      </div>
      {#if following}
        <Button size="sm" variant="outline" onclick={stop}>Stop</Button>
      {:else}
        <Button size="sm" onclick={start}>Follow</Button>
      {/if}
      <Button size="sm" variant="outline" disabled={!following} onclick={() => (paused = !paused)}>
        {paused ? "Resume" : "Pause"}
      </Button>
      {#if paused}
        <Button size="sm" variant="ghost" onclick={resume}>Flush ({pending.length})</Button>
      {/if}
      <Input bind:value={filter} placeholder="Filter…" class="w-40" />
      <Button size="sm" variant="ghost" disabled={visible.length === 0} onclick={copyLogs}>
        Copy
      </Button>
      <Button size="sm" variant="ghost" disabled={visible.length === 0} onclick={downloadLogs}>
        Export
      </Button>
      <span class="text-muted-foreground ml-auto text-xs">
        {visible.length} / {lines.length}
      </span>
    </div>

    {#if errorMessage}
      <p class="text-destructive text-sm">{errorMessage}</p>
    {/if}

    <div
      bind:this={viewport}
      onscroll={handleScroll}
      class="bg-muted/30 h-96 overflow-y-auto rounded-md border p-3 font-mono text-xs"
    >
      {#if visible.length === 0}
        <p class="text-muted-foreground">No log lines yet. Click Follow to start streaming.</p>
      {:else}
        {#each visible as line, index (index)}
          <div class={line.source === "stderr" ? "text-destructive" : ""}>
            <span class={toneFor(line.service)}>[{line.service}]</span>
            {line.line}
          </div>
        {/each}
      {/if}
    </div>
  </div>
{/if}
