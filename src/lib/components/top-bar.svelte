<script lang="ts">
  import * as Sidebar from "$lib/components/ui/sidebar/index.js";
  import * as Tooltip from "$lib/components/ui/tooltip/index.js";
  import { Badge } from "$lib/components/ui/badge/index.js";
  import { Button } from "$lib/components/ui/button/index.js";
  import { Separator } from "$lib/components/ui/separator/index.js";
  import { Moon, Plus, Settings, Sun } from "@lucide/svelte";
  import { toggleMode } from "mode-watcher";
  import type { HealthState } from "$lib/daemon/daemon-state.svelte";

  let {
    healthState,
    title,
    onImportClick,
    onOpenSettings,
  }: {
    healthState: HealthState;
    title: string;
    onImportClick: () => void;
    onOpenSettings: () => void;
  } = $props();

  const connected = $derived(healthState.kind === "connected");
</script>

<header class="flex items-center justify-between gap-4">
  <div class="flex items-center gap-3">
    <Sidebar.Trigger />
    <Separator orientation="vertical" class="h-6" />
    <h2 class="text-2xl leading-tight font-semibold">{title}</h2>
    <Tooltip.Provider>
      <Tooltip.Root>
        <Tooltip.Trigger>
          <Badge variant={connected ? "default" : "outline"}>{healthState.label}</Badge>
        </Tooltip.Trigger>
        <Tooltip.Content>{healthState.detail}</Tooltip.Content>
      </Tooltip.Root>
    </Tooltip.Provider>
  </div>
  <div class="flex items-center gap-2">
    <Button
      variant="ghost"
      size="icon"
      aria-label="Open settings"
      title="Open settings"
      onclick={onOpenSettings}
    >
      <Settings />
    </Button>
    <Button variant="ghost" size="icon" aria-label="Toggle theme" onclick={toggleMode}>
      <Sun class="dark:hidden" />
      <Moon class="hidden dark:block" />
    </Button>
    <Button disabled={!connected} onclick={onImportClick} title="Ctrl+I">
      <Plus />
      Import Project
    </Button>
  </div>
</header>
