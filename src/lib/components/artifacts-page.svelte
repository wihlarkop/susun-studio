<script lang="ts">
  import * as Tabs from "$lib/components/ui/tabs/index.js";
  import { Badge } from "$lib/components/ui/badge/index.js";
  import { Boxes } from "@lucide/svelte";
  import { resolveActiveEngineId } from "$lib/engine-identity";
  import type { RuntimePreference, StudioProject } from "$lib/daemon/client";
  import ArtifactsContainersTab from "./artifacts-containers-tab.svelte";
  import ArtifactsImagesTab from "./artifacts-images-tab.svelte";
  import ArtifactsBuildsTab from "./artifacts-builds-tab.svelte";
  import ArtifactsBuildCacheTab from "./artifacts-build-cache-tab.svelte";
  import ArtifactsRegistryTab from "./artifacts-registry-tab.svelte";
  import RuntimeIdentity from "./runtime-identity.svelte";
  import { presentRuntimeBinding } from "$lib/runtime/presentation";

  let {
    runtimePreference,
    connected,
    projects,
  }: {
    runtimePreference: RuntimePreference | undefined;
    connected: boolean;
    projects: StudioProject[];
  } = $props();

  const binding = $derived(runtimePreference?.binding ?? null);
  const engineId = $derived(binding ? resolveActiveEngineId(binding) : null);
</script>

<div class="flex flex-col gap-4">
  <div class="flex flex-wrap items-center gap-2">
    <Boxes class="size-4 text-muted-foreground" />
    <h3 class="text-lg font-semibold">Artifacts</h3>
    <span class="text-sm text-muted-foreground">on</span>
    {#if binding}
      <RuntimeIdentity presentation={presentRuntimeBinding(binding)} compact />
    {:else}
      <Badge variant="outline">Loading runtime policy</Badge>
    {/if}
  </div>
  <p class="max-w-2xl text-sm text-muted-foreground">
    Inventory and image/build actions are scoped to the runtime above. Studio never redirects
    artifact requests to a different engine when the configured runtime is unavailable.
  </p>

  {#if engineId}
    <Tabs.Root value="containers" class="w-full">
      <Tabs.List>
        <Tabs.Trigger value="containers">Containers</Tabs.Trigger>
        <Tabs.Trigger value="images">Images</Tabs.Trigger>
        <Tabs.Trigger value="builds">Builds</Tabs.Trigger>
        <Tabs.Trigger value="build-cache">Build cache</Tabs.Trigger>
        <Tabs.Trigger value="registry">Registry</Tabs.Trigger>
      </Tabs.List>
      <Tabs.Content value="containers" class="pt-4">
        <ArtifactsContainersTab {engineId} {connected} {projects} />
      </Tabs.Content>
      <Tabs.Content value="images" class="pt-4">
        <ArtifactsImagesTab {engineId} {connected} />
      </Tabs.Content>
      <Tabs.Content value="builds" class="pt-4">
        <ArtifactsBuildsTab {engineId} {connected} {projects} />
      </Tabs.Content>
      <Tabs.Content value="build-cache" class="pt-4">
        <ArtifactsBuildCacheTab {engineId} {connected} />
      </Tabs.Content>
      <Tabs.Content value="registry" class="pt-4">
        <ArtifactsRegistryTab {engineId} {connected} />
      </Tabs.Content>
    </Tabs.Root>
  {:else if binding}
    <div class="rounded-md border border-destructive/30 bg-destructive/5 p-3 text-sm text-destructive">
      {binding.display_name} is {binding.state}. Artifact actions are blocked until the configured
      runtime is available or the preference changes.
    </div>
  {/if}
</div>
