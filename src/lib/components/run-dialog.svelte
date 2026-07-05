<script lang="ts">
  import * as Dialog from "$lib/components/ui/dialog/index.js";
  import { Button } from "$lib/components/ui/button/index.js";
  import { Input } from "$lib/components/ui/input/index.js";
  import { openRunStream } from "$lib/daemon/client";
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
  let lines = $state<{ tone: "out" | "err" | "status"; text: string }[]>([]);
  let running = $state(false);
  let source: EventSource | null = null;

  function append(tone: "out" | "err" | "status", text: string) {
    lines = [...lines.slice(-(MAX_LINES - 1)), { tone, text }];
  }

  async function start() {
    if (running) return;
    const parts = command.trim().split(/\s+/).filter(Boolean);
    lines = [];
    running = true;
    try {
      source = await openRunStream(project.id, service, {
        command: parts.length > 0 ? parts : undefined,
      });
      source.onmessage = (message) => {
        const event = JSON.parse(message.data);
        switch (event.kind) {
          case "created":
            append("status", `container created (${event.container_id?.slice(0, 12)})`);
            break;
          case "output":
            append(event.source === "stderr" ? "err" : "out", event.line);
            break;
          case "exited":
            append("status", `exited with code ${event.exit_code}`);
            break;
          case "removed":
            append("status", "container removed");
            break;
          case "error":
            append("err", event.message ?? "stream error");
            break;
          case "end":
            stop();
            break;
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
      <Dialog.Title>Run in {service}</Dialog.Title>
      <Dialog.Description>
        Starts a disposable one-off container using the service's image, env, volumes, and
        networks (compose `run --rm` semantics; removed automatically when it exits). No
        published ports and no config/secret mounts in this version. Leave the command empty to
        use the service's default command.
      </Dialog.Description>
    </Dialog.Header>
    <form
      class="flex gap-2"
      onsubmit={(event) => {
        event.preventDefault();
        start();
      }}
    >
      <Input bind:value={command} placeholder="(optional) command to run" disabled={running} />
      <Button type="submit" disabled={running}>Run</Button>
      {#if running}
        <Button type="button" variant="outline" onclick={stop}>Stop watching</Button>
      {/if}
    </form>
    <div class="bg-muted/40 max-h-80 overflow-y-auto rounded-md border p-3 font-mono text-xs">
      {#if lines.length === 0}
        <p class="text-muted-foreground">Output appears here.</p>
      {:else}
        {#each lines as line, index (index)}
          <div
            class={line.tone === "err"
              ? "text-destructive"
              : line.tone === "status"
                ? "text-muted-foreground"
                : ""}
          >
            {line.text}
          </div>
        {/each}
      {/if}
    </div>
  </Dialog.Content>
</Dialog.Root>
