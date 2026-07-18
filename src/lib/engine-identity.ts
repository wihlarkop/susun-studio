/**
 * Matches the daemon's `PLATFORM_DEFAULT_ENGINE_ID`: used only when no
 * runtime profile is selected. Once a profile is selected, the daemon
 * validates `engine_id` against that profile's own id — this string is no
 * longer accepted, so callers must never hardcode it.
 */
export const PLATFORM_DEFAULT_ENGINE_ID = "engine-docker-local";

/**
 * The engine id the daemon actually expects for the currently active
 * runtime: the selected profile's own id, or the platform-default sentinel
 * when none is selected.
 */
export function resolveActiveEngineId(selectedProfileId: string | null | undefined): string {
  return selectedProfileId ?? PLATFORM_DEFAULT_ENGINE_ID;
}
