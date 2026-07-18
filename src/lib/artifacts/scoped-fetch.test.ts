import { describe, expect, it } from "vitest";
import {
  applyLoadError,
  applyLoadSuccess,
  applyNotConnected,
  initialScopedFetchState,
  resetForNewEngine,
  withLoading,
} from "./scoped-fetch";

describe("resetForNewEngine", () => {
  it("clears data, loading, and error, and bumps the generation", () => {
    const loaded = applyLoadSuccess(withLoading(initialScopedFetchState<{ id: string }>()), 0, {
      id: "runtime-a-data",
    });
    expect(loaded.data).not.toBeNull();

    const reset = resetForNewEngine(loaded);

    expect(reset.data).toBeNull();
    expect(reset.loading).toBe(false);
    expect(reset.error).toBeNull();
    expect(reset.generation).toBe(loaded.generation + 1);
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
    // Simulates: request started under generation 0, user switches engines
    // (bumping to generation 1) before the response for generation 0 lands.
    const initial = initialScopedFetchState<string>();
    const afterSwitch = resetForNewEngine(initial); // now generation 1, for engine B

    const result = applyLoadSuccess(
      afterSwitch,
      0 /* stale, engine A's generation */,
      "engine-a-data",
    );

    expect(result).toBe(afterSwitch); // unchanged — the late response never wrote through
    expect(result.data).toBeNull();
  });

  it("ignores a late error completion from a superseded generation the same way", () => {
    const initial = initialScopedFetchState<string>();
    const afterSwitch = resetForNewEngine(initial);

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

describe("applyNotConnected", () => {
  it("clears loading for the current generation", () => {
    const state = withLoading(initialScopedFetchState<string>());
    const next = applyNotConnected(state, state.generation);
    expect(next.loading).toBe(false);
  });

  it("ignores a stale not-connected completion the same way as success/error", () => {
    const initial = withLoading(initialScopedFetchState<string>());
    const afterSwitch = resetForNewEngine(initial);
    const result = applyNotConnected(afterSwitch, initial.generation);
    expect(result).toBe(afterSwitch);
  });
});
