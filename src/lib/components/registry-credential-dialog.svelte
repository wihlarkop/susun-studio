<script lang="ts">
  import * as Dialog from "$lib/components/ui/dialog/index.js";
  import { Button } from "$lib/components/ui/button/index.js";
  import { Input } from "$lib/components/ui/input/index.js";
  import {
    createRegistryCredential,
    deleteRegistryCredential,
    rotateRegistryCredential,
    type RegistryCredential,
  } from "$lib/daemon/client";
  import {
    applyCredentialFailure,
    applyCredentialSuccess,
    beginCredentialSubmission,
    initialCredentialOperation,
    resetCredentialOperation,
    toCredentialOperationError,
  } from "$lib/registry/credential-state";
  import { KeyRound, LogOut } from "@lucide/svelte";

  type DialogMode = "create" | "rotate" | "delete";

  let {
    mode,
    credential = null,
    engineId,
    open = $bindable(false),
    oncompleted,
  }: {
    mode: DialogMode;
    credential?: RegistryCredential | null;
    engineId: string;
    open?: boolean;
    oncompleted?: () => void | Promise<void>;
  } = $props();

  let operation = $state(initialCredentialOperation());
  let generation = 0;
  let controller: AbortController | null = null;

  $effect(() => {
    const isOpen = open;
    const credentialId = credential?.id ?? null;
    const engine = engineId;
    controller?.abort();
    controller = null;
    generation += 1;
    operation = resetCredentialOperation(generation);
    void isOpen;
    void credentialId;
    void engine;
    return () => controller?.abort();
  });

  const title = $derived(
    mode === "create"
      ? "Sign in to a registry"
      : mode === "rotate"
        ? `Update ${credential?.registry ?? "credential"}`
        : `Sign out of ${credential?.registry ?? "registry"}`,
  );
  const isSubmitting = $derived(operation.phase === "submitting");

  async function submitCredential(event: SubmitEvent) {
    event.preventDefault();
    const form = event.currentTarget as HTMLFormElement;
    const data = new FormData(form);
    const registry = String(data.get("registry") ?? "").trim();
    const usernameValue = String(data.get("username") ?? "").trim();
    const secret = String(data.get("secret") ?? "");
    if (!secret || (mode === "create" && !registry)) return;

    const started = beginCredentialSubmission(operation);
    operation = started.state;
    const requestGeneration = operation.generation;
    if (started.clearSensitiveInputs) form.reset();
    controller?.abort();
    controller = new AbortController();

    try {
      if (mode === "create") {
        await createRegistryCredential(registry, usernameValue || null, secret, {
          signal: controller.signal,
        });
      } else if (credential) {
        await rotateRegistryCredential(credential.id, usernameValue || null, secret, {
          signal: controller.signal,
        });
      }
      operation = applyCredentialSuccess(operation, requestGeneration);
      await oncompleted?.();
    } catch (caught) {
      if (controller.signal.aborted) return;
      operation = applyCredentialFailure(
        operation,
        requestGeneration,
        toCredentialOperationError(caught),
      );
    }
  }

  async function signOut() {
    if (!credential) return;
    const started = beginCredentialSubmission(operation);
    operation = started.state;
    const requestGeneration = operation.generation;
    controller?.abort();
    controller = new AbortController();
    try {
      await deleteRegistryCredential(credential.id, { signal: controller.signal });
      operation = applyCredentialSuccess(operation, requestGeneration);
      await oncompleted?.();
      open = false;
    } catch (caught) {
      if (controller.signal.aborted) return;
      operation = applyCredentialFailure(
        operation,
        requestGeneration,
        toCredentialOperationError(caught),
      );
    }
  }
</script>

<Dialog.Root bind:open>
  <Dialog.Content class="sm:max-w-md">
    <Dialog.Header>
      <Dialog.Title>{title}</Dialog.Title>
      <Dialog.Description>
        {#if mode === "create"}
          The secret is stored in this device's native credential manager. Studio stores only
          display-safe metadata in its database.
        {:else if mode === "rotate"}
          Replace the native credential without changing the registry identity.
        {:else}
          Remove the Studio-owned credential from this device. External runtime credentials are
          not affected.
        {/if}
      </Dialog.Description>
    </Dialog.Header>

    {#if mode === "delete"}
      <div class="rounded-md border bg-muted/30 p-3 text-sm">
        <div class="font-medium">{credential?.registry}</div>
        <div class="mt-1 text-muted-foreground">
          Future authenticated transfers through Studio will require signing in again.
        </div>
      </div>
    {:else}
      <form class="grid gap-4" onsubmit={submitCredential}>
        {#if mode === "create"}
          <label class="grid gap-1.5 text-sm font-medium">
            Registry
            <Input name="registry" autocomplete="url" placeholder="registry.example.com" required />
          </label>
        {:else}
          <div class="grid gap-1 text-sm">
            <span class="font-medium">Registry</span>
            <span class="font-mono text-muted-foreground">{credential?.registry}</span>
          </div>
        {/if}

        <label class="grid gap-1.5 text-sm font-medium">
          Username
          <Input
            name="username"
            autocomplete="username"
            value={mode === "rotate" ? (credential?.username_label ?? "") : ""}
            placeholder="Optional for access tokens"
          />
        </label>

        <label class="grid gap-1.5 text-sm font-medium">
          Password or access token
          <Input name="secret" type="password" autocomplete="current-password" required />
        </label>

        {#if operation.error}
          <p class="text-sm text-destructive">{operation.error.message}</p>
        {:else if operation.phase === "succeeded"}
          <p class="text-sm text-muted-foreground">Credential saved in the native store.</p>
        {/if}

        <Dialog.Footer>
          <Button type="button" variant="outline" onclick={() => (open = false)}>Close</Button>
          <Button type="submit" disabled={isSubmitting}>
            <KeyRound />
            {isSubmitting ? "Saving" : mode === "create" ? "Sign in" : "Update credential"}
          </Button>
        </Dialog.Footer>
      </form>
    {/if}

    {#if mode === "delete"}
      {#if operation.error}<p class="text-sm text-destructive">{operation.error.message}</p>{/if}
      <Dialog.Footer>
        <Button type="button" variant="outline" onclick={() => (open = false)}>Cancel</Button>
        <Button type="button" variant="destructive" disabled={isSubmitting} onclick={signOut}>
          <LogOut />
          {isSubmitting ? "Signing out" : "Sign out"}
        </Button>
      </Dialog.Footer>
    {/if}
  </Dialog.Content>
</Dialog.Root>
