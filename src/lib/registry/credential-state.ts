import { DaemonRequestError } from "$lib/daemon/client";

export type CredentialOperationError = {
  status: number | null;
  message: string;
};

export type CredentialOperationPhase = "idle" | "submitting" | "succeeded" | "failed";

export type CredentialOperationState = {
  phase: CredentialOperationPhase;
  error: CredentialOperationError | null;
  generation: number;
};

const STATUS_MESSAGES: Record<number, string> = {
  400: "Check the registry address and credential fields.",
  401: "You're not authorized to talk to the daemon.",
  404: "This saved credential no longer exists.",
  409: "A credential is already saved for this registry.",
  413: "The credential is too large.",
  500: "The daemon could not complete the credential operation.",
  503: "Credential storage is unavailable on this device.",
};

export function initialCredentialOperation(): CredentialOperationState {
  return resetCredentialOperation(0);
}

export function resetCredentialOperation(generation: number): CredentialOperationState {
  return { phase: "idle", error: null, generation };
}

export function beginCredentialSubmission(state: CredentialOperationState): {
  state: CredentialOperationState;
  clearSensitiveInputs: true;
} {
  return {
    state: { ...state, phase: "submitting", error: null },
    clearSensitiveInputs: true,
  };
}

export function applyCredentialSuccess(
  state: CredentialOperationState,
  requestGeneration: number,
): CredentialOperationState {
  if (requestGeneration !== state.generation) return state;
  return { ...state, phase: "succeeded", error: null };
}

export function applyCredentialFailure(
  state: CredentialOperationState,
  requestGeneration: number,
  error: CredentialOperationError,
): CredentialOperationState {
  if (requestGeneration !== state.generation) return state;
  return { ...state, phase: "failed", error };
}

export function toCredentialOperationError(error: unknown): CredentialOperationError {
  if (error instanceof DaemonRequestError) {
    return {
      status: error.status,
      message:
        STATUS_MESSAGES[error.status] ?? `The credential operation failed (HTTP ${error.status}).`,
    };
  }
  return { status: null, message: "Could not reach the daemon." };
}
