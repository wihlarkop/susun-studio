<script lang="ts">
  import * as Card from "$lib/components/ui/card/index.js";
  import { Badge } from "$lib/components/ui/badge/index.js";
  import { defaultDaemonBaseUrl } from "$lib/daemon/client";
  import type { HealthState } from "$lib/daemon/daemon-state.svelte";

  let { healthState }: { healthState: HealthState } = $props();
</script>

<Card.Root>
  <Card.Content class="flex flex-col gap-6 p-6 sm:flex-row sm:items-center sm:justify-between">
    <div class="space-y-2">
      <p class="text-xs font-semibold tracking-wide text-primary uppercase">
        Local platform spine
      </p>
      <h3 class="text-xl font-semibold">Connect to Susun Studio daemon to begin</h3>
      <p class="max-w-xl text-sm text-muted-foreground">
        The desktop app is only the client. Workspaces, projects, settings, events, and future
        engine tasks live behind the local daemon API.
      </p>
    </div>
    <div class="flex min-w-40 flex-col gap-1 rounded-lg bg-muted p-4">
      <span class="text-xs text-muted-foreground">Health</span>
      <Badge variant={healthState.kind === "connected" ? "default" : "outline"} class="w-fit">
        {healthState.label}
      </Badge>
      <span class="text-xs text-muted-foreground">
        {healthState.health?.product ?? defaultDaemonBaseUrl}
      </span>
    </div>
  </Card.Content>
</Card.Root>
