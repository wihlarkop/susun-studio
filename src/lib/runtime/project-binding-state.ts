import type { RuntimeBindingSummary } from "$lib/daemon/client";

export type ProjectBindingPresentation = {
  selectorLabel: "Use preferred runtime" | "Pinned runtime";
  bindingLabel: "Preferred runtime" | "Pinned runtime" | "Platform default";
  blocked: boolean;
  blockedReason: string | null;
  canChange: true;
  canClear: boolean;
  keepProjectReadable: true;
};

export function presentProjectBinding(binding: RuntimeBindingSummary): ProjectBindingPresentation {
  const pinned = binding.source === "project_pin";
  const subject = pinned ? "pinned runtime" : "preferred runtime";
  const blockedReason = blockedReasonFor(binding.state, subject);

  return {
    selectorLabel: pinned ? "Pinned runtime" : "Use preferred runtime",
    bindingLabel:
      binding.source === "platform_default"
        ? "Platform default"
        : pinned
          ? "Pinned runtime"
          : "Preferred runtime",
    blocked: blockedReason !== null,
    blockedReason,
    canChange: true,
    canClear: pinned,
    keepProjectReadable: true,
  };
}

function blockedReasonFor(
  state: RuntimeBindingSummary["state"],
  subject: "pinned runtime" | "preferred runtime",
): string | null {
  if (state === "missing") {
    return `The ${subject} is missing.`;
  }
  if (state === "unavailable") {
    return `The ${subject} is unavailable.`;
  }
  return null;
}
