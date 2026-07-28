import type { MutationPhase } from "$lib/artifacts/mutation-state";

export type PushPreviewBinding = {
  engineId: string;
  imageId: string;
  destination: string;
  credentialId: string | null;
};

export function pushPreviewBinding(
  engineId: string,
  imageId: string,
  destination: string,
  credentialId: string | null,
): PushPreviewBinding {
  return {
    engineId,
    imageId,
    destination: destination.trim(),
    credentialId,
  };
}

export function canCommitPush(
  phase: MutationPhase,
  preview: { commit_enabled: boolean; plan_id: string | null } | null,
  previewed: PushPreviewBinding | null,
  current: PushPreviewBinding,
): boolean {
  return (
    phase === "previewed" &&
    preview?.commit_enabled === true &&
    Boolean(preview.plan_id) &&
    previewed !== null &&
    previewed.engineId === current.engineId &&
    previewed.imageId === current.imageId &&
    previewed.destination === current.destination &&
    previewed.credentialId === current.credentialId
  );
}

export function pushResultDigest(result: { digest: string | null }): string {
  return result.digest ?? "Not reported by registry";
}
