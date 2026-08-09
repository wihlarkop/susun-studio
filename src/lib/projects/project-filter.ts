import type { StudioProject } from "$lib/daemon/client";

function normalized(value: string): string {
  return value.trim().replace(/\s+/g, " ").toLocaleLowerCase();
}

function projectSearchText(project: StudioProject): string {
  return [
    project.name,
    project.path,
    ...(project.summary?.services.map((service) => service.name) ?? []),
  ]
    .join(" ")
    .toLocaleLowerCase();
}

export function filterProjects(projects: readonly StudioProject[], query: string): StudioProject[] {
  const term = normalized(query);
  if (!term) return [...projects];
  return projects.filter((project) => projectSearchText(project).includes(term));
}

export function recentProjects(projects: readonly StudioProject[], query: string): StudioProject[] {
  return filterProjects(projects, query)
    .sort((left, right) => {
      const leftRecency = left.last_opened_at_ms ?? left.created_at_ms;
      const rightRecency = right.last_opened_at_ms ?? right.created_at_ms;
      return rightRecency - leftRecency || left.name.localeCompare(right.name);
    })
    .slice(0, 5);
}
