<script lang="ts">
  import * as Table from "$lib/components/ui/table/index.js";
  import * as Card from "$lib/components/ui/card/index.js";
  import * as Tooltip from "$lib/components/ui/tooltip/index.js";
  import { Badge } from "$lib/components/ui/badge/index.js";
  import { cn, displayPath, formatTimestamp, relativeTime } from "$lib/utils";
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

  const workspaceSummary = $derived.by(() => {
    if (projects.length === 0) {
      return workspaceDetail;
    }
    const services = projects.reduce(
      (total, project) => total + (project.summary?.service_count ?? 0),
      0,
    );
    const withDiagnostics = projects.filter((project) => project.has_errors === true).length;
    const parts = [
      `${projects.length} project${projects.length === 1 ? "" : "s"}`,
      `${services} service${services === 1 ? "" : "s"}`,
    ];
    if (withDiagnostics > 0) {
      parts.push(`${withDiagnostics} with diagnostics`);
    }
    return parts.join(" · ");
  });

  function selectViaKeyboard(event: KeyboardEvent, project: StudioProject) {
    if (event.key === "Enter" || event.key === " ") {
      event.preventDefault();
      onSelect(project);
    }
  }
</script>

<div class="space-y-2">
  <div>
    <h3 class="text-lg font-semibold">Workspace Projects</h3>
    <p class="text-sm text-muted-foreground">{workspaceSummary}</p>
  </div>

  <Card.Root class="p-0">
    <Table.Root>
      <Table.Header>
        <Table.Row>
          <Table.Head>Name</Table.Head>
          <Table.Head>Path</Table.Head>
          <Table.Head class="text-right">Services</Table.Head>
          <Table.Head>Status</Table.Head>
        </Table.Row>
      </Table.Header>
      <Table.Body>
        {#if projects.length > 0}
          <Tooltip.Provider>
            {#each projects as project (project.id)}
              <Table.Row
                class={cn(
                  "cursor-pointer focus-visible:ring-2 focus-visible:ring-ring focus-visible:outline-none",
                  selectedId === project.id && "bg-muted",
                )}
                tabindex={0}
                aria-selected={selectedId === project.id}
                onclick={() => onSelect(project)}
                onkeydown={(event) => selectViaKeyboard(event, project)}
              >
                <Table.Cell class="font-medium">{project.name}</Table.Cell>
                <Table.Cell class="max-w-64">
                  <Tooltip.Root>
                    <Tooltip.Trigger class="block w-full truncate text-left" tabindex={-1}>
                      {displayPath(project.path)}
                    </Tooltip.Trigger>
                    <Tooltip.Content>
                      <p class="max-w-96 [overflow-wrap:anywhere]">
                        {displayPath(project.path)}
                      </p>
                      {#if project.last_analyzed_at_ms}
                        <p class="text-xs">
                          Analyzed {relativeTime(project.last_analyzed_at_ms)}
                          ({formatTimestamp(project.last_analyzed_at_ms)})
                        </p>
                      {/if}
                    </Tooltip.Content>
                  </Tooltip.Root>
                </Table.Cell>
                <Table.Cell class="text-right tabular-nums">
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
          </Tooltip.Provider>
        {:else}
          <Table.Row>
            <Table.Cell colspan={4} class="h-24 text-center text-muted-foreground">
              No projects yet. Use <span class="font-medium">Import Project</span> to add a
              Compose workspace.
            </Table.Cell>
          </Table.Row>
        {/if}
      </Table.Body>
    </Table.Root>
  </Card.Root>
</div>
