<script lang="ts">
  import * as Sidebar from "$lib/components/ui/sidebar/index.js";
  import { Badge } from "$lib/components/ui/badge/index.js";
  import { FileText, LayoutGrid, Server, Settings } from "@lucide/svelte";
  import type { HealthState } from "$lib/daemon/daemon-state.svelte";

  type NavItem = {
    label: string;
    description: string;
    icon: typeof LayoutGrid;
    active?: boolean;
  };

  const navItems: NavItem[] = [
    {
      label: "Projects",
      description: "Imported Compose workspaces",
      icon: LayoutGrid,
      active: true,
    },
    { label: "Reports", description: "Analysis and review history", icon: FileText },
    { label: "Engines", description: "Docker-compatible runtimes", icon: Server },
    { label: "Settings", description: "Studio and daemon preferences", icon: Settings },
  ];

  let { healthState }: { healthState: HealthState } = $props();
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
              <Sidebar.MenuButton isActive={item.active} tooltipContent={item.description}>
                <item.icon />
                <span>{item.label}</span>
              </Sidebar.MenuButton>
            </Sidebar.MenuItem>
          {/each}
        </Sidebar.Menu>
      </Sidebar.GroupContent>
    </Sidebar.Group>
  </Sidebar.Content>

  <Sidebar.Footer>
    <div
      class="flex items-center gap-2 rounded-md border p-2 group-data-[collapsible=icon]:hidden"
    >
      <Badge variant={healthState.kind === "connected" ? "default" : "outline"}>
        {healthState.label}
      </Badge>
      <p class="truncate text-xs text-muted-foreground">{healthState.detail}</p>
    </div>
  </Sidebar.Footer>
</Sidebar.Root>
