/**
 * The daemon's `SupportLevel` vocabulary, serialized as these exact
 * snake_case strings (see susun-engine's `SupportLevel`). Kept as a plain
 * string union rather than an enum so an unrecognized future value degrades
 * to the "unknown" label instead of failing to compile.
 */
export type CapabilityLevel =
  | "supported"
  | "supported_subset"
  | "experimental"
  | "unsupported"
  | "unknown";

const KNOWN_LEVELS: ReadonlySet<string> = new Set([
  "supported",
  "supported_subset",
  "experimental",
  "unsupported",
  "unknown",
]);

function asCapabilityLevel(capability: string): CapabilityLevel {
  return KNOWN_LEVELS.has(capability) ? (capability as CapabilityLevel) : "unknown";
}

/** Human-readable label for a capability string, for badges and copy. */
export function capabilityLabel(capability: string): string {
  switch (asCapabilityLevel(capability)) {
    case "supported":
      return "Supported";
    case "supported_subset":
      return "Partial support";
    case "experimental":
      return "Experimental";
    case "unsupported":
      return "Not supported by this engine";
    case "unknown":
      return "Support unknown";
  }
}

/**
 * Whether the provider actually returns usable data for this capability —
 * mirrors the daemon's own `SupportLevel::is_supported()` (`Supported` and
 * `SupportedSubset` only). `Experimental` is deliberately excluded: the
 * daemon only calls the underlying operation when `is_supported()` is true,
 * so an `experimental` capability never carries inventory data even though
 * the provider claims some support for it.
 */
export function isCapabilityUsable(capability: string): boolean {
  const level = asCapabilityLevel(capability);
  return level === "supported" || level === "supported_subset";
}
