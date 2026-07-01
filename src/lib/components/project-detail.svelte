<script lang="ts">
  import * as Card from "$lib/components/ui/card/index.js";
  import { Badge } from "$lib/components/ui/badge/index.js";
  import type { StudioProject } from "$lib/daemon/client";

  let { project }: { project: StudioProject | null } = $props();
</script>

{#if project}
  <Card.Root class="gap-4 p-4">
    <div class="flex items-center justify-between">
      <h3 class="text-lg font-semibold">{project.name}</h3>
      {#if project.has_errors === null}
        <Badge variant="outline">Manual entry</Badge>
      {:else if project.has_errors}
        <Badge variant="destructive">Has diagnostics</Badge>
      {:else}
        <Badge variant="default">Clean</Badge>
      {/if}
    </div>
    <p class="text-sm text-muted-foreground">{project.path}</p>

    {#if project.summary}
      {@const summary = project.summary}
      <div class="grid grid-cols-2 gap-3 text-sm sm:grid-cols-5">
        <div>
          <span class="text-muted-foreground">Services</span>
          <p class="font-medium">
            {summary.active_service_count}/{summary.service_count} active
          </p>
        </div>
        <div>
          <span class="text-muted-foreground">Networks</span>
          <p class="font-medium">{summary.network_count}</p>
        </div>
        <div>
          <span class="text-muted-foreground">Volumes</span>
          <p class="font-medium">{summary.volume_count}</p>
        </div>
        <div>
          <span class="text-muted-foreground">Configs</span>
          <p class="font-medium">{summary.config_count}</p>
        </div>
        <div>
          <span class="text-muted-foreground">Secrets</span>
          <p class="font-medium">{summary.secret_count}</p>
        </div>
      </div>

      {#if summary.services.length > 0}
        <div class="space-y-2">
          <h4 class="text-sm font-semibold">Services</h4>
          {#each summary.services as service (service.name)}
            <div class="rounded-md border p-2 text-sm">
              <div class="flex items-center justify-between">
                <span class="font-medium">{service.name}</span>
                <Badge variant={service.active ? "default" : "outline"}>
                  {service.active ? "active" : "inactive"}
                </Badge>
              </div>
              <p class="text-muted-foreground">
                {service.image ?? (service.has_build ? "built from source" : "no image")}
                {#if service.ports.length > 0}
                  · {service.ports
                    .map((port) => `${port.published ?? port.target}:${port.target}/${port.protocol}`)
                    .join(", ")}
                {/if}
                {#if service.volume_count > 0}
                  · {service.volume_count} volume{service.volume_count === 1 ? "" : "s"}
                {/if}
                {#if service.dependencies.length > 0}
                  · depends on {service.dependencies.join(", ")}
                {/if}
              </p>
            </div>
          {/each}
        </div>
      {/if}

      {#if summary.networks.length + summary.volumes.length + summary.configs.length + summary.secrets.length > 0}
        <div class="grid grid-cols-2 gap-3 text-sm sm:grid-cols-4">
          {#if summary.networks.length > 0}
            <div>
              <span class="text-muted-foreground">Networks</span>
              {#each summary.networks as resource (resource.name)}
                <p>{resource.name}{resource.external ? " (external)" : ""}</p>
              {/each}
            </div>
          {/if}
          {#if summary.volumes.length > 0}
            <div>
              <span class="text-muted-foreground">Volumes</span>
              {#each summary.volumes as resource (resource.name)}
                <p>{resource.name}{resource.external ? " (external)" : ""}</p>
              {/each}
            </div>
          {/if}
          {#if summary.configs.length > 0}
            <div>
              <span class="text-muted-foreground">Configs</span>
              {#each summary.configs as resource (resource.name)}
                <p>{resource.name}{resource.external ? " (external)" : ""}</p>
              {/each}
            </div>
          {/if}
          {#if summary.secrets.length > 0}
            <div>
              <span class="text-muted-foreground">Secrets</span>
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
      <div class="space-y-2">
        <h4 class="text-sm font-semibold">Diagnostics</h4>
        {#each project.diagnostics.diagnostics as diagnostic}
          <div class="rounded-md border p-2 text-sm">
            <div class="flex items-center gap-2">
              <Badge variant={diagnostic.severity === "error" ? "destructive" : "outline"}>
                {diagnostic.severity}
              </Badge>
              <span class="font-mono text-xs text-muted-foreground">{diagnostic.code}</span>
            </div>
            <p>{diagnostic.message}</p>
            {#if diagnostic.help}
              <p class="text-muted-foreground">{diagnostic.help}</p>
            {/if}
          </div>
        {/each}
      </div>
    {/if}
  </Card.Root>
{:else}
  <Card.Root class="p-4">
    <p class="text-sm text-muted-foreground">Select a project to see its details.</p>
  </Card.Root>
{/if}
