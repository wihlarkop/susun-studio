import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";

const packageJson = JSON.parse(readFileSync(new URL("../package.json", import.meta.url), "utf8"));
const tauriConfig = JSON.parse(
  readFileSync(new URL("../src-tauri/tauri.conf.json", import.meta.url), "utf8"),
);

describe("daemon sidecar development startup", () => {
  it("rebuilds the debug daemon before starting the Tauri frontend", () => {
    expect(packageJson.scripts["build:sidecar:dev"]).toBe(
      "node scripts/build-daemon-sidecar.mjs --dev",
    );
    expect(tauriConfig.build.beforeDevCommand).toBe("bun run build:sidecar:dev && bun run dev");
  });
});
