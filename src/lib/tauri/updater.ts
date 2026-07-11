import { check } from "@tauri-apps/plugin-updater";
import { relaunch } from "@tauri-apps/plugin-process";
import { isTauri } from "@tauri-apps/api/core";

export type UpdateCheckResult =
  | { available: false; reason?: "not_desktop" | "none" | "unpublished" }
  | { available: true; version: string; install: () => Promise<void> };

export async function checkForUpdate(): Promise<UpdateCheckResult> {
  if (!isTauri()) {
    return { available: false, reason: "not_desktop" };
  }

  let update;
  try {
    update = await check();
  } catch (error) {
    if (isUnpublishedUpdateError(error)) {
      return { available: false, reason: "unpublished" };
    }
    throw error;
  }

  if (!update) {
    return { available: false, reason: "none" };
  }

  return {
    available: true,
    version: update.version,
    install: async () => {
      await update.downloadAndInstall();
      await relaunch();
    },
  };
}

function isUnpublishedUpdateError(error: unknown): boolean {
  const message = error instanceof Error ? error.message : String(error);
  const normalized = message.toLowerCase();
  return (
    normalized.includes("404") ||
    normalized.includes("not found") ||
    normalized.includes("latest.json") ||
    normalized.includes("no release") ||
    normalized.includes("no version")
  );
}
