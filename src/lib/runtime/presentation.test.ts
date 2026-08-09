import { describe, expect, it } from "vitest";
import type {
  RuntimeBindingSummary,
  RuntimeProfile,
  RuntimeProviderStatus,
} from "$lib/daemon/client";
import {
  presentRuntimeAttribution,
  presentRuntimeBinding,
  presentRuntimeProfile,
} from "./presentation";

const management = {
  can_select: true,
  can_forget: true,
  can_adopt: false,
  requires_recovery: false,
  blocks_destructive_actions: false,
};

function profile(overrides: Partial<RuntimeProfile> = {}): RuntimeProfile {
  return {
    id: "provider-profile",
    provider_id: "provider",
    provider_runtime_key: "runtime",
    display_name: "Podman machine local",
    product: "Podman",
    platform: "windows",
    runtime_class: "external_local",
    ownership_state: "external",
    source: "provider_discovery",
    installation: { state: "installed", detail: null },
    process: { state: "running", detail: null },
    connection: { state: "ready", detail: null },
    endpoint_summary: null,
    availability_state: "available",
    last_seen_at_ms: null,
    missing_since_ms: null,
    last_error: null,
    is_preferred: false,
    observation_revision: 1,
    observed_at_ms: 1,
    management,
    freshness: "fresh",
    ...overrides,
  };
}

function provider(overrides: Partial<RuntimeProviderStatus> = {}): RuntimeProviderStatus {
  return {
    provider_id: "provider",
    display_name: "Podman",
    product: "Podman",
    platform: "windows",
    supported: true,
    experience: {
      can_create_builtin: false,
      can_discover_external: true,
      can_manage_builtin_lifecycle: false,
      can_manage_external_lifecycle: false,
      can_manage_resources: false,
      requires_external_desktop_app: false,
    },
    installation: { state: "installed", detail: null },
    process: { state: "running", detail: null },
    connection: { state: "ready", detail: null },
    freshness: "fresh",
    summary: "",
    remediation: [],
    actions: [],
    profiles: [],
    ...overrides,
  };
}

function binding(overrides: Partial<RuntimeBindingSummary> = {}): RuntimeBindingSummary {
  return {
    source: "global_preference",
    state: "ready",
    profile_id: "provider-profile",
    runtime_class: "external_local",
    display_name: "Podman machine local",
    ...overrides,
  };
}

describe("runtime presentation", () => {
  it("names a Studio-managed built-in runtime truthfully", () => {
    expect(
      presentRuntimeProfile(
        profile({
          runtime_class: "built_in",
          ownership_state: "studio_managed",
          display_name: "ignored provider machine name",
        }),
      ),
    ).toMatchObject({
      title: "Susun Runtime",
      supportingText: "Powered by Podman",
      classLabel: "Built-in",
      stateLabel: "Ready",
      actionBlockedReason: null,
    });
  });

  it("keeps an external Podman runtime external even when provider data changes", () => {
    expect(
      presentRuntimeProfile(
        profile({ product: "Docker Desktop" }),
        provider({ display_name: "Podman" }),
      ),
    ).toMatchObject({
      title: "Podman machine local",
      supportingText: null,
      classLabel: "External",
      stateLabel: "Ready",
    });
  });

  it("describes Docker Desktop as external and records its external-app requirement", () => {
    expect(
      presentRuntimeProfile(
        profile({ display_name: "Docker Desktop", product: "not used" }),
        provider({
          display_name: "not used",
          experience: {
            can_create_builtin: false,
            can_discover_external: true,
            can_manage_builtin_lifecycle: false,
            can_manage_external_lifecycle: false,
            can_manage_resources: false,
            requires_external_desktop_app: true,
          },
        }),
      ),
    ).toMatchObject({
      title: "Docker Desktop",
      classLabel: "External",
      externalAppNote: "Managed by its external desktop app",
    });
  });

  it("labels external remote runtimes from their typed class", () => {
    expect(presentRuntimeProfile(profile({ runtime_class: "external_remote" }))).toMatchObject({
      classLabel: "External remote",
      supportingText: null,
    });
  });

  it.each([
    [binding(), "Ready", null],
    [binding({ state: "unavailable" }), "Unavailable", "The configured runtime is unavailable."],
    [binding({ state: "missing" }), "Missing", "The configured runtime is missing."],
    [
      binding({
        source: "platform_default",
        state: "unconfigured",
        profile_id: null,
        runtime_class: null,
        display_name: "Platform default",
      }),
      "Platform default",
      null,
    ],
  ])(
    "renders binding state %s without fallback claims",
    (value, stateLabel, actionBlockedReason) => {
      expect(presentRuntimeBinding(value)).toMatchObject({ stateLabel, actionBlockedReason });
    },
  );

  it("surfaces ownership conflict and recovery requirements as blocked", () => {
    expect(
      presentRuntimeProfile(
        profile({
          ownership_state: "ownership_conflict",
          management: { ...management, requires_recovery: true, blocks_destructive_actions: true },
        }),
      ),
    ).toMatchObject({
      stateLabel: "Recovery required",
      actionBlockedReason: "Runtime ownership must be recovered before actions can continue.",
    });
  });

  it("bounds a long label while preserving the complete tooltip text", () => {
    const display_name = `Podman machine ${"long-".repeat(18)}name`;
    const result = presentRuntimeProfile(profile({ display_name }));

    expect(result.title.length).toBeLessThan(display_name.length);
    expect(result.tooltip).toBe(display_name);
  });

  it("keeps persisted historical attribution independent from the current profile list", () => {
    expect(
      presentRuntimeAttribution({
        runtime_profile_id: "removed-profile",
        runtime_class: "external_local",
        binding_source: "project_pin",
      }),
    ).toMatchObject({
      title: "Recorded runtime",
      classLabel: "External",
      stateLabel: "Historical",
      supportingText: "Recorded from project pin",
    });
  });
});
