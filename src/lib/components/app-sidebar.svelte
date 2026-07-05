<script lang="ts">
  import * as Sidebar from "$lib/components/ui/sidebar/index.js";
  import { Badge } from "$lib/components/ui/badge/index.js";
  import { FileText, LayoutGrid, ListChecks, Server, Settings } from "@lucide/svelte";
  import { displayPath } from "$lib/utils";
  import type { StudioSettings } from "$lib/daemon/client";
  import type { HealthState } from "$lib/daemon/daemon-state.svelte";

  type View = "projects" | "jobs";

  type NavItem = {
    label: string;
    description: string;
    icon: typeof LayoutGrid;
    view?: View;
    planned?: boolean;
  };

  const navItems: NavItem[] = [
    {
      label: "Projects",
      description: "Imported Compose workspaces",
      icon: LayoutGrid,
      view: "projects",
    },
    {
      label: "Jobs",
      description: "Every job across every project",
      icon: ListChecks,
      view: "jobs",
    },
    {
      label: "Reports",
      description: "Analysis and review history (planned)",
      icon: FileText,
      planned: true,
    },
    {
      label: "Engines",
      description: "Docker-compatible runtimes (planned)",
      icon: Server,
      planned: true,
    },
    {
      label: "Settings",
      description: "Studio and daemon preferences (planned)",
      icon: Settings,
      planned: true,
    },
  ];

  let {
    healthState,
    settings,
    activeView,
    onNavigate,
  }: {
    healthState: HealthState;
    settings: StudioSettings | undefined;
    activeView: View;
    onNavigate: (view: View) => void;
  } = $props();
</script>

<Sidebar.Root collapsible="icon">
  <Sidebar.Header>
    <div class="flex items-center gap-3 px-2 py-1">
      <div
        class="flex size-9 items-center justify-center rounded-lg bg-primary font-bold text-primary-foreground"
      >
        S
      </div>
      <div class="group-data-[collapsible=icon]:hidden">
        <p class="text-sm leading-tight font-semibold">Susun Studio</p>
        <p class="text-xs text-muted-foreground">Daemon-first Compose workspace</p>
      </div>
    </div>
  </Sidebar.Header>

  <Sidebar.Content>
    <Sidebar.Group>
      <Sidebar.GroupLabel>Navigate</Sidebar.GroupLabel>
      <Sidebar.GroupContent>
        <Sidebar.Menu>
          {#each navItems as item (item.label)}
            <Sidebar.MenuItem>
              <Sidebar.MenuButton
                isActive={item.view !== undefined && item.view === activeView}
                tooltipContent={item.description}
                aria-disabled={item.planned}
                class={item.planned ? "opacity-60" : undefined}
                onclick={() => {
                  if (item.view) onNavigate(item.view);
                }}
              >
                <item.icon />
                <span>{item.label}</span>
                {#if item.planned}
                  <Badge variant="outline" class="ml-auto text-xs">Soon</Badge>
                {/if}
              </Sidebar.MenuButton>
            </Sidebar.MenuItem>
          {/each}
        </Sidebar.Menu>
      </Sidebar.GroupContent>
    </Sidebar.Group>
  </Sidebar.Content>

  <Sidebar.Footer>
    <div
      class="flex flex-col gap-2 rounded-md border p-2 group-data-[collapsible=icon]:hidden"
    >
      <div class="flex items-center gap-2">
        <Badge variant={healthState.kind === "connected" ? "default" : "outline"}>
          {healthState.label}
        </Badge>
        <p class="truncate text-xs text-muted-foreground">{healthState.detail}</p>
      </div>
      {#if settings?.default_project_root}
        <p class="truncate text-xs text-muted-foreground" title={settings.default_project_root}>
          Root: {displayPath(settings.default_project_root)}
        </p>
      {/if}
    </div>
  </Sidebar.Footer>
</Sidebar.Root>
