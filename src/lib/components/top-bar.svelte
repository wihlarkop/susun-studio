<script lang="ts">
  import * as Sidebar from "$lib/components/ui/sidebar/index.js";
  import { Button } from "$lib/components/ui/button/index.js";

  let { connected, onImportClick }: { connected: boolean; onImportClick: () => void } = $props();
</script>

<header class="flex items-center justify-between gap-4">
  <div class="flex items-center gap-3">
    <Sidebar.Trigger />
    <div>
      <p class="text-xs font-semibold tracking-wide text-primary uppercase">Phase 1 Foundation</p>
      <h2 class="text-2xl leading-tight font-semibold">Projects</h2>
    </div>
  </div>
  <Button disabled={!connected} onclick={onImportClick}>Import Project</Button>
</header>
