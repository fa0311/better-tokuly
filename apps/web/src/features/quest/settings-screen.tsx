import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import type { MessageInitShape, quest } from "@tokuly/api";
import { profileKeys } from "@tokuly/core/profile";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { Card } from "#/components/ui/card";
import { EmptyState, Page, Skeleton } from "#/components/ui/page";
import { SwitchRow } from "#/components/ui/switch";
import { errorMessage } from "#/lib/errors";
import { questClient, questKeys } from "./api";

type Change = MessageInitShape<typeof quest.UpdatePreferencesRequestSchema>;

/**
 * `/quest/settings`: who sees the player's progress (rankings, the profile)
 * and whether a streak that is about to end sends a reminder.
 */
export function QuestSettingsScreen() {
  const { t } = useTranslation("quest");
  const queryClient = useQueryClient();
  const { data, isPending, isError } = useQuery({
    queryKey: questKeys.preferences,
    queryFn: async () => (await questClient().getPreferences({})).preferences,
  });

  const update = useMutation({
    mutationFn: (change: Change) => questClient().updatePreferences(change),
    // The switch moves at once; the server's answer (or the old value, on failure) settles it.
    onMutate: (change) => {
      const before = queryClient.getQueryData<quest.Preferences>(questKeys.preferences);
      if (before)
        queryClient.setQueryData(questKeys.preferences, { ...before, ...definedOnly(change) });
      return before;
    },
    onSuccess: (response) => {
      queryClient.setQueryData(questKeys.preferences, response.preferences);
      // What others see of the player changed: rankings and profile cards are read again.
      void queryClient.invalidateQueries({ queryKey: questKeys.leaderboards });
      void queryClient.invalidateQueries({ queryKey: questKeys.season });
      void queryClient.invalidateQueries({ queryKey: questKeys.profiles });
      void queryClient.invalidateQueries({ queryKey: profileKeys.profiles });
    },
    onError: (error, _, before) => {
      if (before) queryClient.setQueryData(questKeys.preferences, before);
      toast.error(errorMessage(error, { namespace: "quest" }));
    },
  });

  return (
    <Page title={t("settings.title")} back="/quest">
      {isPending ? (
        <Skeleton className="h-56" />
      ) : isError || !data ? (
        <EmptyState emoji="🛰️" title={t("settings.error")} />
      ) : (
        <Card className="p-2">
          <SwitchRow
            label={t("settings.leaderboards.label")}
            hint={t("settings.leaderboards.hint")}
            checked={data.showOnLeaderboards}
            onChange={(showOnLeaderboards) => update.mutate({ showOnLeaderboards })}
          />
          <SwitchRow
            label={t("settings.profile.label")}
            hint={t("settings.profile.hint")}
            checked={data.showOnProfile}
            onChange={(showOnProfile) => update.mutate({ showOnProfile })}
          />
          <SwitchRow
            label={t("settings.reminders.label")}
            hint={t("settings.reminders.hint")}
            checked={data.streakReminders}
            onChange={(streakReminders) => update.mutate({ streakReminders })}
          />
        </Card>
      )}
    </Page>
  );
}

function definedOnly(change: Change): Partial<quest.Preferences> {
  return Object.fromEntries(Object.entries(change).filter(([, value]) => value !== undefined));
}
