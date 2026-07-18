import { describe, expect, it } from "vitest";
import { DaemonRequestError } from "$lib/daemon/client";
import { toArtifactRequestError } from "./fetch-error";

describe("toArtifactRequestError", () => {
  it("preserves the HTTP status from a DaemonRequestError", () => {
    const result = toArtifactRequestError(new DaemonRequestError(502, "engine unavailable"));
    expect(result.status).toBe(502);
  });

  it("gives each known status a distinct, fixed message", () => {
    expect(toArtifactRequestError(new DaemonRequestError(401, "x")).message).toMatch(
      /not authorized/i,
    );
    expect(toArtifactRequestError(new DaemonRequestError(404, "x")).message).toMatch(
      /no longer visible/i,
    );
    expect(toArtifactRequestError(new DaemonRequestError(422, "x")).message).toMatch(
      /could not be completed/i,
    );
    expect(toArtifactRequestError(new DaemonRequestError(500, "x")).message).toMatch(
      /internal error/i,
    );
    expect(toArtifactRequestError(new DaemonRequestError(502, "x")).message).toMatch(
      /could not be reached/i,
    );
  });

  it("falls back to a generic status-shaped message for an unlisted status", () => {
    const result = toArtifactRequestError(new DaemonRequestError(418, "x"));
    expect(result.message).toContain("418");
  });

  it("reports a null status for a network-level failure", () => {
    const result = toArtifactRequestError(new Error("Failed to fetch"));
    expect(result.status).toBeNull();
  });

  it("never forwards the daemon's raw error body text, even when it looks sensitive", () => {
    const sensitiveBodies = [
      "engine unavailable: connection refused at unix:///var/run/docker.sock",
      "engine unavailable: \\\\.\\pipe\\docker_engine access denied",
      "database error: FOREIGN KEY constraint failed at C:\\Users\\edo\\AppData\\studio.db",
      "planning failed: DATABASE_URL=postgres://user:hunter2@host/db is invalid",
    ];
    for (const body of sensitiveBodies) {
      const result = toArtifactRequestError(new DaemonRequestError(502, body));
      expect(result.message).not.toContain("docker.sock");
      expect(result.message).not.toContain("pipe");
      expect(result.message).not.toContain("AppData");
      expect(result.message).not.toContain("hunter2");
      expect(result.message).not.toBe(body);
    }
  });

  it("never forwards a raw network-layer error message either", () => {
    const result = toArtifactRequestError(
      new Error("NetworkError: could not resolve host internal.corp"),
    );
    expect(result.message).not.toContain("internal.corp");
  });

  it("stringifies a non-Error, non-DaemonRequestError throw into the same bounded shape", () => {
    const result = toArtifactRequestError("something odd");
    expect(result).toEqual({ status: null, message: "Could not reach the daemon." });
  });
});
