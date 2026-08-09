import { invoke, isTauri } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type { RuntimeProfile, RuntimeStatus } from "$lib/daemon/client";

export type TrayRuntimeSummary = {
  title: string;
  kind: "built_in_managed" | "external" | "unconfigured";
  state: "ready" | "stopped" | "unavailable" | "missing" | "unconfigured";
};

export type TrayNavigationIntent = "open" | "runtime_settings" | "runtime_setup" | "runtime";
export type TrayRuntimeAction = "start" | "stop";

export function toTrayRuntimeSummary(status: RuntimeStatus | undefined): TrayRuntimeSummary {
  if (!status || status.policy.binding.state === "unconfigured") {
    return { title: "Runtime not configured", kind: "unconfigured", state: "unconfigured" };
  }

  const profile = selectedProfile(status);
  if (!profile) {
    return {
      title: status.policy.binding.display_name,
      kind: "external",
      state: bindingState(status.policy.binding.state),
    };
  }

  const state = profileState(profile);
  const isManagedBuiltIn =
    profile.runtime_class === "built_in" &&
    profile.ownership_state === "studio_managed" &&
    !profile.management.requires_recovery;
  return {
    title: profile.runtime_class === "built_in" ? "Susun Runtime" : profile.display_name,
    kind: isManagedBuiltIn ? "built_in_managed" : "external",
    state,
  };
}

export async function syncTrayRuntimeSummary(status: RuntimeStatus | undefined): Promise<void> {
  if (!isTauri()) return;
  try {
    await invoke("update_tray_runtime_summary", { summary: toTrayRuntimeSummary(status) });
  } catch {
    // Tray synchronization is best-effort. The daemon's shared state remains
    // authoritative and a later refresh will retry with a bounded summary.
  }
}

export async function listenForTrayRequests(handlers: {
  onNavigation: (intent: TrayNavigationIntent) => void;
  onRuntimeAction: (action: TrayRuntimeAction) => void;
}): Promise<UnlistenFn> {
  if (!isTauri()) return () => {};
  const [unlistenNavigation, unlistenAction] = await Promise.all([
    listen<TrayNavigationIntent>("studio-tray-navigation-requested", (event) =>
      handlers.onNavigation(event.payload),
    ),
    listen<TrayRuntimeAction>("runtime-tray-action-requested", (event) =>
      handlers.onRuntimeAction(event.payload),
    ),
  ]);
  return () => {
    unlistenNavigation();
    unlistenAction();
  };
}

function selectedProfile(status: RuntimeStatus): RuntimeProfile | undefined {
  const profileId = status.policy.binding.profile_id;
  return profileId
    ? status.providers
        .flatMap((provider) => provider.profiles)
        .find((profile) => profile.id === profileId)
    : undefined;
}

function bindingState(
  state: RuntimeStatus["policy"]["binding"]["state"],
): TrayRuntimeSummary["state"] {
  switch (state) {
    case "ready":
      return "ready";
    case "missing":
      return "missing";
    case "unavailable":
      return "unavailable";
    case "unconfigured":
      return "unconfigured";
  }
}

function profileState(profile: RuntimeProfile): TrayRuntimeSummary["state"] {
  if (profile.availability_state === "missing") return "missing";
  if (profile.availability_state !== "available" || profile.management.requires_recovery) {
    return "unavailable";
  }
  return profile.process.state === "running" ? "ready" : "stopped";
}
