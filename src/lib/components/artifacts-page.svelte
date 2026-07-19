<script lang="ts">
  import * as Tabs from "$lib/components/ui/tabs/index.js";
  import { Badge } from "$lib/components/ui/badge/index.js";
  import { Boxes } from "@lucide/svelte";
  import { resolveActiveEngineId } from "$lib/engine-identity";
  import type { RuntimeProfile, StudioProject } from "$lib/daemon/client";
  import ArtifactsContainersTab from "./artifacts-containers-tab.svelte";
  import ArtifactsImagesTab from "./artifacts-images-tab.svelte";
  import ArtifactsBuildsTab from "./artifacts-builds-tab.svelte";
  import ArtifactsBuildCacheTab from "./artifacts-build-cache-tab.svelte";
  import ArtifactsRegistryTab from "./artifacts-registry-tab.svelte";

  let {
    profiles,
    connected,
    projects,
  }: { profiles: RuntimeProfile[]; connected: boolean; projects: StudioProject[] } = $props();

  const selected = $derived(profiles.find((profile) => profile.is_selected) ?? null);
  // Never hardcode the legacy id here — this is the same resolution every
  // artifact request uses, so the header always names the engine the data
  // actually came from.
  const engineId = $derived(resolveActiveEngineId(selected?.id));
</script>

<div class="flex flex-col gap-4">
  <div class="flex flex-wrap items-center gap-2">
    <Boxes class="size-4 text-muted-foreground" />
    <h3 class="text-lg font-semibold">Artifacts</h3>
    <span class="text-sm text-muted-foreground">on</span>
    {#if selected}
      <span class="text-sm font-medium">{selected.display_name}</span>
      <Badge variant={selected.runtime_class === "built_in" ? "default" : "secondary"}>
        {selected.runtime_class === "built_in" ? "Built-in" : "External"}
      </Badge>
    {:else}
      <Badge variant="outline">Platform default (Local Docker)</Badge>
    {/if}
  </div>
  <p class="max-w-2xl text-sm text-muted-foreground">
    Inventory and image/build actions for the engine behind the runtime above. Switch runtimes
    from the Runtime page to inspect a different engine.
  </p>

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
</div>
