import { type QueryClient, useQuery, useQueryClient } from "@tanstack/react-query";
import { type Api, create, identity } from "@tokuly/api";
import { Alert } from "react-native";
import { currentAccount, isActive } from "#/features/session/account";
import { api } from "#/lib/api";
import { errorMessage } from "#/lib/errors";

const accountService = (from: Api) => from.service(identity.AccountService);

export const settingsKeys = {
  settings: ["identity", "settings"] as const,
};

/** The account's privacy and appearance settings. */
export function useSettings() {
  return useQuery({
    queryKey: settingsKeys.settings,
    queryFn: async () =>
      (await accountService(api()).getSettings({})).settings ?? create(identity.SettingsSchema),
    staleTime: 5 * 60_000,
  });
}

/**
 * Saves run one after another: the server replaces a settings group as a
 * whole, so two overlapping requests could otherwise land in the wrong order
 * and undo each other.
 */
let saving: Promise<void> = Promise.resolve();
let unsaved = 0;

/**
 * Changes fields of the privacy settings. The screen shows the new value at
 * once; if the server refuses it, the screen reads what the server has and
 * says why. The save speaks as the account that made the change, even when
 * it waits behind another one past a switch (`query` is that account's cache);
 * once that account is no longer the active one, its answer is not shown.
 */
export function changePrivacy(query: QueryClient, patch: Partial<identity.PrivacySettings>): void {
  const account = currentAccount();
  const current = query.getQueryData<identity.Settings>(settingsKeys.settings);
  if (!account || !current?.privacy) return;
  const privacy = create(identity.PrivacySettingsSchema, { ...current.privacy, ...patch });
  query.setQueryData<identity.Settings>(settingsKeys.settings, { ...current, privacy });

  unsaved += 1;
  saving = saving.then(async () => {
    try {
      const response = await accountService(account.api).updateSettings({ privacy });
      unsaved -= 1;
      // While later changes are still queued the screen is ahead of this response.
      if (unsaved === 0 && response.settings && isActive(account))
        query.setQueryData(settingsKeys.settings, response.settings);
    } catch (error) {
      unsaved -= 1;
      if (!isActive(account)) return;
      Alert.alert(errorMessage(error, { namespace: "settings" }));
      await query.invalidateQueries({ queryKey: settingsKeys.settings });
    }
  });
}

/** `changePrivacy` bound to the account's query client. */
export function useChangePrivacy() {
  const query = useQueryClient();
  return (patch: Partial<identity.PrivacySettings>) => changePrivacy(query, patch);
}
