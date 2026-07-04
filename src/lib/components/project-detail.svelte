<script lang="ts">
  import * as Card from "$lib/components/ui/card/index.js";
  import { Badge } from "$lib/components/ui/badge/index.js";
  import { Separator } from "$lib/components/ui/separator/index.js";
  import { displayPath, formatTimestamp, relativeTime } from "$lib/utils";
  import type { StudioProject } from "$lib/daemon/client";

  let { project }: { project: StudioProject | null } = $props();
</script>

{#if project}
  <Card.Root class="gap-0 p-0">
    <Card.Header class="gap-1 p-4">
      <div class="flex items-center justify-between gap-2">
        <Card.Title class="text-lg">{project.name}</Card.Title>
        {#if project.has_errors === null}
          <Badge variant="outline">Manual entry</Badge>
        {:else if project.has_errors}
          <Badge variant="destructive">Has diagnostics</Badge>
        {:else}
          <Badge variant="default">Clean</Badge>
        {/if}
      </div>
      <Card.Description class="font-mono text-xs [overflow-wrap:anywhere]">
        {displayPath(project.path)}
      </Card.Description>
      {#if project.last_analyzed_at_ms}
        <Card.Description class="text-xs" title={formatTimestamp(project.last_analyzed_at_ms)}>
          Analyzed {relativeTime(project.last_analyzed_at_ms)}
        </Card.Description>
      {/if}
    </Card.Header>

    <Separator />

    <Card.Content class="flex flex-col gap-4 p-4">
      {#if project.summary}
        {@const summary = project.summary}
        <div class="grid grid-cols-2 gap-3 text-sm sm:grid-cols-5">
          <div>
            <span class="text-xs text-muted-foreground">Services</span>
            <p class="font-medium tabular-nums">
              {summary.active_service_count}/{summary.service_count} active
            </p>
          </div>
          <div>
            <span class="text-xs text-muted-foreground">Networks</span>
            <p class="font-medium tabular-nums">{summary.network_count}</p>
          </div>
          <div>
            <span class="text-xs text-muted-foreground">Volumes</span>
            <p class="font-medium tabular-nums">{summary.volume_count}</p>
          </div>
          <div>
            <span class="text-xs text-muted-foreground">Configs</span>
            <p class="font-medium tabular-nums">{summary.config_count}</p>
          </div>
          <div>
            <span class="text-xs text-muted-foreground">Secrets</span>
            <p class="font-medium tabular-nums">{summary.secret_count}</p>
          </div>
        </div>

        {#if summary.services.length > 0}
          <Separator />
          <div class="space-y-2">
            <h4 class="text-sm font-semibold">Services</h4>
            {#each summary.services as service (service.name)}
              <div class="rounded-md border p-3 text-sm">
                <div class="flex items-center justify-between gap-2">
                  <span class="font-medium">{service.name}</span>
                  <Badge variant={service.active ? "default" : "outline"}>
                    {service.active ? "active" : "inactive"}
                  </Badge>
                </div>
                <p class="mt-1 text-xs text-muted-foreground [overflow-wrap:anywhere]">
                  {service.image ?? (service.has_build ? "built from source" : "no image")}
                </p>
                {#if service.ports.length > 0 || service.volume_count > 0 || service.dependencies.length > 0}
                  <div class="mt-2 flex flex-wrap gap-1">
                    {#each service.ports as port (`${port.published}:${port.target}/${port.protocol}`)}
                      <Badge variant="secondary" class="font-mono text-xs">
                        {port.published ?? port.target}:{port.target}/{port.protocol}
                      </Badge>
                    {/each}
                    {#if service.volume_count > 0}
                      <Badge variant="outline" class="text-xs">
                        {service.volume_count} volume{service.volume_count === 1 ? "" : "s"}
                      </Badge>
                    {/if}
                    {#each service.dependencies as dependency (dependency)}
                      <Badge variant="outline" class="text-xs">→ {dependency}</Badge>
                    {/each}
                  </div>
                {/if}
              </div>
            {/each}
          </div>
        {/if}

        {#if summary.networks.length + summary.volumes.length + summary.configs.length + summary.secrets.length > 0}
          <Separator />
          <div class="grid grid-cols-2 gap-3 text-sm">
            {#if summary.networks.length > 0}
              <div>
                <span class="text-xs text-muted-foreground">Networks</span>
                {#each summary.networks as resource (resource.name)}
                  <p>{resource.name}{resource.external ? " (external)" : ""}</p>
                {/each}
              </div>
            {/if}
            {#if summary.volumes.length > 0}
              <div>
                <span class="text-xs text-muted-foreground">Volumes</span>
                {#each summary.volumes as resource (resource.name)}
                  <p>{resource.name}{resource.external ? " (external)" : ""}</p>
                {/each}
              </div>
            {/if}
            {#if summary.configs.length > 0}
              <div>
                <span class="text-xs text-muted-foreground">Configs</span>
                {#each summary.configs as resource (resource.name)}
                  <p>{resource.name}{resource.external ? " (external)" : ""}</p>
                {/each}
              </div>
            {/if}
            {#if summary.secrets.length > 0}
              <div>
                <span class="text-xs text-muted-foreground">Secrets</span>
                {#each summary.secrets as resource (resource.name)}
                  <p>{resource.name}{resource.external ? " (external)" : ""}</p>
                {/each}
              </div>
            {/if}
          </div>
        {/if}
      {:else}
        <p class="text-sm text-muted-foreground">
          This project was created manually and has no Susun analysis yet.
        </p>
      {/if}

      {#if project.diagnostics && project.diagnostics.diagnostics.length > 0}
        <Separator />
        <div class="space-y-2">
          <h4 class="text-sm font-semibold">Diagnostics</h4>
          {#each project.diagnostics.diagnostics as diagnostic}
            <div class="rounded-md border p-3 text-sm">
              <div class="flex items-center gap-2">
                <Badge variant={diagnostic.severity === "error" ? "destructive" : "outline"}>
                  {diagnostic.severity}
                </Badge>
                <span class="font-mono text-xs text-muted-foreground">{diagnostic.code}</span>
              </div>
              <p class="mt-1">{diagnostic.message}</p>
              {#if diagnostic.help}
                <p class="text-muted-foreground">{diagnostic.help}</p>
              {/if}
            </div>
          {/each}
        </div>
      {/if}
    </Card.Content>
  </Card.Root>
{:else}
  <Card.Root class="flex h-full min-h-32 items-center justify-center p-4">
    <p class="text-sm text-muted-foreground">Select a project to see its details.</p>
  </Card.Root>
{/if}
