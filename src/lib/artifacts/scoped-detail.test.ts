import { describe, expect, it } from "vitest";
import { scopedDetailKey, toDetailEntry } from "./scoped-detail";

describe("scopedDetailKey", () => {
  it("produces different keys for the same artifact id on two different engines", () => {
    const keyOnA = scopedDetailKey("profile-runtime-a", "c1");
    const keyOnB = scopedDetailKey("profile-runtime-b", "c1");
    expect(keyOnA).not.toBe(keyOnB);
  });

  it("produces the same key for the same engine and artifact id", () => {
    expect(scopedDetailKey("profile-runtime-a", "c1")).toBe(
      scopedDetailKey("profile-runtime-a", "c1"),
    );
  });

  it("cannot be spoofed by concatenation across the id boundary", () => {
    // If the key were a naive `${engineId}:${artifactId}` (or similar
    // printable separator) join, ("profile-runtime-a:extra", "c1") and
    // ("profile-runtime-a", "extra:c1") could collide. The NUL separator
    // cannot appear in either id, so this must never collide.
    const a = scopedDetailKey("profile-runtime-a:extra", "c1");
    const b = scopedDetailKey("profile-runtime-a", "extra:c1");
    expect(a).not.toBe(b);
  });
});

describe("toDetailEntry", () => {
  it("maps a present value to found, preserving the capability", () => {
    const entry = toDetailEntry({ capability: "supported_subset", value: { id: "c1" } });
    expect(entry).toEqual({ kind: "found", value: { id: "c1" }, capability: "supported_subset" });
  });

  it("maps a null value to unsupported, preserving the capability", () => {
    const entry = toDetailEntry<{ id: string }>({ capability: "unsupported", value: null });
    expect(entry).toEqual({ kind: "unsupported", capability: "unsupported" });
  });

  it("preserves a fully supported capability distinctly from supported_subset", () => {
    const full = toDetailEntry({ capability: "supported", value: { id: "c1" } });
    const partial = toDetailEntry({ capability: "supported_subset", value: { id: "c1" } });
    expect(full.kind).toBe("found");
    expect(partial.kind).toBe("found");
    if (full.kind !== "found" || partial.kind !== "found") throw new Error("unreachable");
    expect(full.capability).not.toBe(partial.capability);
  });
});
