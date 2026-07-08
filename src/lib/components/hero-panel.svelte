<script lang="ts">
  import * as Card from "$lib/components/ui/card/index.js";
  import { Button } from "$lib/components/ui/button/index.js";
  import { Skeleton } from "$lib/components/ui/skeleton/index.js";
  import { RefreshCw } from "@lucide/svelte";
  import { getDaemonBaseUrl } from "$lib/daemon/client";
  import type { HealthState } from "$lib/daemon/daemon-state.svelte";

  let { healthState, onRetry }: { healthState: HealthState; onRetry: () => void } = $props();
</script>

{#if healthState.kind === "checking"}
  <Card.Root>
    <Card.Content class="flex items-center gap-4 p-6">
      <Skeleton class="h-10 w-10 rounded-full" />
      <div class="space-y-2">
        <Skeleton class="h-4 w-64" />
        <Skeleton class="h-3 w-40" />
      </div>
    </Card.Content>
  </Card.Root>
{:else if healthState.kind === "disconnected"}
  <Card.Root>
    <Card.Content class="flex flex-col gap-2 p-6">
      <p class="text-xs font-semibold tracking-wide text-primary uppercase">Daemon offline</p>
      <h3 class="text-xl font-semibold">Start the Susun Studio daemon to begin</h3>
      <p class="max-w-xl text-sm text-muted-foreground">
        The desktop app is only the client; projects, settings, and analysis live behind the
        local daemon API at
        <code class="rounded bg-muted px-1 py-0.5 font-mono text-xs">{getDaemonBaseUrl()}</code>.
        Run
        <code class="rounded bg-muted px-1 py-0.5 font-mono text-xs">bun run daemon</code>
        from the repository root. This page rechecks automatically every few seconds.
      </p>
      <p class="text-xs text-muted-foreground">{healthState.detail}</p>
      <Button variant="outline" class="w-fit" onclick={onRetry}>
        <RefreshCw />
        Retry now
      </Button>
    </Card.Content>
  </Card.Root>
{/if}
