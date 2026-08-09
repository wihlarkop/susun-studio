import type { RuntimeWorkflowCompatibility } from "$lib/daemon/client";

export type CompatibilityGroup = {
  id: "projects" | "artifacts" | "runtime" | "recovery";
  label: string;
  workflows: Array<RuntimeWorkflowCompatibility & { label: string }>;
};

const workflowOrder = [
  "project_plan_execute",
  "logs_events_exec",
  "watch",
  "container_inventory_actions",
  "image_inventory_tag_remove_prune",
  "registry_pull",
  "registry_push",
  "registry_credentials",
  "image_build",
  "build_cache",
  "volumes_networks",
  "runtime_lifecycle",
  "runtime_resources",
  "diagnostics",
  "metadata_migration",
] as const;

const workflowLabels: Record<string, string> = {
  project_plan_execute: "Project actions",
  logs_events_exec: "Logs and events",
  watch: "Watch",
  container_inventory_actions: "Containers",
  image_inventory_tag_remove_prune: "Images",
  registry_pull: "Registry pull",
  registry_push: "Registry push",
  registry_credentials: "Registry credentials",
  image_build: "Image builds",
  build_cache: "Build cache",
  volumes_networks: "Volumes and networks",
  runtime_lifecycle: "Runtime lifecycle",
  runtime_resources: "Runtime resources",
  diagnostics: "Diagnostics",
  metadata_migration: "Project binding migration",
};

const groupForWorkflow: Record<string, CompatibilityGroup["id"]> = {
  project_plan_execute: "projects",
  logs_events_exec: "projects",
  watch: "projects",
  container_inventory_actions: "artifacts",
  image_inventory_tag_remove_prune: "artifacts",
  registry_pull: "artifacts",
  registry_push: "artifacts",
  registry_credentials: "artifacts",
  image_build: "artifacts",
  build_cache: "artifacts",
  volumes_networks: "runtime",
  runtime_lifecycle: "runtime",
  runtime_resources: "runtime",
  diagnostics: "recovery",
  metadata_migration: "recovery",
};

const groupLabels: Record<CompatibilityGroup["id"], string> = {
  projects: "Projects",
  artifacts: "Artifacts",
  runtime: "Runtime",
  recovery: "Recovery",
};

export function compatibilityGroups(
  workflows: RuntimeWorkflowCompatibility[],
): CompatibilityGroup[] {
  const workflowById = new Map(workflows.map((workflow) => [workflow.id, workflow]));
  const groups = new Map<CompatibilityGroup["id"], CompatibilityGroup>();

  for (const id of workflowOrder) {
    const workflow = workflowById.get(id);
    if (!workflow) continue;
    const groupId = groupForWorkflow[id];
    let group = groups.get(groupId);
    if (!group) {
      group = { id: groupId, label: groupLabels[groupId], workflows: [] };
      groups.set(groupId, group);
    }
    group.workflows.push({ ...workflow, label: workflowLabels[id] });
  }

  const groupOrder: CompatibilityGroup["id"][] = ["projects", "artifacts", "runtime", "recovery"];
  return groupOrder
    .map((id) => groups.get(id))
    .filter((group): group is CompatibilityGroup => Boolean(group));
}

export function compatibilityLevelLabel(level: RuntimeWorkflowCompatibility["level"]): string {
  switch (level) {
    case "supported":
      return "Supported";
    case "limited":
      return "Limited";
    case "unsupported":
      return "Unsupported";
    case "unavailable":
      return "Unavailable";
    case "unknown":
      return "Unknown";
  }
}

export function boundedCompatibilityError(error: unknown): string {
  const status =
    typeof error === "object" && error !== null && "status" in error
      ? (error as { status?: unknown }).status
      : undefined;
  if (status === 404) return "This runtime is no longer available.";
  if (status === 502) return "The runtime could not be reached.";
  if (typeof status === "number" && status >= 500)
    return "Compatibility could not be checked right now.";
  return "Compatibility could not be loaded. Try again.";
}

export function acceptsCompatibilityResult(
  requestedProfileId: string,
  requestGeneration: number,
  currentProfileId: string | null,
  currentGeneration: number,
): boolean {
  return requestedProfileId === currentProfileId && requestGeneration === currentGeneration;
}

export function shouldRequestCompatibility(
  expanded: boolean,
  requestedProfileId: string,
  currentProfileId: string | null,
): boolean {
  return expanded && requestedProfileId === currentProfileId;
}
