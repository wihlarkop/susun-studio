<script lang="ts">
  import * as Dialog from "$lib/components/ui/dialog/index.js";
  import { Button } from "$lib/components/ui/button/index.js";
  import { deleteProject } from "$lib/daemon/client";
  import type { StudioProject } from "$lib/daemon/client";

  let {
    project,
    open = $bindable(false),
    onRemoved,
  }: {
    project: StudioProject;
    open?: boolean;
    onRemoved: () => void;
  } = $props();

  let removing = $state(false);
  let errorMessage = $state<string | null>(null);

  async function confirmRemove() {
    removing = true;
    errorMessage = null;
    try {
      await deleteProject(project.id);
      open = false;
      onRemoved();
    } catch (error) {
      errorMessage = error instanceof Error ? error.message : String(error);
    } finally {
      removing = false;
    }
  }
</script>

<Dialog.Root bind:open>
  <Dialog.Content class="sm:max-w-md">
    <Dialog.Header>
      <Dialog.Title>Remove {project.name} from Studio?</Dialog.Title>
      <Dialog.Description>
        This only removes it from Studio's tracked project list. Nothing in Docker is touched —
        containers keep running if they're up, and you can re-import this project later from the
        same Compose files.
      </Dialog.Description>
    </Dialog.Header>
    {#if errorMessage}
      <p class="text-destructive text-sm">{errorMessage}</p>
    {/if}
    <Dialog.Footer>
      <Button type="button" variant="outline" onclick={() => (open = false)}>Cancel</Button>
      <Button type="button" variant="destructive" disabled={removing} onclick={confirmRemove}>
        {removing ? "Removing…" : "Remove"}
      </Button>
    </Dialog.Footer>
  </Dialog.Content>
</Dialog.Root>
