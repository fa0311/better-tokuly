import { type QueryClient, useQuery, useQueryClient } from "@tanstack/react-query";
import { create, identity, social } from "@tokuly/api";
import { profileKeys } from "@tokuly/core/profile";
import { useEffect } from "react";
import { toast } from "sonner";
import { accountClient, api } from "#/lib/api";
import { errorMessage } from "#/lib/errors";

export const friendClient = () => api().service(social.FriendService);

/** Query keys of everything the settings screens read. */
export const keys = {
  settings: ["identity", "settings"] as const,
  sessions: ["identity", "sessions"] as const,
  passkeys: ["identity", "passkeys"] as const,
  securityEvents: ["identity", "security-events"] as const,
  connections: ["identity", "connections"] as const,
  blocked: ["social", "blocked"] as const,
  /** Shared with the profile screen, so an edit shows there at once. */
  profile: profileKeys.profile,
};

/** Everything cached here that belongs to the signed-in account alone (the profile is public). */
export const PRIVATE_KEYS = [
  keys.settings,
  keys.sessions,
  keys.passkeys,
  keys.securityEvents,
  keys.connections,
  keys.blocked,
];

/** The account's privacy and appearance settings. */
export function useSettings() {
  return useQuery({
    queryKey: keys.settings,
    queryFn: async () =>
      (await accountClient().getSettings({})).settings ?? create(identity.SettingsSchema),
    staleTime: 5 * 60_000,
  });
}

type Group = "privacy" | "appearance";
type GroupValue<G extends Group> = G extends "privacy"
  ? identity.PrivacySettings
  : identity.AppearanceSettings;

/**
 * Saves run one after another: the server replaces a settings group as a
 * whole, so two overlapping requests could otherwise land in the wrong order
 * and undo each other.
 */
let saving: Promise<void> = Promise.resolve();
let unsaved = 0;

/**
 * Changes fields of one settings group. The screen shows the new value at
 * once; if the server refuses it, the change is rolled back to what the server
 * has and the reason is shown.
 */
export function changeSettings<G extends Group>(
  queryClient: QueryClient,
  group: G,
  patch: Partial<GroupValue<G>>,
): void {
  const current = queryClient.getQueryData<identity.Settings>(keys.settings);
  const stored = current?.[group];
  if (!current || !stored) return;
  const next = { ...stored, ...patch };
  queryClient.setQueryData<identity.Settings>(keys.settings, { ...current, [group]: next });

  unsaved += 1;
  saving = saving.then(async () => {
    try {
      const response = await accountClient().updateSettings({ [group]: next });
      unsaved -= 1;
      // While later changes are still queued the screen is ahead of this response.
      if (unsaved === 0 && response.settings)
        queryClient.setQueryData(keys.settings, response.settings);
    } catch (error) {
      unsaved -= 1;
      toast.error(errorMessage(error));
      await queryClient.invalidateQueries({ queryKey: keys.settings });
    }
  });
}

/** `changeSettings` bound to the app's query client. */
export function useChangeSettings() {
  const queryClient = useQueryClient();
  return <G extends Group>(group: G, patch: Partial<GroupValue<G>>) =>
    changeSettings(queryClient, group, patch);
}

/** Puts a fresh copy of the account into the session, so every screen shows it. */
export function adoptMe(me: identity.Me | undefined): void {
  if (me) api().session.updateMe(me);
}

/** Re-reads the account (verification state, passkey count, ...) from the server. */
export async function refreshMe(): Promise<void> {
  adoptMe((await accountClient().getMe({})).me);
}

/**
 * Keeps the account fresh while a screen that shows its state is open: read
 * again when the screen opens and whenever the tab comes back to the front
 * (the email may have been verified in another tab meanwhile).
 */
export function useFreshMe(): void {
  useEffect(() => {
    const refresh = () => {
      if (document.visibilityState === "visible") void refreshMe().catch(() => {});
    };
    refresh();
    document.addEventListener("visibilitychange", refresh);
    return () => document.removeEventListener("visibilitychange", refresh);
  }, []);
}
