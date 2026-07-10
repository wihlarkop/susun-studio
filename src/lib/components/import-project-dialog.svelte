<script lang="ts">
  import * as Dialog from "$lib/components/ui/dialog/index.js";
  import { Button } from "$lib/components/ui/button/index.js";
  import { Input } from "$lib/components/ui/input/index.js";
  import { Badge } from "$lib/components/ui/badge/index.js";
  import { ChevronDown, FolderOpen, X } from "@lucide/svelte";
  import { displayPath } from "$lib/utils";
  import type {
    ImportProjectRequest,
    ImportProjectResponse,
    RuntimeProfile,
  } from "$lib/daemon/client";

  let {
    open = $bindable(false),
    connected,
    runtimeProfiles,
    onImport,
  }: {
    open?: boolean;
    connected: boolean;
    runtimeProfiles: RuntimeProfile[];
    onImport: (request: ImportProjectRequest) => Promise<ImportProjectResponse>;
  } = $props();

  const inTauri = typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;

  let files = $state<string[]>([]);
  let fileInput = $state("");
  let envFileInput = $state("");
  let projectNameInput = $state("");
  let profilesInput = $state("");
  let engineProfileId = $state("");
  let submitting = $state(false);
  let errorMessage = $state<string | null>(null);
  let lastResult = $state<ImportProjectResponse | null>(null);

  function resetForm() {
    files = [];
    fileInput = "";
    envFileInput = "";
    projectNameInput = "";
    profilesInput = "";
    engineProfileId = "";
    errorMessage = null;
    lastResult = null;
  }

  function addFile(path: string) {
    const trimmed = path.trim();
    if (trimmed.length > 0 && !files.includes(trimmed)) {
      files = [...files, trimmed];
    }
  }

  function addTypedFile() {
    addFile(fileInput);
    fileInput = "";
  }

  function removeFile(path: string) {
    files = files.filter((file) => file !== path);
  }

  async function browseComposeFiles() {
    const { open: openFileDialog } = await import("@tauri-apps/plugin-dialog");
    const selection = await openFileDialog({
      multiple: true,
      title: "Select Compose files",
      filters: [{ name: "Compose files", extensions: ["yaml", "yml"] }],
    });
    for (const path of selection ?? []) {
      addFile(path);
    }
  }

  async function browseEnvFile() {
    const { open: openFileDialog } = await import("@tauri-apps/plugin-dialog");
    const selection = await openFileDialog({ multiple: false, title: "Select env file" });
    if (selection) {
      envFileInput = selection;
    }
  }

  async function handleSubmit(event: SubmitEvent) {
    event.preventDefault();

    if (fileInput.trim().length > 0) {
      addTypedFile();
    }

    if (files.length === 0) {
      errorMessage = "At least one compose file is required.";
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
        runtime_profile_id: engineProfileId || null,
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
        Select one or more Compose files. Additional files overlay the first, like
        <code class="font-mono text-xs">docker compose -f a.yaml -f b.yaml</code>.
      </Dialog.Description>
    </Dialog.Header>

    <form class="flex flex-col gap-3" onsubmit={handleSubmit}>
      <div class="flex flex-col gap-1 text-sm">
        <span class="font-medium">Compose files</span>
        {#if files.length > 0}
          <ul class="flex flex-col gap-1">
            {#each files as file, index (file)}
              <li class="flex items-center gap-2 rounded-md border px-2 py-1">
                {#if index > 0}
                  <Badge variant="outline" class="shrink-0 text-xs">overlay</Badge>
                {/if}
                <span class="min-w-0 flex-1 truncate font-mono text-xs" title={file}>
                  {displayPath(file)}
                </span>
                <Button
                  type="button"
                  variant="ghost"
                  size="icon"
                  class="size-6 shrink-0"
                  aria-label={`Remove ${file}`}
                  onclick={() => removeFile(file)}
                >
                  <X />
                </Button>
              </li>
            {/each}
          </ul>
        {/if}
        <div class="flex gap-2">
          {#if inTauri}
            <Button type="button" variant="outline" class="shrink-0" onclick={browseComposeFiles}>
              <FolderOpen />
              Browse…
            </Button>
          {/if}
          <Input
            bind:value={fileInput}
            placeholder="/path/to/compose.yaml"
            onkeydown={(event) => {
              if (event.key === "Enter") {
                event.preventDefault();
                addTypedFile();
              }
            }}
          />
          <Button type="button" variant="secondary" class="shrink-0" onclick={addTypedFile}>
            Add
          </Button>
        </div>
      </div>

      <div class="flex flex-col gap-1 text-sm">
        <span class="font-medium">Env file (optional)</span>
        <div class="flex gap-2">
          {#if inTauri}
            <Button type="button" variant="outline" class="shrink-0" onclick={browseEnvFile}>
              <FolderOpen />
              Browse…
            </Button>
          {/if}
          <Input bind:value={envFileInput} placeholder="/path/to/.env" />
        </div>
      </div>

      <label class="flex flex-col gap-1 text-sm">
        <span class="font-medium">Project name override (optional)</span>
        <Input bind:value={projectNameInput} placeholder="my-project" />
      </label>

      <label class="flex flex-col gap-1 text-sm">
        <span class="font-medium">Profiles (optional, comma-separated)</span>
        <Input bind:value={profilesInput} placeholder="dev,debug" />
      </label>

      <label class="flex flex-col gap-1 text-sm">
        <span class="font-medium">Engine (optional)</span>
        <div class="relative">
          <select
            class="h-9 w-full appearance-none rounded-md border bg-background bg-none pr-8 pl-3 text-sm"
            bind:value={engineProfileId}
          >
            <option value="">Use active engine</option>
            {#each runtimeProfiles as profile (profile.id)}
              <option value={profile.id}>{profile.display_name}</option>
            {/each}
          </select>
          <ChevronDown
            class="pointer-events-none absolute top-1/2 right-2 size-4 -translate-y-1/2 text-muted-foreground"
          />
        </div>
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
        <Button type="button" variant="outline" onclick={() => (open = false)}>Cancel</Button>
        <Button type="submit" disabled={!connected || submitting}>
          {submitting ? "Importing…" : "Import"}
        </Button>
      </Dialog.Footer>
    </form>
  </Dialog.Content>
</Dialog.Root>
