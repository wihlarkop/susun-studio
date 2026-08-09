import type { RuntimeBindingSummary } from "$lib/daemon/client";

/**
 * Matches the daemon's `PLATFORM_DEFAULT_ENGINE_ID`: used only for the
 * explicit, unconfigured compatibility state. A configured preference or
 * project pin that is missing or unavailable has no engine id to call.
 */
export const PLATFORM_DEFAULT_ENGINE_ID = "engine-docker-local";

/**
 * The engine id the daemon expects for this policy decision. Returning
 * `null` for missing/unavailable bindings makes a caller stop rather than
 * silently sending work to the platform default.
 */
export function resolveActiveEngineId(binding: RuntimeBindingSummary): string | null {
  if (binding.source === "platform_default" && binding.state === "unconfigured") {
    return PLATFORM_DEFAULT_ENGINE_ID;
  }

  return binding.state === "ready" ? binding.profile_id : null;
}
