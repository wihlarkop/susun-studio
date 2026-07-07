import { invoke, isTauri } from "@tauri-apps/api/core";
import { setDaemonConnection } from "$lib/daemon/client";

type DaemonConnectionPayload = {
  base_url: string;
  token: string;
};

export async function initDaemonConnection(): Promise<void> {
  if (!isTauri()) {
    return;
  }

  try {
    const connection = await invoke<DaemonConnectionPayload>("resolve_daemon_connection");
    setDaemonConnection({ baseUrl: connection.base_url, token: connection.token });
  } catch (error) {
    console.error("failed to resolve daemon connection from the Tauri shell", error);
  }
}
