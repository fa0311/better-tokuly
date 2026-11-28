import { Button } from "#/components/ui/button";
import { EmptyState, Page, Skeleton } from "#/components/ui/page";
import { errorMessage } from "#/lib/errors";
import { useTranslation } from "#/lib/use-translation";
import { usePreferences } from "./api";
import { CategoriesCard } from "./settings/categories-card";
import { MutedUsersCard } from "./settings/muted-users-card";
import { PauseCard } from "./settings/pause-card";
import { PreviewCard } from "./settings/preview-card";
import { PushCard } from "./settings/push-card";
import { QuietHoursCard } from "./settings/quiet-hours-card";

/**
 * Notification settings: push on this device and the account's devices, what
 * pushes show, the pause, quiet hours, what to be told about, and who never to hear from.
 */
export function NotificationSettingsScreen() {
  const { t } = useTranslation("notifications");
  const preferences = usePreferences();

  return (
    <Page wide title={t("settings.title")} back="/notifications">
      <div className="grid gap-5 lg:grid-cols-2 lg:items-start">
        <div className="flex min-w-0 flex-col gap-5">
          <PushCard />
          {preferences.data ? (
            <>
              <PreviewCard preferences={preferences.data} />
              <PauseCard preferences={preferences.data} />
              <QuietHoursCard preferences={preferences.data} />
            </>
          ) : (
            preferences.isPending && (
              <>
                <Skeleton className="h-36 rounded-card" />
                <Skeleton className="h-56 rounded-card" />
              </>
            )
          )}
        </div>
        <div className="flex min-w-0 flex-col gap-5">
          {preferences.data ? (
            <CategoriesCard preferences={preferences.data} />
          ) : preferences.isPending ? (
            <Skeleton className="h-[40rem] rounded-card" />
          ) : (
            <EmptyState
              emoji="😵‍💫"
              title={errorMessage(preferences.error)}
              action={
                <Button variant="secondary" onClick={() => preferences.refetch()}>
                  {t("actions.retry")}
                </Button>
              }
            />
          )}
          <MutedUsersCard />
        </div>
      </div>
    </Page>
  );
}
