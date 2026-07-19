import { describe, expect, it } from "vitest";
import { resolveArtifactViewState, type ArtifactViewStateInput } from "./workspace-state";

const base: ArtifactViewStateInput = {
  connected: true,
  loading: false,
  hasData: false,
  error: null,
  capability: null,
  itemCount: null,
};

describe("resolveArtifactViewState", () => {
  it("reports disconnected regardless of any other input", () => {
    const state = resolveArtifactViewState({
      ...base,
      connected: false,
      hasData: true,
      capability: "supported",
      itemCount: 3,
    });
    expect(state).toEqual({ kind: "disconnected" });
  });

  it("reports loading before any data or error has arrived", () => {
    const state = resolveArtifactViewState({ ...base, loading: true });
    expect(state).toEqual({ kind: "loading" });
  });

  it("reports unreachable for a 502 with no data yet", () => {
    const state = resolveArtifactViewState({
      ...base,
      error: { status: 502, message: "engine unavailable: connection refused" },
    });
    expect(state).toEqual({
      kind: "unreachable",
      error: { status: 502, message: "engine unavailable: connection refused" },
    });
  });

  it("reports request-error for a non-502 failure with no data yet", () => {
    const state = resolveArtifactViewState({
      ...base,
      error: { status: 500, message: "database error" },
    });
    expect(state.kind).toBe("request-error");
  });

  it("reports request-error for a network-level failure (no HTTP status)", () => {
    const state = resolveArtifactViewState({
      ...base,
      error: { status: null, message: "Failed to fetch" },
    });
    expect(state.kind).toBe("request-error");
  });

  it("reports unsupported once data has loaded but the capability isn't usable", () => {
    const state = resolveArtifactViewState({
      ...base,
      hasData: true,
      capability: "unsupported",
      itemCount: 0,
    });
    expect(state).toEqual({ kind: "unsupported", capability: "unsupported" });
  });

  it("prefers unsupported over empty when both would otherwise apply", () => {
    const state = resolveArtifactViewState({
      ...base,
      hasData: true,
      capability: "experimental",
      itemCount: 0,
    });
    expect(state.kind).toBe("unsupported");
  });

  it("reports empty for a usable capability with zero items", () => {
    const state = resolveArtifactViewState({
      ...base,
      hasData: true,
      capability: "supported",
      itemCount: 0,
    });
    expect(state).toEqual({ kind: "empty" });
  });

  it("never reports empty for a non-list surface (itemCount null)", () => {
    const state = resolveArtifactViewState({
      ...base,
      hasData: true,
      capability: "supported",
      itemCount: null,
    });
    expect(state.kind).toBe("ready");
  });

  it("reports ready for a usable capability with items present", () => {
    const state = resolveArtifactViewState({
      ...base,
      hasData: true,
      capability: "supported_subset",
      itemCount: 4,
    });
    expect(state).toEqual({ kind: "ready" });
  });

  it("reports refreshing when data is held and a new fetch is in flight", () => {
    const state = resolveArtifactViewState({
      ...base,
      hasData: true,
      loading: true,
      capability: "supported",
      itemCount: 2,
    });
    expect(state.kind).toBe("refreshing");
  });

  it("reports stale when data is held but the latest refresh failed", () => {
    const state = resolveArtifactViewState({
      ...base,
      hasData: true,
      capability: "supported",
      itemCount: 2,
      error: { status: 502, message: "engine unavailable" },
    });
    expect(state).toEqual({
      kind: "stale",
      error: { status: 502, message: "engine unavailable" },
    });
  });

  it("prefers stale over refreshing when both a stale error and a new fetch are present", () => {
    // A retry after a failure re-sets `loading` while the old error is still
    // being displayed — stale must win so the failure doesn't just vanish
    // silently while the retry is in flight.
    const state = resolveArtifactViewState({
      ...base,
      hasData: true,
      loading: true,
      capability: "supported",
      itemCount: 2,
      error: { status: 502, message: "engine unavailable" },
    });
    expect(state.kind).toBe("stale");
  });

  it("treats a registry-style surface (no top-level capability) as ready once data loads", () => {
    const state = resolveArtifactViewState({
      ...base,
      hasData: true,
      capability: null,
      itemCount: null,
    });
    expect(state).toEqual({ kind: "ready" });
  });
});
