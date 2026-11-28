import { useInfiniteQuery } from "@tanstack/react-query";
import type { admin } from "@tokuly/api";
import { LoadMore } from "#/components/ui/load-more";
import { EmptyState, Page, Skeleton } from "#/components/ui/page";
import { Tabs } from "#/components/ui/tabs";
import { cn } from "#/lib/cn";
import { errorMessage } from "#/lib/errors";
import { useTranslation } from "#/lib/use-translation";
import { announcementClient, categoryKey } from "./api";
import { ANNOUNCEMENT_CATEGORIES, type AnnouncementCategory } from "./categories";
import { AnnouncementCard } from "./parts";

/** `/announcements`: the product update log, pinned announcements first. */
export function AnnouncementsScreen({
  category,
  onCategoryChange,
}: {
  category?: AnnouncementCategory;
  onCategoryChange(category: AnnouncementCategory | undefined): void;
}) {
  const { t, i18n } = useTranslation("announcements");
  const list = useInfiniteQuery({
    queryKey: ["admin", "announcements", "published", i18n.language, category ?? 0],
    queryFn: ({ pageParam }) =>
      announcementClient().listAnnouncements({
        page: { cursor: pageParam, limit: 20 },
        locale: i18n.language,
        category,
      }),
    initialPageParam: "",
    getNextPageParam: (last) => last.page?.nextCursor || undefined,
  });
  const announcements = list.data?.pages.flatMap((page) => page.announcements) ?? [];

  // Two columns on wide screens: pinned announcements and the newest one span the row, the rest
  // pair up. Once the whole list is there, a card left without a partner spans the row too.
  const leads = announcements.filter(
    (announcement, index) => announcement.pinned || index === 0,
  ).length;
  const spansRow = (index: number) => {
    const announcement = announcements[index];
    if (announcement?.pinned || index === 0) return true;
    return (
      !list.hasNextPage &&
      index === announcements.length - 1 &&
      (announcements.length - leads) % 2 === 1
    );
  };

  return (
    <Page title={t("title")} wide>
      <div className="mx-auto max-w-4xl">
        <p className="mb-4 text-sm font-bold text-ink-soft">{t("lead")}</p>
        <Tabs<string>
          className="mb-5"
          value={String(category ?? "")}
          onChange={(value) =>
            onCategoryChange(value ? (Number(value) as AnnouncementCategory) : undefined)
          }
          items={[
            { value: "", label: t("all") },
            ...ANNOUNCEMENT_CATEGORIES.map((option) => ({
              value: String(option),
              label: t(categoryKey(option)),
            })),
          ]}
        />
        {list.isPending ? (
          <div className="grid gap-4 md:grid-cols-2" aria-busy>
            <Skeleton className="h-44 md:col-span-2" />
            <Skeleton className="h-44" />
            <Skeleton className="h-44" />
          </div>
        ) : list.isError ? (
          <EmptyState emoji="🌀" title={errorMessage(list.error)} />
        ) : announcements.length === 0 ? (
          <EmptyState
            emoji="📭"
            title={t(category ? "emptyCategory" : "empty")}
            body={category ? undefined : t("emptyBody")}
          />
        ) : (
          <>
            <ul className="grid gap-4 md:grid-cols-2">
              {announcements.map((announcement: admin.Announcement, index) => (
                <li key={announcement.id} className={cn(spansRow(index) && "md:col-span-2")}>
                  <AnnouncementCard announcement={announcement} linkTo={announcement.id} />
                </li>
              ))}
            </ul>
            <LoadMore query={list} showEnd={announcements.length > 6} />
          </>
        )}
      </div>
    </Page>
  );
}
