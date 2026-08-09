export type OnboardingStateSnapshot = {
  state: "pending" | "completed" | "dismissed";
  choice: "built_in" | "existing" | null;
  completed_at_ms: number | null;
  updated_at_ms: number;
};

type RuntimeBindingSnapshot = {
  state: string;
  profile_id: string | null;
};

type RuntimeProfileSnapshot = {
  id: string;
  runtime_class: string;
  ownership_state?: string;
  management: { can_select: boolean };
};

export function resolveOnboardingView(input: {
  connected: boolean;
  onboarding: OnboardingStateSnapshot | undefined;
  binding?: RuntimeBindingSnapshot;
}): { kind: "wait" | "chooser" | "hidden" } {
  if (!input.connected || !input.onboarding) return { kind: "wait" };
  if (input.onboarding.state === "pending" && input.onboarding.choice === null) {
    return { kind: "chooser" };
  }
  return { kind: "hidden" };
}

export function canCompleteBuiltInOnboarding(input: {
  onboarding: OnboardingStateSnapshot;
  binding: RuntimeBindingSnapshot;
  profiles: RuntimeProfileSnapshot[];
}): boolean {
  if (input.onboarding.state !== "pending" || input.onboarding.choice !== null) return false;
  if (input.binding.state !== "ready" || !input.binding.profile_id) return false;
  return input.profiles.some(
    (profile) =>
      profile.id === input.binding.profile_id &&
      profile.runtime_class === "built_in" &&
      profile.ownership_state === "studio_managed" &&
      profile.management.can_select,
  );
}

export function selectableExternalProfiles(profiles: RuntimeProfileSnapshot[]): string[] {
  return profiles
    .filter((profile) => profile.runtime_class !== "built_in" && profile.management.can_select)
    .map((profile) => profile.id);
}

export function canDismissInitialOnboarding(input: {
  reopened: boolean;
  onboarding: OnboardingStateSnapshot;
}): boolean {
  return !input.reopened && input.onboarding.state === "pending" && input.onboarding.choice === null;
}
