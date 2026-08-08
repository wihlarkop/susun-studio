import { describe, expect, it } from "vitest";
import type { RuntimeBindingSummary } from "$lib/daemon/client";
import { PLATFORM_DEFAULT_ENGINE_ID, resolveActiveEngineId } from "./engine-identity";

function binding(overrides: Partial<RuntimeBindingSummary> = {}): RuntimeBindingSummary {
  return {
    source: "global_preference",
    state: "ready",
    profile_id: "profile-podman-1",
    runtime_class: "external_local",
    display_name: "Podman",
    ...overrides,
  };
}

describe("resolveActiveEngineId", () => {
  it("uses the ready policy-bound profile id", () => {
    expect(resolveActiveEngineId(binding())).toBe("profile-podman-1");
  });

  it("uses the platform-default sentinel only for the explicit compatibility state", () => {
    expect(
      resolveActiveEngineId(
        binding({
          source: "platform_default",
          state: "unconfigured",
          profile_id: null,
          runtime_class: null,
          display_name: "Platform default",
        }),
      ),
    ).toBe(PLATFORM_DEFAULT_ENGINE_ID);
  });

  it("does not fall back when the configured preference is unavailable", () => {
    expect(
      resolveActiveEngineId(binding({ state: "unavailable", display_name: "Stopped Podman" })),
    ).toBeNull();
  });

  it("does not fall back when the configured preference is missing", () => {
    expect(
      resolveActiveEngineId(
        binding({
          state: "missing",
          profile_id: "removed-runtime",
          runtime_class: null,
          display_name: "Missing runtime (removed-runtime)",
        }),
      ),
    ).toBeNull();
  });
});
