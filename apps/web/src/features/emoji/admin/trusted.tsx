import { useInfiniteQuery, useMutation } from "@tanstack/react-query";
import { toDate } from "@tokuly/api";
import { emojiKeys } from "@tokuly/core/emoji";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { Avatar } from "#/components/ui/avatar";
import { Button } from "#/components/ui/button";
import { LoadMore } from "#/components/ui/load-more";
import { EmptyState, Skeleton } from "#/components/ui/page";
import { errorMessage } from "#/lib/errors";
import { emojiAdminClient, UserSearch, useAdminRefresh } from "./shared";

/** Trusted submitters: people whose emoji join the catalogue without review. */
export function TrustedSubmitters() {
  const { t, i18n } = useTranslation("emoji");
  const refresh = useAdminRefresh();
  const list = useInfiniteQuery({
    queryKey: emojiKeys.adminTrusted,
    queryFn: ({ pageParam }) =>
      emojiAdminClient().listTrustedSubmitters({ page: { limit: 30, cursor: pageParam } }),
    initialPageParam: "",
    getNextPageParam: (last) => last.page?.nextCursor || undefined,
  });
  const setTrusted = useMutation({
    mutationFn: (input: { userId: string; trusted: boolean; name: string }) =>
      emojiAdminClient().setSubmitterTrusted({ userId: input.userId, trusted: input.trusted }),
    onSuccess: async (_, input) => {
      toast.success(
        t(input.trusted ? "admin.trustedDone" : "admin.untrustedDone", { name: input.name }),
      );
      await refresh();
    },
    onError: (error) => toast.error(errorMessage(error, { namespace: "emoji" })),
  });
  const submitters = list.data?.pages.flatMap((page) => page.submitters) ?? [];
  const format = new Intl.DateTimeFormat(i18n.language, {
    year: "numeric",
    month: "short",
    day: "numeric",
  });

  return (
    <div className="max-w-2xl">
      <p className="text-sm leading-relaxed text-ink-soft">{t("admin.trustedLead")}</p>
      <div className="mt-3">
        <UserSearch
          label={t("admin.trustSomeone")}
          excludeIds={submitters.map((entry) => entry.user?.id ?? "")}
          onPick={(user) =>
            setTrusted.mutate({ userId: user.id, trusted: true, name: user.displayName })
          }
        />
      </div>
      <div className="mt-4 rounded-2xl border-2 border-line bg-surface">
        {list.isPending ? (
          <Skeleton className="m-3 h-24" />
        ) : submitters.length === 0 ? (
          <EmptyState emoji="🤝" title={t("admin.trustedEmpty")} />
        ) : (
          <ul className="divide-y-2 divide-line">
            {submitters.map((entry) => {
              const at = toDate(entry.trustedAt);
              return (
                entry.user && (
                  <li
                    key={entry.user.id}
                    data-testid="trusted-row"
                    className="flex items-center gap-3 px-3 py-2.5"
                  >
                    <Avatar user={entry.user} size="sm" />
                    <div className="min-w-0 flex-1">
                      <p className="truncate font-bold">
                        {entry.user.displayName}{" "}
                        <span className="font-normal text-ink-faint">@{entry.user.handle}</span>
                      </p>
                      <p className="truncate text-xs text-ink-soft">
                        {t("admin.trustedMeta", { count: entry.approvedCount })}
                        {at && ` · ${format.format(at)}`}
                        {entry.trustedBy && ` · @${entry.trustedBy.handle}`}
                      </p>
                    </div>
                    <Button
                      variant="secondary"
                      size="sm"
                      onClick={() =>
                        setTrusted.mutate({
                          userId: entry.user?.id ?? "",
                          trusted: false,
                          name: entry.user?.displayName ?? "",
                        })
                      }
                    >
                      {t("admin.untrust")}
                    </Button>
                  </li>
                )
              );
            })}
          </ul>
        )}
      </div>
      <LoadMore query={list} />
    </div>
  );
}
