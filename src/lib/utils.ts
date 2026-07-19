import { clsx, type ClassValue } from "clsx";
import { twMerge } from "tailwind-merge";

export function cn(...inputs: ClassValue[]) {
  return twMerge(clsx(inputs));
}

/**
 * Strips the Windows extended-length prefix (`\\?\` or `\\?\UNC\`) that
 * std::fs::canonicalize adds on the daemon side, for display purposes only.
 */
export function displayPath(path: string): string {
  if (path.startsWith("\\\\?\\UNC\\")) {
    return `\\\\${path.slice(8)}`;
  }
  if (path.startsWith("\\\\?\\")) {
    return path.slice(4);
  }
  return path;
}

export function formatTimestamp(ms: number): string {
  return new Date(ms).toLocaleString();
}

export function formatBytes(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  const units = ["KB", "MB", "GB", "TB"];
  let value = bytes / 1024;
  let unitIndex = 0;
  while (value >= 1024 && unitIndex < units.length - 1) {
    value /= 1024;
    unitIndex += 1;
  }
  return `${value.toFixed(1)} ${units[unitIndex]}`;
}

const relativeTimeFormat = new Intl.RelativeTimeFormat(undefined, { numeric: "auto" });
const relativeTimeSteps: [limit: number, divisor: number, unit: Intl.RelativeTimeFormatUnit][] = [
  [60_000, 1_000, "second"],
  [3_600_000, 60_000, "minute"],
  [86_400_000, 3_600_000, "hour"],
  [Infinity, 86_400_000, "day"],
];

export function relativeTime(ms: number, now = Date.now()): string {
  const elapsed = ms - now;
  const [, divisor, unit] =
    relativeTimeSteps.find(([limit]) => Math.abs(elapsed) < limit) ??
    relativeTimeSteps[relativeTimeSteps.length - 1];
  return relativeTimeFormat.format(Math.round(elapsed / divisor), unit);
}

// eslint-disable-next-line @typescript-eslint/no-explicit-any
export type WithoutChild<T> = T extends { child?: any } ? Omit<T, "child"> : T;
// eslint-disable-next-line @typescript-eslint/no-explicit-any
export type WithoutChildren<T> = T extends { children?: any } ? Omit<T, "children"> : T;
export type WithoutChildrenOrChild<T> = WithoutChildren<WithoutChild<T>>;
export type WithElementRef<T, U extends HTMLElement = HTMLElement> = T & { ref?: U | null };
