import { describe, expect, it } from "vitest";
import type { StudioProject } from "$lib/daemon/client";
import { filterProjects, recentProjects } from "./project-filter";

function project(overrides: Partial<StudioProject> = {}): StudioProject {
  return {
    id: "project",
    name: "Gateway",
    path: "C:/Projects/Gateway",
    created_at_ms: 10,
    last_opened_at_ms: null,
    last_analyzed_at_ms: null,
    has_errors: false,
    summary: {
      schema_version: { major: 1, minor: 0 },
      project_name: "Gateway",
      project_instance: null,
      service_count: 1,
      active_service_count: 0,
      network_count: 0,
      volume_count: 0,
      config_count: 0,
      secret_count: 0,
      networks: [],
      volumes: [],
      configs: [],
      secrets: [],
      has_errors: false,
      diagnostic_count: 0,
      services: [
        {
          name: "api",
          active: false,
          image: null,
          has_build: false,
          profile_count: 0,
          profiles: [],
          port_count: 0,
          ports: [],
          volume_count: 0,
          volumes: [],
          network_count: 0,
          networks: [],
          config_count: 0,
          configs: [],
          secret_count: 0,
          secrets: [],
          dependency_count: 0,
          dependencies: [],
        },
      ],
    },
    diagnostics: null,
    runtime_profile_id: null,
    runtime_binding: {
      source: "platform_default",
      state: "unconfigured",
      profile_id: null,
      runtime_class: null,
      display_name: "Platform default",
    },
    ...overrides,
  };
}

describe("project filtering", () => {
  const projects = [
    project({ id: "gateway", name: "Gateway", created_at_ms: 10 }),
    project({ id: "worker", name: "Worker", path: "D:/Work/Queue", created_at_ms: 20 }),
    project({
      id: "web",
      name: "Web",
      path: "E:/Sites/Web",
      last_opened_at_ms: 30,
      created_at_ms: 5,
    }),
  ];

  it("matches names, display paths, and service names case-insensitively", () => {
    expect(filterProjects(projects, "gateway").map((item) => item.id)).toEqual(["gateway"]);
    expect(filterProjects(projects, " queue ").map((item) => item.id)).toEqual(["worker"]);
    expect(filterProjects(projects, " API ").map((item) => item.id)).toEqual([
      "gateway",
      "worker",
      "web",
    ]);
  });

  it("normalizes whitespace and does not mutate the source array", () => {
    const source = [...projects];

    expect(filterProjects(projects, "  gateway  ").map((item) => item.id)).toEqual(["gateway"]);
    expect(projects).toEqual(source);
  });

  it("orders recent projects by opened time then creation and caps at five", () => {
    const extra = Array.from({ length: 5 }, (_, index) =>
      project({
        id: `extra-${index}`,
        name: `Extra ${index}`,
        created_at_ms: 100 - index,
        last_opened_at_ms: null,
      }),
    );

    expect(recentProjects([...projects, ...extra], "").map((item) => item.id)).toEqual([
      "extra-0",
      "extra-1",
      "extra-2",
      "extra-3",
      "extra-4",
    ]);
  });

  it("returns every filtered project in All mode", () => {
    expect(filterProjects(projects, "").map((item) => item.id)).toEqual([
      "gateway",
      "worker",
      "web",
    ]);
  });
});
