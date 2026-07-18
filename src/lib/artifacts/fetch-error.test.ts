import { describe, expect, it } from "vitest";
import { DaemonRequestError } from "$lib/daemon/client";
import { toArtifactRequestError } from "./fetch-error";

describe("toArtifactRequestError", () => {
  it("preserves the HTTP status from a DaemonRequestError", () => {
    const result = toArtifactRequestError(new DaemonRequestError(502, "engine unavailable"));
    expect(result).toEqual({ status: 502, message: "engine unavailable" });
  });

  it("reports a null status for a plain Error (network-level failure)", () => {
    const result = toArtifactRequestError(new Error("Failed to fetch"));
    expect(result).toEqual({ status: null, message: "Failed to fetch" });
  });

  it("stringifies a non-Error throw rather than losing it", () => {
    const result = toArtifactRequestError("something odd");
    expect(result).toEqual({ status: null, message: "something odd" });
  });
});
