import { type InfiniteData, useInfiniteQuery, useMutation, useQueryClient } from "@tanstack/react-query";
import type { common, social } from "@tokuly/api";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { Avatar } from "#/components/ui/avatar";
import { Button } from "#/components/ui/button";
import { ConfirmDialog } from "#/components/ui/confirm";
import { LoadMore } from "#/components/ui/load-more";
import { EmptyState, ListRow, Skeleton } from "#/components/ui/page";
import { errorMessage } from "#/lib/errors";
import { friendClient, keys } from "../data";
import { SettingsPage } from "../shell/settings-page";

/** `/settings/privacy/blocked`: the people you blocked, most recent first, each with "unblock". */
export function BlockedUsersScreen() {
  const { t } = useTranslation("settings");
  const queryClient = useQueryClient();
  const query = useInfiniteQuery({
    queryKey: keys.blocked,
    queryFn: ({ pageParam }) => friendClient().listBlockedUsers({ page: { limit: 30, cursor: pageParam } }),
    initialPageParam: "",
    getNextPageParam: (last) => last.page?.nextCursor || undefined,
  });
  const [confirming, setConfirming] = useState<{ user: common.UserSummary | null; open: boolean }>({ user: null, open: false });
  const close = () => setConfirming((current) => ({ ...current, open: false }));

  const unblock = useMutation({
    mutationFn: (user: common.UserSummary) => friendClient().unblockUser({ userId: user.id }),
    onSuccess: (_, user) => {
      // Drop the row at once; the profile and friend screens re-read their side.
      queryClient.setQueryData<InfiniteData<social.ListBlockedUsersResponse>>(keys.blocked, (data) =>
        data ? { ...data, pages: data.pages.map((page) => ({ ...page, users: page.users.filter((blocked) => blocked.id !== user.id) })) } : data,
      );
      toast.success(t("privacy.blocked.unblocked", { name: user.displayName }));
      close();
      void queryClient.invalidateQueries({ queryKey: ["social"] });
      void queryClient.invalidateQueries({ queryKey: ["identity", "profile"] });
    },
    onError: (error) => {
      close();
      toast.error(errorMessage(error));
    },
  });

  const users = query.data?.pages.flatMap((page) => page.users) ?? [];

  return (
    <SettingsPage title={t("privacy.blocked.title")} back="/settings/privacy">
      <p className="px-1 pb-3 text-sm leading-relaxed text-ink-soft">{t("privacy.blocked.lead")}</p>
      {query.isPending ? (
        <div className="flex flex-col gap-2">
          {[0, 1, 2].map((i) => (
            <Skeleton key={i} className="h-16" />
          ))}
        </div>
      ) : users.length === 0 ? (
        <EmptyState emoji="🕊️" title={t("privacy.blocked.empty")} body={t("privacy.blocked.emptyBody")} />
      ) : (
        <>
          <ul className="sticker rounded-card bg-surface p-2">
            {users.map((user) => (
              <li key={user.id} data-testid="blocked-user">
                <ListRow
                  leading={<Avatar user={user} outlined />}
                  title={user.displayName}
                  subtitle={`@${user.handle}`}
                  trailing={
                    <Button size="sm" variant="secondary" className="shrink-0" onClick={() => setConfirming({ user, open: true })}>
                      {t("privacy.blocked.unblock")}
                    </Button>
                  }
                />
              </li>
            ))}
          </ul>
          <LoadMore query={query} />
        </>
      )}
      <ConfirmDialog
        open={confirming.open}
        onOpenChange={(open) => !open && close()}
        title={t("privacy.blocked.unblockTitle", { name: confirming.user?.displayName ?? "" })}
        description={t("privacy.blocked.unblockBody")}
        confirmLabel={t("privacy.blocked.unblock")}
        loading={unblock.isPending}
        onConfirm={() => confirming.user && unblock.mutate(confirming.user)}
      />
    </SettingsPage>
  );
}
