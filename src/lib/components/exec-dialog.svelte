<script lang="ts">
  import * as Dialog from "$lib/components/ui/dialog/index.js";
  import { Button } from "$lib/components/ui/button/index.js";
  import { Input } from "$lib/components/ui/input/index.js";
  import { openExecStream } from "$lib/daemon/client";
  import type { StudioProject } from "$lib/daemon/client";

  let {
    project,
    service,
    open = $bindable(false),
  }: {
    project: StudioProject;
    service: string;
    open?: boolean;
  } = $props();

  const MAX_LINES = 500;
  let command = $state("");
  let lines = $state<{ tone: "out" | "err"; text: string }[]>([]);
  let running = $state(false);
  let source: EventSource | null = null;

  function append(tone: "out" | "err", text: string) {
    lines = [...lines.slice(-(MAX_LINES - 1)), { tone, text }];
  }

  async function start() {
    const parts = command.trim().split(/\s+/).filter(Boolean);
    if (parts.length === 0 || running) return;
    lines = [];
    running = true;
    try {
      source = await openExecStream(project.id, service, { command: parts });
      source.onmessage = (message) => {
        const event = JSON.parse(message.data);
        if (event.kind === "output") {
          append(event.source === "stderr" ? "err" : "out", event.line);
        } else if (event.kind === "error") {
          append("err", event.message ?? "stream error");
        } else if (event.kind === "end") {
          stop();
        }
      };
      source.onerror = () => stop();
    } catch (error) {
      append("err", error instanceof Error ? error.message : String(error));
      running = false;
    }
  }

  function stop() {
    source?.close();
    source = null;
    running = false;
  }

  $effect(() => {
    if (!open) stop();
  });
</script>

<Dialog.Root bind:open>
  <Dialog.Content class="sm:max-w-2xl">
    <Dialog.Header>
      <Dialog.Title>Exec in {service}</Dialog.Title>
      <Dialog.Description>
        Runs a one-shot command in the running container. Arguments split on spaces; no shell
        quoting.
      </Dialog.Description>
    </Dialog.Header>
    <form
      class="flex gap-2"
      onsubmit={(event) => {
        event.preventDefault();
        start();
      }}
    >
      <Input bind:value={command} placeholder="ls -la /" disabled={running} />
      <Button type="submit" disabled={running || command.trim() === ""}>Run</Button>
      {#if running}
        <Button type="button" variant="outline" onclick={stop}>Stop</Button>
      {/if}
    </form>
    <div class="bg-muted/40 max-h-80 overflow-y-auto rounded-md border p-3 font-mono text-xs">
      {#if lines.length === 0}
        <p class="text-muted-foreground">Output appears here.</p>
      {:else}
        {#each lines as line, index (index)}
          <div class={line.tone === "err" ? "text-destructive" : ""}>{line.text}</div>
        {/each}
      {/if}
    </div>
  </Dialog.Content>
</Dialog.Root>
