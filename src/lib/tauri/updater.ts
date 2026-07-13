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
    throw new Error("Update check failed. Verify network access and try again.");
  }

  if (!update) {
    return { available: false, reason: "none" };
  }

  return {
    available: true,
    version: update.version,
    install: async () => {
      try {
        await update.downloadAndInstall();
      } catch {
        throw new Error(
          "Update installation did not complete. Restart Studio, check the installed version, and try again later.",
        );
      }
      try {
        await relaunch();
      } catch {
        throw new Error(
          "The update was installed, but Studio could not relaunch. Restart it manually.",
        );
      }
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
