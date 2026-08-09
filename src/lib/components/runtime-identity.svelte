<script lang="ts">
  import { Badge } from "$lib/components/ui/badge/index.js";
  import * as Tooltip from "$lib/components/ui/tooltip/index.js";
  import type { RuntimePresentation } from "$lib/runtime/presentation";

  let { presentation, compact = false }: { presentation: RuntimePresentation; compact?: boolean } = $props();

  const badgeVariant = $derived(
    presentation.tone === "danger"
      ? "destructive"
      : presentation.tone === "warning"
        ? "outline"
        : presentation.tone === "positive"
          ? "default"
          : "secondary",
  );
</script>

<Tooltip.Provider>
  <div class:gap-1={compact} class="flex min-w-0 flex-wrap items-center gap-2">
    <Tooltip.Root>
      <Tooltip.Trigger class="min-w-0 max-w-full truncate text-left font-medium" tabindex={-1}>
        {presentation.title}
      </Tooltip.Trigger>
      <Tooltip.Content>
        <p class="max-w-80 [overflow-wrap:anywhere]">{presentation.tooltip}</p>
      </Tooltip.Content>
    </Tooltip.Root>
    <Badge variant="outline" class="shrink-0">{presentation.classLabel}</Badge>
    <Badge variant={badgeVariant} class="shrink-0">{presentation.stateLabel}</Badge>
    {#if presentation.supportingText}
      <span class="shrink-0 text-xs text-muted-foreground">{presentation.supportingText}</span>
    {/if}
    {#if presentation.externalAppNote}
      <span class="shrink-0 text-xs text-muted-foreground">{presentation.externalAppNote}</span>
    {/if}
  </div>
</Tooltip.Provider>
