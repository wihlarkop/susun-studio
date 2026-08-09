import { describe, expect, it } from "vitest";
import type { RuntimeWorkflowCompatibility } from "$lib/daemon/client";
import {
  acceptsCompatibilityResult,
  boundedCompatibilityError,
  compatibilityGroups,
  compatibilityLevelLabel,
  shouldRequestCompatibility,
} from "./compatibility";

describe("runtime compatibility presentation", () => {
  it("keeps fixed workflow ordering and distinct capability labels", () => {
    const workflows: RuntimeWorkflowCompatibility[] = [
      { id: "runtime_lifecycle", level: "unsupported", reason_code: "external", detail: "" },
      { id: "project_plan_execute", level: "supported", reason_code: "ok", detail: "" },
      {
        id: "image_build",
        level: "unsupported",
        reason_code: "build_endpoint_not_supported",
        detail: "",
      },
    ];
    expect(
      compatibilityGroups(workflows).map((group) => [
        group.id,
        group.workflows.map((workflow) => workflow.id),
      ]),
    ).toEqual([
      ["projects", ["project_plan_execute"]],
      ["artifacts", ["image_build"]],
      ["runtime", ["runtime_lifecycle"]],
    ]);
    expect(compatibilityLevelLabel("limited")).toBe("Limited");
  });

  it("maps raw daemon failures to bounded guidance", () => {
    expect(
      boundedCompatibilityError({ status: 502, message: "//./pipe/private argv secret" }),
    ).not.toContain("pipe");
  });

  it("rejects stale compatibility responses after a profile change", () => {
    expect(acceptsCompatibilityResult("old", 1, "new", 2)).toBe(false);
    expect(acceptsCompatibilityResult("current", 3, "current", 3)).toBe(true);
  });

  it("only requests compatibility for the expanded current profile", () => {
    expect(shouldRequestCompatibility(false, "podman", "podman")).toBe(false);
    expect(shouldRequestCompatibility(true, "podman", "docker")).toBe(false);
    expect(shouldRequestCompatibility(true, "podman", "podman")).toBe(true);
  });
});
