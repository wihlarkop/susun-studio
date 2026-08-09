import { describe, expect, it } from "vitest";
import {
  canCompleteBuiltInOnboarding,
  canDismissInitialOnboarding,
  resolveOnboardingView,
  selectableExternalProfiles,
} from "./onboarding-state";

const pending = { state: "pending", choice: null, completed_at_ms: null, updated_at_ms: 1 } as const;
const completedExisting = {
  state: "completed",
  choice: "existing",
  completed_at_ms: 2,
  updated_at_ms: 2,
} as const;

const builtInReady = {
  id: "susun-runtime",
  runtime_class: "built_in",
  ownership_state: "studio_managed",
  management: { can_select: true },
};

describe("runtime onboarding state", () => {
  it("waits for a connected daemon without changing pending onboarding", () => {
    expect(resolveOnboardingView({ connected: false, onboarding: pending })).toEqual({ kind: "wait" });
  });

  it("shows the chooser only for a pending state with no choice", () => {
    expect(resolveOnboardingView({ connected: true, onboarding: pending })).toEqual({ kind: "chooser" });
  });

  it("completes built-in onboarding only after the managed profile is ready and preferred", () => {
    expect(
      canCompleteBuiltInOnboarding({
        onboarding: pending,
        binding: { state: "ready", profile_id: "susun-runtime" },
        profiles: [builtInReady],
      }),
    ).toBe(true);
    expect(
      canCompleteBuiltInOnboarding({
        onboarding: pending,
        binding: { state: "unavailable", profile_id: "susun-runtime" },
        profiles: [builtInReady],
      }),
    ).toBe(false);
  });

  it("lists only selectable external runtime profiles", () => {
    expect(
      selectableExternalProfiles([
        builtInReady,
        { id: "podman", runtime_class: "external_local", management: { can_select: true } },
        { id: "offline", runtime_class: "external_local", management: { can_select: false } },
      ]),
    ).toEqual(["podman"]);
  });

  it("hides dismissed onboarding until an explicit reopen", () => {
    expect(
      resolveOnboardingView({
        connected: true,
        onboarding: { state: "dismissed", choice: null, completed_at_ms: null, updated_at_ms: 2 },
      }),
    ).toEqual({ kind: "hidden" });
  });

  it("does not reopen first-run onboarding when a completed choice becomes unavailable", () => {
    expect(
      resolveOnboardingView({
        connected: true,
        onboarding: completedExisting,
        binding: { state: "unavailable", profile_id: "podman" },
      }),
    ).toEqual({ kind: "hidden" });
  });

  it("keeps restored completed onboarding state intact", () => {
    expect(resolveOnboardingView({ connected: true, onboarding: completedExisting })).toEqual({
      kind: "hidden",
    });
  });

  it("allows only the initial pending flow to persist dismissal", () => {
    expect(canDismissInitialOnboarding({ reopened: false, onboarding: pending })).toBe(true);
    expect(canDismissInitialOnboarding({ reopened: true, onboarding: pending })).toBe(false);
    expect(canDismissInitialOnboarding({ reopened: false, onboarding: completedExisting })).toBe(false);
  });
});
