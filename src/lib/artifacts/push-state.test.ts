import { describe, expect, test } from "vitest";

import { canCommitPush, pushPreviewBinding, pushResultDigest } from "$lib/artifacts/push-state";

const preview = {
  commit_enabled: true,
  plan_id: "rap_push",
};

describe("trusted push state", () => {
  test("requires destination, credential, engine, and image to match the preview", () => {
    const binding = pushPreviewBinding("engine-a", "image-a", "registry.example/app:v1", "cred-a");
    expect(canCommitPush("previewed", preview, binding, binding)).toBe(true);
    expect(
      canCommitPush("previewed", preview, binding, {
        ...binding,
        destination: "registry.example/app:v2",
      }),
    ).toBe(false);
    expect(canCommitPush("previewed", preview, binding, { ...binding, credentialId: null })).toBe(
      false,
    );
    expect(canCommitPush("previewed", preview, binding, { ...binding, engineId: "engine-b" })).toBe(
      false,
    );
    expect(canCommitPush("previewed", preview, binding, { ...binding, imageId: "image-b" })).toBe(
      false,
    );
  });

  test("never replays a plan after any commit attempt", () => {
    const binding = pushPreviewBinding("engine-a", "image-a", "app:v1", null);
    expect(canCommitPush("committing", preview, binding, binding)).toBe(false);
    expect(canCommitPush("commit_failed", preview, binding, binding)).toBe(false);
    expect(canCommitPush("succeeded", preview, binding, binding)).toBe(false);
  });

  test("renders provider digest separately and admits when none was reported", () => {
    expect(pushResultDigest({ digest: "sha256:abc" })).toBe("sha256:abc");
    expect(pushResultDigest({ digest: null })).toBe("Not reported by registry");
  });
});
