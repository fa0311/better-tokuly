import { useInfiniteQuery, useMutation, useQueryClient } from "@tanstack/react-query";
import { useState } from "react";
import { toast } from "sonner";
import { Button } from "#/components/ui/button";
import { Card } from "#/components/ui/card";
import { ConfirmDialog } from "#/components/ui/confirm";
import { LoadMore } from "#/components/ui/load-more";
import { EmptyState, Page, Skeleton } from "#/components/ui/page";
import { EmojiText } from "#/features/emoji";
import { errorMessage } from "#/lib/errors";
import { useTranslation } from "#/lib/use-translation";
import { authorizationClient, type dev, keys, nextCursorOf, PAGE_SIZE } from "./api";
import { SquareImage } from "./image-field";
import { Ago, Notice, When } from "./parts";
import { ScopeMeanings } from "./scope-picker";

/** `/developers/authorized`: the applications that have access to the caller's account. */
export function AuthorizedAppsScreen() {
  const { t } = useTranslation("dev");
  const queryClient = useQueryClient();
  const [revoking, setRevoking] = useState<dev.AuthorizedApp>();

  const apps = useInfiniteQuery({
    queryKey: keys.authorized,
    queryFn: ({ pageParam }) =>
      authorizationClient().listAuthorizedApps({ page: { cursor: pageParam, limit: PAGE_SIZE } }),
    initialPageParam: "",
    getNextPageParam: nextCursorOf,
  });

  const revoke = useMutation({
    mutationFn: (appId: string) => authorizationClient().revokeAuthorizedApp({ appId }),
    onSuccess: async () => {
      setRevoking(undefined);
      toast.success(t("authorized.revoked"));
      await queryClient.invalidateQueries({ queryKey: keys.authorized });
    },
    onError: (error) => toast.error(errorMessage(error, { namespace: "dev" })),
  });

  const rows = apps.data?.pages.flatMap((page) => page.apps) ?? [];
  return (
    <Page title={t("authorized.title")} back="/developers">
      <p className="mb-4 px-1 text-sm leading-relaxed text-ink-soft">{t("authorized.lead")}</p>
      {apps.isPending ? (
        <Skeleton className="h-48" />
      ) : apps.isError ? (
        <Notice tone="danger">{errorMessage(apps.error, { namespace: "dev" })}</Notice>
      ) : rows.length === 0 ? (
        <EmptyState emoji="🔐" title={t("authorized.empty")} body={t("authorized.emptyBody")} />
      ) : (
        <ul className="flex flex-col gap-3">
          {rows.map((row) => (
            <li key={row.app?.id}>
              <Card className="p-4">
                <div className="flex items-center gap-3">
                  <SquareImage image={row.app?.icon} name={row.app?.name ?? ""} />
                  <div className="min-w-0 flex-1">
                    <p className="font-black break-words">
                      <EmojiText text={row.app?.name ?? ""} />
                    </p>
                    <p className="truncate text-sm text-ink-soft">
                      {t("oauth.madeBy")} @{row.app?.owner?.handle}
                    </p>
                  </div>
                </div>
                <p className="mt-3 text-xs font-bold text-ink-soft">
                  {t("authorized.since")} <When at={row.authorizedAt} />
                  {row.lastUsedAt && (
                    <>
                      {" · "}
                      {t("authorized.lastUsed")} <Ago at={row.lastUsedAt} />
                    </>
                  )}
                </p>
                <details className="mt-2">
                  <summary className="cursor-pointer text-sm font-bold text-brand">
                    {t("authorized.what", { count: row.scopes.length })}
                  </summary>
                  <div className="mt-2">
                    <ScopeMeanings scopes={row.scopes} />
                  </div>
                </details>
                <div className="mt-3 flex justify-end">
                  <Button variant="danger" size="sm" onClick={() => setRevoking(row)}>
                    {t("authorized.revoke")}
                  </Button>
                </div>
              </Card>
            </li>
          ))}
          <LoadMore query={apps} />
        </ul>
      )}
      <ConfirmDialog
        open={revoking !== undefined}
        onOpenChange={(open) => !open && setRevoking(undefined)}
        title={t("authorized.revokeTitle", { name: revoking?.app?.name })}
        description={t("authorized.revokeBody")}
        confirmLabel={t("authorized.revoke")}
        danger
        loading={revoke.isPending}
        onConfirm={() => revoking?.app && revoke.mutate(revoking.app.id)}
      />
    </Page>
  );
}
