import { useInfiniteQuery } from "@tanstack/react-query";
import { Link } from "@tanstack/react-router";
import { ChevronRight } from "lucide-react";
import { useTranslation } from "react-i18next";
import { Avatar } from "#/components/ui/avatar";
import { LoadMore } from "#/components/ui/load-more";
import { EmptyState, Page, Skeleton } from "#/components/ui/page";
import { stampKeys, storeClient } from "./api";
import { PackCard, PackGridSkeleton } from "./parts";

/** Every published pack of one creator. */
export function CreatorScreen({ userId }: { userId: string }) {
  const { t } = useTranslation("stamps");
  const packs = useInfiniteQuery({
    queryKey: stampKeys.creator(userId),
    queryFn: ({ pageParam }) =>
      storeClient().listCreatorPacks({ userId, page: { limit: 24, cursor: pageParam } }),
    initialPageParam: "",
    getNextPageParam: (last) => last.page?.nextCursor || undefined,
  });
  const creator = packs.data?.pages[0]?.creator;
  const items = packs.data?.pages.flatMap((page) => page.packs) ?? [];

  return (
    <Page title={t("creator.title")} back="/stamps" wide>
      {packs.isPending ? (
        <>
          <Skeleton className="mb-5 h-24" />
          <PackGridSkeleton />
        </>
      ) : packs.isError || !creator ? (
        <EmptyState emoji="🧭" title={t("creator.notFound")} />
      ) : (
        <>
          <Link
            to="/u/$handle"
            params={{ handle: creator.handle }}
            className="pressable sticker mb-5 flex items-center gap-4 rounded-card bg-surface p-4"
          >
            <Avatar user={creator} size="lg" outlined />
            <span className="min-w-0 flex-1">
              <span className="block truncate text-xl font-black">{creator.displayName}</span>
              <span className="block truncate font-bold text-ink-soft">@{creator.handle}</span>
              <span className="mt-0.5 block text-sm font-bold text-brand">
                {t("creator.profile")}
              </span>
            </span>
            <ChevronRight className="size-6 shrink-0 text-ink-faint" />
          </Link>
          {items.length === 0 ? (
            <EmptyState emoji="🎨" title={t("creator.empty")} />
          ) : (
            <>
              <div className="grid grid-cols-2 gap-3 sm:grid-cols-3 md:grid-cols-4 lg:grid-cols-6">
                {items.map((pack) => (
                  <PackCard key={pack.id} pack={pack} />
                ))}
              </div>
              <LoadMore query={packs} />
            </>
          )}
        </>
      )}
    </Page>
  );
}
