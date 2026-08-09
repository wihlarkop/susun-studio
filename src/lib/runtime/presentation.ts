import type {
  RuntimeBindingSource,
  RuntimeBindingSummary,
  RuntimeProfile,
  RuntimeProviderStatus,
} from "$lib/daemon/client";

export type RuntimePresentationTone = "positive" | "neutral" | "warning" | "danger";

export type RuntimePresentation = {
  title: string;
  tooltip: string;
  supportingText: string | null;
  classLabel: "Built-in" | "External" | "External remote" | "Platform default" | "Recorded runtime";
  stateLabel: string;
  tone: RuntimePresentationTone;
  actionBlockedReason: string | null;
  externalAppNote: string | null;
};

const MAX_RUNTIME_LABEL_LENGTH = 42;

export function isRuntimeBindingRequestable(binding: RuntimeBindingSummary): boolean {
  return (
    binding.state === "ready" ||
    (binding.source === "platform_default" && binding.state === "unconfigured")
  );
}

export function presentRuntimeBinding(binding: RuntimeBindingSummary): RuntimePresentation {
  const classLabel = classLabelFor(binding.runtime_class);
  const title = binding.runtime_class === "built_in" ? "Susun Runtime" : binding.display_name;
  const tooltip = title;

  switch (binding.state) {
    case "ready":
      return presentation({
        title,
        tooltip,
        classLabel,
        stateLabel: "Ready",
        tone: "positive",
      });
    case "unavailable":
      return presentation({
        title,
        tooltip,
        classLabel,
        stateLabel: "Unavailable",
        tone: "warning",
        actionBlockedReason: "The configured runtime is unavailable.",
      });
    case "missing":
      return presentation({
        title,
        tooltip,
        classLabel,
        stateLabel: "Missing",
        tone: "danger",
        actionBlockedReason: "The configured runtime is missing.",
      });
    case "unconfigured":
      return presentation({
        title: binding.source === "platform_default" ? "Platform default" : title,
        tooltip: binding.source === "platform_default" ? "Platform default" : tooltip,
        classLabel: "Platform default",
        stateLabel: "Platform default",
        tone: "neutral",
      });
  }
}

export function presentRuntimeProfile(
  profile: RuntimeProfile,
  provider?: Pick<RuntimeProviderStatus, "experience">,
): RuntimePresentation {
  const classLabel = classLabelFor(profile.runtime_class);
  const title = profile.runtime_class === "built_in" ? "Susun Runtime" : profile.display_name;
  const tooltip = profile.runtime_class === "built_in" ? "Susun Runtime" : profile.display_name;
  const externalAppNote = provider?.experience.requires_external_desktop_app
    ? "Managed by its external desktop app"
    : null;

  if (profile.ownership_state === "ownership_conflict" || profile.management.requires_recovery) {
    return presentation({
      title,
      tooltip,
      classLabel,
      stateLabel: "Recovery required",
      tone: "danger",
      actionBlockedReason: "Runtime ownership must be recovered before actions can continue.",
      externalAppNote,
    });
  }

  if (profile.availability_state === "missing") {
    return presentation({
      title,
      tooltip,
      classLabel,
      stateLabel: "Missing",
      tone: "danger",
      actionBlockedReason: "This runtime is missing.",
      externalAppNote,
    });
  }

  if (profile.availability_state === "unknown") {
    return presentation({
      title,
      tooltip,
      classLabel,
      stateLabel: "Unavailable",
      tone: "warning",
      actionBlockedReason: "This runtime must be rechecked before actions can continue.",
      externalAppNote,
    });
  }

  return presentation({
    title,
    tooltip,
    classLabel,
    stateLabel: "Ready",
    tone: "positive",
    externalAppNote,
  });
}

export function presentRuntimeAttribution(attribution: {
  runtime_profile_id: string | null;
  runtime_class: string | null;
  binding_source?: RuntimeBindingSource | null;
}): RuntimePresentation {
  const knownClass = historicalClassLabel(attribution.runtime_class);
  const title = knownClass === "Built-in" ? "Susun Runtime" : "Recorded runtime";
  const tooltip = attribution.runtime_profile_id
    ? `${title} (${attribution.runtime_profile_id})`
    : title;
  const source = attribution.binding_source?.replaceAll("_", " ") ?? "historical record";

  return presentation({
    title,
    tooltip,
    classLabel: knownClass,
    stateLabel: "Historical",
    tone: "neutral",
    externalAppNote: null,
    actionBlockedReason: null,
    supportingText: `Recorded from ${source}`,
  });
}

function presentation(
  input: Omit<RuntimePresentation, "supportingText" | "actionBlockedReason" | "externalAppNote"> &
    Partial<
      Pick<RuntimePresentation, "supportingText" | "actionBlockedReason" | "externalAppNote">
    >,
): RuntimePresentation {
  return {
    ...input,
    title: boundedLabel(input.title),
    supportingText:
      input.supportingText ?? (input.classLabel === "Built-in" ? "Powered by Podman" : null),
    actionBlockedReason: input.actionBlockedReason ?? null,
    externalAppNote: input.externalAppNote ?? null,
  };
}

function classLabelFor(
  runtimeClass: RuntimeBindingSummary["runtime_class"],
): RuntimePresentation["classLabel"] {
  switch (runtimeClass) {
    case "built_in":
      return "Built-in";
    case "external_remote":
      return "External remote";
    case "external_local":
      return "External";
    case null:
      return "Platform default";
  }
}

function historicalClassLabel(runtimeClass: string | null): RuntimePresentation["classLabel"] {
  if (runtimeClass === "built_in") return "Built-in";
  if (runtimeClass === "external_local") return "External";
  if (runtimeClass === "external_remote") return "External remote";
  return "Recorded runtime";
}

function boundedLabel(label: string): string {
  if (label.length <= MAX_RUNTIME_LABEL_LENGTH) {
    return label;
  }

  return `${label.slice(0, MAX_RUNTIME_LABEL_LENGTH - 1)}…`;
}
