import { type QueryClient, useQuery, useQueryClient } from "@tanstack/react-query";
import { type Api, create, notify } from "@tokuly/api";
import { notifyKeys } from "@tokuly/core/notifications";
import { Alert } from "react-native";
import { currentAccount } from "#/features/session/account";
import { api } from "#/lib/api";
import { errorMessage } from "#/lib/errors";

const preferencesService = (from: Api) => from.service(notify.PreferencesService);

/** The account's notification preferences: categories, quiet hours, the pause, what pushes show. */
export function usePreferences() {
  return useQuery({
    queryKey: notifyKeys.preferences,
    queryFn: async () =>
      (await preferencesService(api()).getPreferences({})).preferences ??
      create(notify.PreferencesSchema),
  });
}

/**
 * Changes are sent one after another, in the order they were made: two saves
 * racing each other could otherwise land in the wrong order and the server
 * would keep the older one.
 */
let saving: Promise<void> = Promise.resolve();
let unsaved = 0;

/**
 * One change: shown at once (`guess`), sent as the account that made it, and
 * the server's answer (the whole preferences) then replaces the guess — unless
 * later changes are still on their way. A refusal reads the preferences again
 * and says why.
 */
function change(
  query: QueryClient,
  guess: ((preferences: notify.Preferences) => notify.Preferences) | null,
  send: (
    service: ReturnType<typeof preferencesService>,
  ) => Promise<{ preferences?: notify.Preferences }>,
): Promise<boolean> {
  const account = currentAccount();
  if (!account) return Promise.resolve(false);
  if (guess)
    query.setQueryData<notify.Preferences>(
      notifyKeys.preferences,
      (preferences) => preferences && guess(preferences),
    );
  unsaved += 1;
  const done = saving.then(async () => {
    try {
      const { preferences } = await send(preferencesService(account.api));
      unsaved -= 1;
      if (unsaved === 0 && preferences) query.setQueryData(notifyKeys.preferences, preferences);
      return true;
    } catch (error) {
      unsaved -= 1;
      Alert.alert(errorMessage(error, { namespace: "notifications" }));
      await query.invalidateQueries({ queryKey: notifyKeys.preferences });
      return false;
    }
  });
  saving = done.then(() => undefined);
  return done;
}

export interface QuietHoursValue {
  enabled: boolean;
  startMinute: number;
  endMinute: number;
}

/** The quiet hours of `preferences`; never set, they are off, from 22:00 to 7:00. */
export function quietHoursOf(preferences: notify.Preferences | undefined): QuietHoursValue {
  return {
    enabled: preferences?.quietHours?.enabled ?? false,
    startMinute: preferences?.quietHours?.startMinute ?? 22 * 60,
    endMinute: preferences?.quietHours?.endMinute ?? 7 * 60,
  };
}

/** The changes to the preferences, bound to the account's query client. */
export function usePreferenceActions() {
  const query = useQueryClient();
  return {
    setCategory: (category: string, edit: { inbox?: boolean; push?: boolean }) =>
      change(
        query,
        (preferences) => ({
          ...preferences,
          categories: preferences.categories.map((entry) =>
            entry.category === category
              ? { ...entry, inbox: edit.inbox ?? entry.inbox, push: edit.push ?? entry.push }
              : entry,
          ),
        }),
        (service) => service.updatePreferences({ categories: [{ category, ...edit }] }),
      ),
    /** Builds on the quiet hours as the latest change left them, sent or still on its way. */
    setQuietHours: (edit: Partial<QuietHoursValue>) => {
      const quietHours = {
        ...quietHoursOf(query.getQueryData<notify.Preferences>(notifyKeys.preferences)),
        ...edit,
      };
      return change(
        query,
        (preferences) => ({
          ...preferences,
          quietHours: create(notify.QuietHoursSchema, quietHours),
        }),
        (service) => service.updatePreferences({ quietHours }),
      );
    },
    setPushPreview: (pushPreview: notify.PushPreview) =>
      change(
        query,
        (preferences) => ({ ...preferences, pushPreview }),
        (service) => service.updatePreferences({ pushPreview }),
      ),
    /** 0 = until resumed. */
    pause: (durationSeconds: number) =>
      change(query, null, (service) => service.pausePush({ durationSeconds })),
    resume: () => change(query, null, (service) => service.resumePush({})),
  };
}
