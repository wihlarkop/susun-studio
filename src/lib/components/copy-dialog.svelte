<script lang="ts">
  import * as Dialog from "$lib/components/ui/dialog/index.js";
  import { Button } from "$lib/components/ui/button/index.js";
  import { Input } from "$lib/components/ui/input/index.js";
  import { FolderOpen } from "@lucide/svelte";
  import { copyService } from "$lib/daemon/client";
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

  const inTauri = typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;

  let direction = $state<"to_container" | "from_container">("to_container");
  let hostPath = $state("");
  let containerPath = $state("");
  let submitting = $state(false);
  let result = $state<string | null>(null);
  let errorMessage = $state<string | null>(null);

  async function browse() {
    const { open: openFileDialog } = await import("@tauri-apps/plugin-dialog");
    const selection = await openFileDialog({
      directory: direction === "from_container",
      multiple: false,
      title: direction === "to_container" ? "Select file to copy" : "Select destination folder",
    });
    if (selection) {
      hostPath = selection;
    }
  }

  async function submit(event: SubmitEvent) {
    event.preventDefault();
    submitting = true;
    errorMessage = null;
    result = null;
    try {
      await copyService(project.id, service, {
        direction,
        host_path: hostPath,
        container_path: containerPath,
      });
      result = "Copy complete.";
    } catch (error) {
      errorMessage = error instanceof Error ? error.message : String(error);
    } finally {
      submitting = false;
    }
  }
</script>

<Dialog.Root bind:open>
  <Dialog.Content class="sm:max-w-lg">
    <Dialog.Header>
      <Dialog.Title>Copy files — {service}</Dialog.Title>
      <Dialog.Description>
        Copies one file into a container directory (max 64 MiB), or extracts a container path
        into a host directory.
      </Dialog.Description>
    </Dialog.Header>
    <form class="flex flex-col gap-3" onsubmit={submit}>
      <div class="flex gap-4 text-sm">
        <label class="flex items-center gap-2">
          <input type="radio" bind:group={direction} value="to_container" />
          Host → container
        </label>
        <label class="flex items-center gap-2">
          <input type="radio" bind:group={direction} value="from_container" />
          Container → host
        </label>
      </div>

      <label class="flex flex-col gap-1 text-sm">
        <span class="font-medium">
          {direction === "to_container" ? "Host file" : "Host destination folder"}
        </span>
        <div class="flex gap-2">
          {#if inTauri}
            <Button type="button" variant="outline" class="shrink-0" onclick={browse}>
              <FolderOpen />
              Browse…
            </Button>
          {/if}
          <Input bind:value={hostPath} placeholder="/path/on/host" />
        </div>
      </label>

      <label class="flex flex-col gap-1 text-sm">
        <span class="font-medium">
          {direction === "to_container" ? "Container destination directory" : "Container path"}
        </span>
        <Input bind:value={containerPath} placeholder="/path/in/container" />
      </label>

      {#if errorMessage}
        <p class="text-sm text-destructive">{errorMessage}</p>
      {/if}
      {#if result}
        <p class="text-success text-sm">{result}</p>
      {/if}

      <Dialog.Footer>
        <Button type="button" variant="outline" onclick={() => (open = false)}>Close</Button>
        <Button type="submit" disabled={submitting || !hostPath || !containerPath}>
          {submitting ? "Copying…" : "Copy"}
        </Button>
      </Dialog.Footer>
    </form>
  </Dialog.Content>
</Dialog.Root>
