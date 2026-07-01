<script lang="ts">
  import * as Card from "$lib/components/ui/card/index.js";
  import type { StudioSettings } from "$lib/daemon/client";

  let { settings }: { settings: StudioSettings | undefined } = $props();
</script>

<Card.Root class="flex-row items-center justify-between gap-4 p-4">
  <span class="text-xs text-muted-foreground">Default project root</span>
  <strong class="max-w-full [overflow-wrap:anywhere] text-sm font-medium">
    {settings?.default_project_root || "Not configured"}
  </strong>
</Card.Root>
