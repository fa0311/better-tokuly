import { useMutation, useQueryClient } from "@tanstack/react-query";
import { create, notify } from "@tokuly/api";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { errorMessage } from "#/lib/errors";
import { notifyKeys, preferencesClient } from "./api";

const MUTATION_KEY = ["notify", "preferences"] as const;
/**
 * Changes are sent one after another, in the order they were made: two saves
 * racing each other could otherwise land in the wrong order and the server
 * would keep the older one.
 */
const IN_ORDER = { id: "notify.preferences" } as const;

export interface QuietHoursValue {
  enabled: boolean;
  startMinute: number;
  endMinute: number;
}

/**
 * Changes to the notification preferences. A switch moves the moment it is
 * tapped; the server's answer (the whole preferences) then replaces the guess.
 */
export function usePreferenceActions() {
  const { t } = useTranslation("notifications");
  const queryClient = useQueryClient();

  const guess = (edit: (preferences: notify.Preferences) => notify.Preferences) =>
    queryClient.setQueryData<notify.Preferences>(notifyKeys.preferences, (preferences) => preferences && edit(preferences));
  const settle = (preferences: notify.Preferences | undefined) => {
    // While other changes are still on their way, an answer is already out of date.
    if (preferences && queryClient.isMutating({ mutationKey: MUTATION_KEY }) <= 1) queryClient.setQueryData(notifyKeys.preferences, preferences);
  };
  const onError = (error: unknown) => {
    toast.error(errorMessage(error));
    void queryClient.invalidateQueries({ queryKey: notifyKeys.preferences });
  };

  const setCategory = useMutation({
    mutationKey: MUTATION_KEY,
    scope: IN_ORDER,
    mutationFn: (change: { category: string; inbox?: boolean; push?: boolean }) => preferencesClient().updatePreferences({ categories: [change] }),
    onMutate: (change) =>
      guess((preferences) => ({
        ...preferences,
        categories: preferences.categories.map((category) =>
          category.category === change.category ? { ...category, inbox: change.inbox ?? category.inbox, push: change.push ?? category.push } : category,
        ),
      })),
    onSuccess: (response) => settle(response.preferences),
    onError,
  });

  const setQuietHours = useMutation({
    mutationKey: MUTATION_KEY,
    scope: IN_ORDER,
    mutationFn: (quietHours: QuietHoursValue) => preferencesClient().updatePreferences({ quietHours }),
    onMutate: (quietHours) => guess((preferences) => ({ ...preferences, quietHours: create(notify.QuietHoursSchema, quietHours) })),
    onSuccess: (response) => settle(response.preferences),
    onError,
  });

  const pause = useMutation({
    mutationKey: MUTATION_KEY,
    scope: IN_ORDER,
    /** 0 = until resumed. */
    mutationFn: (durationSeconds: number) => preferencesClient().pausePush({ durationSeconds }),
    onSuccess: (response) => {
      settle(response.preferences);
      toast.success(t("pause.toast.paused"));
    },
    onError,
  });

  const resume = useMutation({
    mutationKey: MUTATION_KEY,
    scope: IN_ORDER,
    mutationFn: () => preferencesClient().resumePush({}),
    onSuccess: (response) => {
      settle(response.preferences);
      toast.success(t("pause.toast.resumed"));
    },
    onError,
  });

  return { setCategory, setQuietHours, pause, resume };
}
