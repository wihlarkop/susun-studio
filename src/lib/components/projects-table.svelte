<script lang="ts">
  import * as Table from "$lib/components/ui/table/index.js";
  import * as Card from "$lib/components/ui/card/index.js";
  import { Badge } from "$lib/components/ui/badge/index.js";
  import { cn } from "$lib/utils";
  import type { StudioProject } from "$lib/daemon/client";

  let {
    projects,
    workspaceDetail,
    selectedId,
    onSelect,
  }: {
    projects: StudioProject[];
    workspaceDetail: string;
    selectedId: string | null;
    onSelect: (project: StudioProject) => void;
  } = $props();
</script>

<div class="space-y-2">
  <div>
    <h3 class="text-lg font-semibold">Workspace Projects</h3>
    <p class="text-sm text-muted-foreground">{workspaceDetail}</p>
  </div>

  <Card.Root class="p-0">
    <Table.Root>
      <Table.Header>
        <Table.Row>
          <Table.Head>Name</Table.Head>
          <Table.Head>Path</Table.Head>
          <Table.Head>Services</Table.Head>
          <Table.Head>Status</Table.Head>
        </Table.Row>
      </Table.Header>
      <Table.Body>
        {#if projects.length > 0}
          {#each projects as project (project.id)}
            <Table.Row
              class={cn("cursor-pointer", selectedId === project.id && "bg-muted")}
              onclick={() => onSelect(project)}
            >
              <Table.Cell>{project.name}</Table.Cell>
              <Table.Cell>{project.path}</Table.Cell>
              <Table.Cell>
                {project.summary ? project.summary.service_count : "—"}
              </Table.Cell>
              <Table.Cell>
                {#if project.has_errors === null}
                  <Badge variant="outline">Manual entry</Badge>
                {:else if project.has_errors}
                  <Badge variant="destructive">Has diagnostics</Badge>
                {:else}
                  <Badge variant="default">Clean</Badge>
                {/if}
              </Table.Cell>
            </Table.Row>
          {/each}
        {:else}
          <Table.Row>
            <Table.Cell class="text-muted-foreground">No projects imported</Table.Cell>
            <Table.Cell class="text-muted-foreground"
              >Connect the daemon to add a workspace</Table.Cell
            >
            <Table.Cell></Table.Cell>
            <Table.Cell></Table.Cell>
          </Table.Row>
        {/if}
      </Table.Body>
    </Table.Root>
  </Card.Root>
</div>
