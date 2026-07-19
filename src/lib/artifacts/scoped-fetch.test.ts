import { describe, expect, it } from "vitest";
import {
  applyLoadError,
  applyLoadSuccess,
  initialScopedFetchState,
  resetForNewEngine,
  withLoading,
} from "./scoped-fetch";

describe("resetForNewEngine", () => {
  it("builds a cleared state from a plain generation number, not a previous state value", () => {
    // The whole point of taking a plain number instead of a
    // ScopedFetchState is that this must be callable without reading any
    // reactive value - callers pass an already-advanced counter in.
    const reset = resetForNewEngine<{ id: string }>(3, true);

    expect(reset).toEqual({ data: null, loading: true, error: null, generation: 3 });
  });

  it("supports constructing a non-loading cleared state (the not-connected branch)", () => {
    const reset = resetForNewEngine<{ id: string }>(1, false);
    expect(reset.loading).toBe(false);
  });
});

describe("applyLoadSuccess / applyLoadError", () => {
  it("applies a completion whose generation matches the current one", () => {
    const state = initialScopedFetchState<string>();
    const next = applyLoadSuccess(state, state.generation, "data");
    expect(next.data).toBe("data");
    expect(next.loading).toBe(false);
  });

  it("ignores a late completion from a superseded (previous engine's) generation", () => {
    // Simulates: a request started under generation 0, the user switches
    // engines (advancing to generation 1) before the response for
    // generation 0 lands.
    const afterSwitch = resetForNewEngine<string>(1, true);

    const result = applyLoadSuccess(
      afterSwitch,
      0 /* stale, engine A's generation */,
      "engine-a-data",
    );

    expect(result).toBe(afterSwitch); // unchanged — the late response never wrote through
    expect(result.data).toBeNull();
  });

  it("ignores a late error completion from a superseded generation the same way", () => {
    const afterSwitch = resetForNewEngine<string>(1, true);

    const result = applyLoadError(afterSwitch, 0, { status: 502, message: "unreachable" });

    expect(result).toBe(afterSwitch);
    expect(result.error).toBeNull();
  });

  it("a same-generation error still applies normally", () => {
    const state = initialScopedFetchState<string>();
    const next = applyLoadError(state, state.generation, {
      status: 500,
      message: "internal error",
    });
    expect(next.error).toEqual({ status: 500, message: "internal error" });
    expect(next.loading).toBe(false);
  });
});

describe("withLoading", () => {
  it("marks loading without clearing existing data or error", () => {
    const state = applyLoadError(
      applyLoadSuccess(initialScopedFetchState<string>(), 0, "data"),
      0,
      { status: 502, message: "unreachable" },
    );
    const next = withLoading(state);
    expect(next.loading).toBe(true);
    expect(next.data).toBe("data");
    expect(next.error).toEqual({ status: 502, message: "unreachable" });
  });
});
