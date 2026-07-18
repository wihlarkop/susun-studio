/**
 * A cached (or errored) detail lookup for one artifact. Always carries the
 * capability the daemon reported for that lookup, including
 * "supported_subset", even when found, so a partial-support detail read is
 * never visually indistinguishable from a fully-supported one.
 */
export type ScopedDetailEntry<T> =
  | { kind: "found"; value: T; capability: string }
  | { kind: "unsupported"; capability: string }
  | { kind: "error"; message: string };

const KEY_SEPARATOR = String.fromCharCode(0);

/**
 * Composite cache key scoping a detail entry to the exact engine it came
 * from. Using the artifact id alone would let a cached (or in-flight, late
 * arriving) result from one engine be read as if it belonged to another -
 * two different engines can have a container or image with the same id.
 * A NUL separator is used since neither an engine id nor an artifact id can
 * contain one, unlike a printable separator such as a colon or slash, which
 * aren't guaranteed to be excluded from either.
 */
export function scopedDetailKey(engineId: string, artifactId: string): string {
  return engineId + KEY_SEPARATOR + artifactId;
}

/** Maps a detail response's capability/value pair into the cache entry to
 * store, preserving whichever capability level the daemon reported either
 * way. */
export function toDetailEntry<T>(response: {
  capability: string;
  value: T | null;
}): ScopedDetailEntry<T> {
  return response.value !== null
    ? { kind: "found", value: response.value, capability: response.capability }
    : { kind: "unsupported", capability: response.capability };
}
