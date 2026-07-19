<script lang="ts">
  const toneByStatus: Record<string, string> = {
    // container states
    running: "success",
    created: "info",
    restarting: "warning",
    paused: "warning",
    exited: "muted",
    // health states
    healthy: "success",
    starting: "warning",
    unhealthy: "destructive",
    // job states
    succeeded: "success",
    failed: "destructive",
    cancelled: "muted",
    // plan safety
    safe: "success",
    caution: "warning",
    destructive: "destructive",
    // SDK capability levels (SupportLevel)
    supported: "success",
    supported_subset: "info",
    experimental: "warning",
    unsupported: "muted",
    unknown: "muted",
  };

  let { status, label }: { status: string; label?: string } = $props();
  const tone = $derived(toneByStatus[status] ?? "muted");
</script>

<span
  class={[
    "inline-flex items-center gap-1 rounded-full px-2 py-0.5 text-xs font-medium",
    tone === "success" && "bg-success/12 text-success",
    tone === "warning" && "bg-warning/15 text-warning-foreground dark:text-warning",
    tone === "destructive" && "bg-destructive/12 text-destructive",
    tone === "info" && "bg-info/12 text-info",
    tone === "muted" && "bg-muted text-muted-foreground",
  ]}
>
  <span class="size-1.5 rounded-full bg-current"></span>
  {label ?? status}
</span>
