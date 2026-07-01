<script lang="ts">
  import * as Table from "$lib/components/ui/table/index.js";
  import * as Card from "$lib/components/ui/card/index.js";
  import type { StudioProject } from "$lib/daemon/client";

  let { projects, workspaceDetail }: { projects: StudioProject[]; workspaceDetail: string } =
    $props();
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
        </Table.Row>
      </Table.Header>
      <Table.Body>
        {#if projects.length > 0}
          {#each projects as project (project.id)}
            <Table.Row>
              <Table.Cell>{project.name}</Table.Cell>
              <Table.Cell>{project.path}</Table.Cell>
            </Table.Row>
          {/each}
        {:else}
          <Table.Row>
            <Table.Cell class="text-muted-foreground">No projects imported</Table.Cell>
            <Table.Cell class="text-muted-foreground"
              >Connect the daemon to add a workspace</Table.Cell
            >
          </Table.Row>
        {/if}
      </Table.Body>
    </Table.Root>
  </Card.Root>
</div>
