<script lang="ts">
  import * as Dialog from "$lib/components/ui/dialog/index.js";
  import { Button } from "$lib/components/ui/button/index.js";
  import { Input } from "$lib/components/ui/input/index.js";
  import type { ImportProjectRequest, ImportProjectResponse } from "$lib/daemon/client";

  let {
    open = $bindable(false),
    connected,
    onImport,
  }: {
    open?: boolean;
    connected: boolean;
    onImport: (request: ImportProjectRequest) => Promise<ImportProjectResponse>;
  } = $props();

  let filesInput = $state("");
  let envFileInput = $state("");
  let projectNameInput = $state("");
  let profilesInput = $state("");
  let submitting = $state(false);
  let errorMessage = $state<string | null>(null);
  let lastResult = $state<ImportProjectResponse | null>(null);

  function resetForm() {
    filesInput = "";
    envFileInput = "";
    projectNameInput = "";
    profilesInput = "";
    errorMessage = null;
    lastResult = null;
  }

  async function handleSubmit(event: SubmitEvent) {
    event.preventDefault();

    const files = filesInput
      .split(",")
      .map((value) => value.trim())
      .filter((value) => value.length > 0);

    if (files.length === 0) {
      errorMessage = "At least one compose file path is required.";
      return;
    }

    const profiles = profilesInput
      .split(",")
      .map((value) => value.trim())
      .filter((value) => value.length > 0);

    submitting = true;
    errorMessage = null;

    try {
      const response = await onImport({
        files,
        env_file: envFileInput.trim() || null,
        project_name: projectNameInput.trim() || null,
        profiles,
      });
      lastResult = response;
      if (response.project) {
        open = false;
        resetForm();
      }
    } catch (error) {
      errorMessage = error instanceof Error ? error.message : "Import failed";
    } finally {
      submitting = false;
    }
  }
</script>

<Dialog.Root bind:open>
  <Dialog.Content class="sm:max-w-lg">
    <Dialog.Header>
      <Dialog.Title>Import Compose Project</Dialog.Title>
      <Dialog.Description>
        Provide absolute paths to one or more Compose files. Additional files overlay the first.
      </Dialog.Description>
    </Dialog.Header>

    <form class="flex flex-col gap-3" onsubmit={handleSubmit}>
      <label class="flex flex-col gap-1 text-sm">
        <span class="font-medium">Compose files (comma-separated)</span>
        <Input bind:value={filesInput} placeholder="/path/to/compose.yaml" required />
      </label>

      <label class="flex flex-col gap-1 text-sm">
        <span class="font-medium">Env file (optional)</span>
        <Input bind:value={envFileInput} placeholder="/path/to/.env" />
      </label>

      <label class="flex flex-col gap-1 text-sm">
        <span class="font-medium">Project name override (optional)</span>
        <Input bind:value={projectNameInput} placeholder="my-project" />
      </label>

      <label class="flex flex-col gap-1 text-sm">
        <span class="font-medium">Profiles (optional, comma-separated)</span>
        <Input bind:value={profilesInput} placeholder="dev,debug" />
      </label>

      {#if errorMessage}
        <p class="text-sm text-destructive">{errorMessage}</p>
      {/if}

      {#if lastResult && !lastResult.project}
        <p class="text-sm text-destructive">
          Import produced no project ({lastResult.has_errors ? "errors" : "no data"} in
          diagnostics). Check the file paths and try again.
        </p>
      {/if}

      <Dialog.Footer>
        <Button type="submit" disabled={!connected || submitting}>
          {submitting ? "Importing..." : "Import"}
        </Button>
      </Dialog.Footer>
    </form>
  </Dialog.Content>
</Dialog.Root>
