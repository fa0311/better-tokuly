import { useMutation, useQueryClient } from "@tanstack/react-query";
import type { community } from "@tokuly/api";
import { communityKeys } from "@tokuly/core/community";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { Avatar } from "#/components/ui/avatar";
import { Button } from "#/components/ui/button";
import { LoadMore } from "#/components/ui/load-more";
import { EmptyState, Skeleton } from "#/components/ui/page";
import { EmojiText } from "#/features/emoji";
import { errorMessage } from "#/lib/errors";
import { moderationClient, NS, useBans } from "../api";
import { Notice, useWhen } from "../parts";

/** Bans in force, newest first; lifting one lets the person join again. */
export function BansSection({ community: c }: { community: community.Community }) {
  const { t } = useTranslation(NS);
  const when = useWhen();
  const queryClient = useQueryClient();
  const bans = useBans(c.id);
  const unban = useMutation({
    mutationFn: (userId: string) => moderationClient().unbanMember({ communityId: c.id, userId }),
    onSuccess: () => {
      void queryClient.invalidateQueries({ queryKey: communityKeys.bans(c.id) });
      toast(t("manage.bans.lifted"));
    },
    onError: (error) => toast.error(errorMessage(error, { namespace: NS })),
  });
  if (bans.isPending) return <Skeleton className="h-24" />;
  return (
    <div className="flex flex-col gap-3">
      <Notice>{t("manage.bans.hint")}</Notice>
      {bans.items?.length === 0 ? (
        <EmptyState emoji="🕊️" title={t("manage.bans.empty")} />
      ) : (
        <>
          {bans.items?.map((ban) => (
            <article key={ban.user?.id} className="rounded-card bg-surface p-4">
              <div className="flex items-center gap-3">
                <Avatar user={ban.user} />
                <div className="min-w-0 flex-1">
                  <p className="truncate font-bold">
                    <EmojiText text={ban.user?.displayName || ban.user?.handle || ""} />
                  </p>
                  <p className="truncate text-sm text-ink-soft">@{ban.user?.handle}</p>
                </div>
                <Button
                  variant="secondary"
                  size="sm"
                  disabled={unban.isPending}
                  onClick={() => ban.user && unban.mutate(ban.user.id)}
                >
                  {t("manage.bans.lift")}
                </Button>
              </div>
              <p className="mt-2 break-words text-sm">{ban.reason}</p>
              <p className="mt-1 text-xs text-ink-faint">
                {ban.expiresAt
                  ? t("ban.until", { time: when(Number(ban.expiresAt.seconds) * 1000) })
                  : t("ban.forever")}
                {ban.bannedBy && ` · ${t("manage.bans.by", { name: ban.bannedBy.displayName })}`}
              </p>
            </article>
          ))}
          <LoadMore query={bans} />
        </>
      )}
    </div>
  );
}
