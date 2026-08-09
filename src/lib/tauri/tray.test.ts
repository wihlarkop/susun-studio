import { describe, expect, it } from "vitest";
import type { RuntimeStatus } from "$lib/daemon/client";
import { toTrayRuntimeSummary } from "./tray";

function status(overrides: Record<string, unknown> = {}): RuntimeStatus {
  return {
    policy: {
      preferred_profile_id: "managed",
      binding: {
        source: "global_preference",
        state: "ready",
        profile_id: "managed",
        runtime_class: "built_in",
        display_name: "Susun Runtime",
      },
    },
    providers: [
      {
        provider_id: "windows-podman",
        display_name: "Podman",
        product: "podman",
        platform: "windows",
        supported: true,
        experience: {
          can_create_builtin: true,
          can_discover_external: true,
          can_manage_builtin_lifecycle: true,
          can_manage_external_lifecycle: false,
          can_manage_resources: true,
          requires_external_desktop_app: false,
        },
        installation: { state: "installed", detail: null },
        process: { state: "running", detail: null },
        connection: { state: "summarized", detail: null },
        freshness: "fresh",
        summary: "Ready",
        remediation: [],
        actions: [],
        profiles: [
          {
            id: "managed",
            provider_id: "windows-podman",
            provider_runtime_key: "podman-machine-default",
            display_name: "Susun Runtime",
            product: "podman",
            platform: "windows",
            runtime_class: "built_in",
            ownership_state: "studio_managed",
            source: "studio_setup",
            installation: { state: "installed", detail: null },
            process: { state: "running", detail: null },
            connection: { state: "summarized", detail: null },
            endpoint_summary: null,
            availability_state: "available",
            last_seen_at_ms: null,
            missing_since_ms: null,
            last_error: null,
            is_preferred: true,
            observation_revision: 1,
            observed_at_ms: 1,
            management: {
              can_select: true,
              can_forget: false,
              can_adopt: false,
              requires_recovery: false,
              blocks_destructive_actions: false,
            },
            freshness: "fresh",
          },
        ],
      },
    ],
    ...overrides,
  } as RuntimeStatus;
}

describe("tray runtime summary", () => {
  it("maps only an observed Studio-managed built-in runtime to lifecycle-capable state", () => {
    expect(toTrayRuntimeSummary(status())).toEqual({
      title: "Susun Runtime",
      kind: "built_in_managed",
      state: "ready",
    });
  });

  it("keeps external runtimes external regardless of their display text", () => {
    const next = status();
    next.providers[0].profiles[0] = {
      ...next.providers[0].profiles[0],
      display_name: "Susun Runtime (managed)",
      runtime_class: "external_local",
      ownership_state: "external",
      source: "provider_discovery",
    };
    next.policy.binding = {
      ...next.policy.binding,
      runtime_class: "external_local",
      display_name: "Susun Runtime (managed)",
    };

    expect(toTrayRuntimeSummary(next)).toMatchObject({ kind: "external", state: "ready" });
  });

  it("maps ownership conflict and missing profiles to states with no lifecycle capability", () => {
    const conflicted = status();
    conflicted.providers[0].profiles[0] = {
      ...conflicted.providers[0].profiles[0],
      ownership_state: "ownership_conflict",
    };
    const missing = status();
    missing.providers[0].profiles[0] = {
      ...missing.providers[0].profiles[0],
      availability_state: "missing",
    };

    expect(toTrayRuntimeSummary(conflicted)).toMatchObject({ kind: "external" });
    expect(toTrayRuntimeSummary(missing)).toMatchObject({ state: "missing" });
  });
});
