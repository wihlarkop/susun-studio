import { describe, expect, it } from "vitest";
import type { RuntimeBindingSummary } from "$lib/daemon/client";
import { presentProjectBinding } from "./project-binding-state";

function binding(overrides: Partial<RuntimeBindingSummary> = {}): RuntimeBindingSummary {
  return {
    source: "global_preference",
    state: "ready",
    profile_id: "preferred",
    runtime_class: "built_in",
    display_name: "Susun Runtime",
    ...overrides,
  };
}

describe("project runtime binding presentation", () => {
  it("describes an unpinned project using a ready preferred runtime", () => {
    expect(presentProjectBinding(binding())).toMatchObject({
      selectorLabel: "Use preferred runtime",
      bindingLabel: "Preferred runtime",
      blocked: false,
    });
  });

  it("describes an explicit project pin without falling back", () => {
    expect(
      presentProjectBinding(binding({ source: "project_pin", runtime_class: "external_local" })),
    ).toMatchObject({
      selectorLabel: "Pinned runtime",
      bindingLabel: "Pinned runtime",
      blocked: false,
    });
  });

  it("keeps platform-default compatibility explicit", () => {
    expect(
      presentProjectBinding(
        binding({
          source: "platform_default",
          state: "unconfigured",
          profile_id: null,
          runtime_class: null,
          display_name: "Platform default",
        }),
      ),
    ).toMatchObject({
      selectorLabel: "Use preferred runtime",
      bindingLabel: "Platform default",
      blocked: false,
    });
  });

  it.each([
    ["missing", "The pinned runtime is missing."],
    ["unavailable", "The pinned runtime is unavailable."],
  ] as const)("blocks a %s project pin", (state, blockedReason) => {
    expect(presentProjectBinding(binding({ source: "project_pin", state }))).toMatchObject({
      blocked: true,
      blockedReason,
    });
  });

  it.each([
    ["missing", "The preferred runtime is missing."],
    ["unavailable", "The preferred runtime is unavailable."],
  ] as const)("blocks an unpinned project when preference is %s", (state, blockedReason) => {
    expect(presentProjectBinding(binding({ state }))).toMatchObject({
      blocked: true,
      blockedReason,
    });
  });

  it("allows changing or clearing a pin without hiding project content", () => {
    expect(presentProjectBinding(binding({ source: "project_pin" }))).toMatchObject({
      canChange: true,
      canClear: true,
      keepProjectReadable: true,
    });
  });
});
