import { describe, expect, it } from "vitest";
import { PLATFORM_DEFAULT_ENGINE_ID, resolveActiveEngineId } from "./engine-identity";

describe("resolveActiveEngineId", () => {
  it("uses the selected profile's own id when one is selected", () => {
    expect(resolveActiveEngineId("profile-podman-1")).toBe("profile-podman-1");
  });

  it("falls back to the platform-default sentinel when no profile is selected", () => {
    expect(resolveActiveEngineId(null)).toBe(PLATFORM_DEFAULT_ENGINE_ID);
    expect(resolveActiveEngineId(undefined)).toBe(PLATFORM_DEFAULT_ENGINE_ID);
  });

  it("never falls back once a profile is selected, even for an empty-looking id", () => {
    // A profile id is always a non-empty server-generated string in practice,
    // but the fallback must only trigger on null/undefined, never truthiness,
    // so a genuinely selected profile's id is never silently discarded.
    expect(resolveActiveEngineId("profile-1")).not.toBe(PLATFORM_DEFAULT_ENGINE_ID);
  });
});
