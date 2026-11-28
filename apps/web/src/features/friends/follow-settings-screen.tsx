import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { Link } from "@tanstack/react-router";
import { ChevronRight, Lock } from "lucide-react";
import { type ReactNode, useState } from "react";
import { toast } from "sonner";
import { ConfirmDialog } from "#/components/ui/confirm";
import { Page, Skeleton } from "#/components/ui/page";
import { SwitchRow } from "#/components/ui/switch";
import { errorMessage } from "#/lib/errors";
import { useTranslation } from "#/lib/use-translation";
import { followClient, followKeys, refreshFollows } from "./follow-data";

/** `/settings/follows`: the 鍵アカウント switch and the way to waiting requests. */
export function FollowSettingsScreen() {
  const { t } = useTranslation("follows");
  const queryClient = useQueryClient();
  const settings = useQuery({
    queryKey: followKeys.settings,
    queryFn: () => followClient().getFollowSettings({}),
  });
  const [confirmUnlock, setConfirmUnlock] = useState(false);
  const locked = settings.data?.settings?.locked ?? false;
  const pending = settings.data?.pendingRequestCount ?? 0;

  const update = useMutation({
    mutationFn: (lock: boolean) =>
      followClient().updateFollowSettings({ settings: { locked: lock } }),
    onSuccess: (response) => {
      setConfirmUnlock(false);
      queryClient.setQueryData(followKeys.settings, {
        ...settings.data,
        settings: response.settings,
        pendingRequestCount: response.settings?.locked ? pending : 0,
      });
      toast.success(
        response.settings?.locked
          ? t("settings.lockedToast")
          : response.approvedRequestCount > 0
            ? t("settings.approvedToast", { count: response.approvedRequestCount })
            : t("settings.unlockedToast"),
      );
      return refreshFollows(queryClient);
    },
    onError: (error) => {
      setConfirmUnlock(false);
      toast.error(errorMessage(error, { namespace: "follows" }));
    },
  });

  return (
    <Page title={t("settings.title")} back="/settings">
      <Group title={t("settings.lockGroup")} lead={t("settings.lockLead")}>
        {settings.isPending ? (
          <Skeleton className="h-16" />
        ) : (
          <SwitchRow
            label={
              <span className="inline-flex items-center gap-2">
                <Lock className="size-4" aria-hidden />
                {t("settings.lock")}
              </span>
            }
            hint={locked ? t("settings.lockOnHint") : t("settings.lockOffHint")}
            checked={locked}
            disabled={update.isPending}
            onChange={(next) =>
              !next && pending > 0 ? setConfirmUnlock(true) : update.mutate(next)
            }
          />
        )}
      </Group>
      <Group title={t("requests.title")}>
        <Link
          to="/friends/follow-requests"
          className="flex items-center gap-3 rounded-2xl px-3 py-3 hover:bg-surface-2"
        >
          <span className="min-w-0 flex-1 font-bold">{t("settings.requestsRow")}</span>
          {pending > 0 && (
            <span className="rounded-full bg-pink px-2 text-xs font-black text-white">
              {pending}
            </span>
          )}
          <ChevronRight className="size-5 text-ink-faint" aria-hidden />
        </Link>
      </Group>
      <ConfirmDialog
        open={confirmUnlock}
        onOpenChange={setConfirmUnlock}
        title={t("settings.unlockTitle")}
        description={t("settings.unlockBody", { count: pending })}
        confirmLabel={t("settings.unlockConfirm")}
        loading={update.isPending}
        onConfirm={() => update.mutate(false)}
      />
    </Page>
  );
}

/** A titled card of settings, like the settings screens' own groups. */
function Group({
  title,
  lead,
  children,
}: {
  title: ReactNode;
  lead?: ReactNode;
  children: ReactNode;
}) {
  return (
    <section className="mt-7 first:mt-0">
      <h2 className="px-1 text-lg font-black">{title}</h2>
      {lead && <p className="mt-0.5 px-1 text-sm leading-relaxed text-ink-soft">{lead}</p>}
      <div className="sticker mt-3 rounded-card bg-surface p-2">{children}</div>
    </section>
  );
}
