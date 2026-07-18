import { describe, expect, it } from "vitest";
import { capabilityLabel, isCapabilityUsable } from "./capability";

describe("isCapabilityUsable", () => {
  it("is usable for supported and supported_subset", () => {
    expect(isCapabilityUsable("supported")).toBe(true);
    expect(isCapabilityUsable("supported_subset")).toBe(true);
  });

  it("is not usable for experimental, unsupported, or unknown", () => {
    // Experimental is deliberately grouped with unsupported/unknown: the
    // daemon only calls the underlying SDK operation when its own
    // `SupportLevel::is_supported()` is true, which excludes Experimental —
    // so an experimental capability never actually carries inventory data.
    expect(isCapabilityUsable("experimental")).toBe(false);
    expect(isCapabilityUsable("unsupported")).toBe(false);
    expect(isCapabilityUsable("unknown")).toBe(false);
  });

  it("treats an unrecognized capability string as not usable", () => {
    expect(isCapabilityUsable("some_future_level")).toBe(false);
  });
});

describe("capabilityLabel", () => {
  it("gives each known level a distinct, accurate label", () => {
    expect(capabilityLabel("supported")).toBe("Supported");
    expect(capabilityLabel("supported_subset")).toBe("Partial support");
    expect(capabilityLabel("experimental")).toBe("Experimental");
    expect(capabilityLabel("unsupported")).toBe("Not supported by this engine");
    expect(capabilityLabel("unknown")).toBe("Support unknown");
  });

  it("falls back to the unknown label for an unrecognized string", () => {
    expect(capabilityLabel("some_future_level")).toBe("Support unknown");
  });
});
