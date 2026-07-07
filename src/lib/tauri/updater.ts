import { check } from "@tauri-apps/plugin-updater";
import { relaunch } from "@tauri-apps/plugin-process";
import { isTauri } from "@tauri-apps/api/core";

export type UpdateCheckResult =
  | { available: false }
  | { available: true; version: string; install: () => Promise<void> };

export async function checkForUpdate(): Promise<UpdateCheckResult> {
  if (!isTauri()) {
    return { available: false };
  }

  const update = await check();
  if (!update) {
    return { available: false };
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
