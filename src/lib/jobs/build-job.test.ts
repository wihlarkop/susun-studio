import { describe, expect, it } from "vitest";
import type { BuildProgressEntry, ImageBuildResult, JobExecutionResult } from "$lib/daemon/client";
import { isBuildJobActive, isImageBuildResult, visibleBuildProgress } from "./build-job";

const executionResult: JobExecutionResult = {
  summary: { total_actions: 1, succeeded: 1, failed: 0, skipped: 0, cancelled: 0 },
};

const buildResult: ImageBuildResult = {
  image_reference: "myapp-web:latest",
  image_digest: null,
};

describe("isImageBuildResult", () => {
  it("returns true for an image-build result", () => {
    expect(isImageBuildResult(buildResult)).toBe(true);
  });

  it("returns false for an up/down/build/clean execution result", () => {
    expect(isImageBuildResult(executionResult)).toBe(false);
  });

  it("returns false for null", () => {
    expect(isImageBuildResult(null)).toBe(false);
  });
});

describe("isBuildJobActive", () => {
  it("treats queued and running as active", () => {
    expect(isBuildJobActive("queued")).toBe(true);
    expect(isBuildJobActive("running")).toBe(true);
  });

  it("treats every terminal status as not active", () => {
    expect(isBuildJobActive("succeeded")).toBe(false);
    expect(isBuildJobActive("failed")).toBe(false);
    expect(isBuildJobActive("cancelled")).toBe(false);
  });
});

describe("visibleBuildProgress", () => {
  function entry(sequence: number): BuildProgressEntry {
    return {
      sequence,
      kind: "vertex_log",
      vertex_id: "v1",
      log_stream: "stdout",
      text: `line ${sequence}`,
      status: null,
      current: null,
      total: null,
      created_at_ms: sequence,
    };
  }

  it("returns everything unchanged when under the limit", () => {
    const entries = [entry(0), entry(1), entry(2)];
    const result = visibleBuildProgress(entries, 200);
    expect(result).toEqual({ visible: entries, hiddenCount: 0 });
  });

  it("returns everything unchanged when exactly at the limit", () => {
    const entries = [entry(0), entry(1)];
    const result = visibleBuildProgress(entries, 2);
    expect(result).toEqual({ visible: entries, hiddenCount: 0 });
  });

  /** DOM growth is what this whole function exists to bound: once the
   * server's own 500-row cap is approached, the frontend must still only
   * ever render a fixed-size window, not the full history. */
  it("keeps only the most recent entries, in their original order, and reports how many were hidden", () => {
    const entries = [entry(0), entry(1), entry(2), entry(3), entry(4)];
    const result = visibleBuildProgress(entries, 2);
    expect(result.visible).toEqual([entry(3), entry(4)]);
    expect(result.hiddenCount).toBe(3);
  });

  it("handles an empty list", () => {
    const result = visibleBuildProgress([], 200);
    expect(result).toEqual({ visible: [], hiddenCount: 0 });
  });
});
