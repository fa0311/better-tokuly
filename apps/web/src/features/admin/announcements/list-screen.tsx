import { useInfiniteQuery } from "@tanstack/react-query";
import { Link, useNavigate } from "@tanstack/react-router";
import { admin } from "@tokuly/api";
import { Bell, Pin, Plus } from "lucide-react";
import { useTranslation } from "react-i18next";
import { LoadMore } from "#/components/ui/load-more";
import { EmptyState } from "#/components/ui/page";
import { Tabs } from "#/components/ui/tabs";
import { CategoryBadge } from "#/features/announcements";
import { announcementAdminClient } from "../api";
import { formatDateTime } from "../format";
import { announcementStatusKey } from "../labels";
import { ConsolePage } from "../ui/console-page";
import { type Column, DataTable } from "../ui/data-table";
import { ErrorState, RowsSkeleton } from "../ui/states";
import { type Tone, ToneChip } from "../ui/tone";
import { UserCell } from "../ui/user";
import type { ManagedAnnouncementsSearch, StatusFilter } from "./list-search";

const STATUSES: Record<StatusFilter, admin.AnnouncementStatus> = {
  draft: admin.AnnouncementStatus.DRAFT,
  scheduled: admin.AnnouncementStatus.SCHEDULED,
  published: admin.AnnouncementStatus.PUBLISHED,
};

const STATUS_TONES: Partial<Record<admin.AnnouncementStatus, Tone>> = {
  [admin.AnnouncementStatus.SCHEDULED]: "info",
  [admin.AnnouncementStatus.PUBLISHED]: "good",
};

export function AnnouncementStatusChip({ status }: { status: admin.AnnouncementStatus }) {
  const { t } = useTranslation("admin");
  return (
    <ToneChip tone={STATUS_TONES[status] ?? "neutral"}>{t(announcementStatusKey(status))}</ToneChip>
  );
}

/** `/admin/announcements`: every announcement in any state. */
export function ManagedAnnouncementsScreen({ search }: { search: ManagedAnnouncementsSearch }) {
  const { t } = useTranslation("admin");
  const navigate = useNavigate();
  const list = useInfiniteQuery({
    queryKey: ["admin", "announcements", "managed", "list", search.status ?? "all"],
    queryFn: ({ pageParam }) =>
      announcementAdminClient().listManagedAnnouncements({
        page: { cursor: pageParam, limit: 30 },
        status: search.status ? STATUSES[search.status] : admin.AnnouncementStatus.UNSPECIFIED,
      }),
    initialPageParam: "",
    getNextPageParam: (last) => last.page?.nextCursor || undefined,
  });
  const announcements = list.data?.pages.flatMap((page) => page.announcements) ?? [];

  const columns: Column<admin.ManagedAnnouncement>[] = [
    {
      key: "title",
      header: t("announcements.columns.title"),
      card: "title",
      cell: (row) => (
        <span className="flex min-w-0 flex-col">
          <Link
            to="/admin/announcements/$announcementId"
            params={{ announcementId: row.id }}
            className="line-clamp-2 font-bold hover:underline"
          >
            {row.pinned && (
              <Pin
                className="mr-1 inline size-3.5 text-brand"
                aria-label={t("announcements.fields.pinned")}
              />
            )}
            {row.title?.ja || t("announcements.untitled")}
          </Link>
          {row.title?.en && (
            <span className="line-clamp-1 text-xs font-normal text-ink-soft">{row.title.en}</span>
          )}
        </span>
      ),
    },
    {
      key: "status",
      header: t("announcements.columns.status"),
      card: "bare",
      cell: (row) => <AnnouncementStatusChip status={row.status} />,
    },
    {
      key: "category",
      header: t("announcements.columns.category"),
      card: "bare",
      cell: (row) => <CategoryBadge category={row.category} />,
    },
    {
      key: "publishedAt",
      header: t("announcements.columns.publishedAt"),
      className: "whitespace-nowrap",
      cell: (row) => <span className="text-ink-soft">{formatDateTime(row.publishedAt)}</span>,
    },
    {
      key: "notify",
      header: t("announcements.columns.notify"),
      cell: (row) =>
        row.notifyUsers ? (
          <span className="inline-flex items-center gap-1 whitespace-nowrap text-ink-soft">
            <Bell className="size-3.5" />
            {row.notifiedAt
              ? t("announcements.notifiedCount", { count: row.notifiedCount })
              : t("announcements.notifyPending")}
          </span>
        ) : (
          <span className="text-ink-faint">—</span>
        ),
    },
    {
      key: "author",
      header: t("announcements.columns.author"),
      cell: (row) => <UserCell user={row.author} />,
    },
    {
      key: "updated",
      header: t("announcements.columns.updated"),
      className: "whitespace-nowrap",
      cell: (row) => <span className="text-ink-soft">{formatDateTime(row.updatedAt)}</span>,
    },
  ];

  return (
    <ConsolePage
      title={t("announcements.title")}
      actions={
        <Link
          to="/admin/announcements/new"
          className="pressable sticker inline-flex h-9 items-center gap-2 rounded-full bg-brand px-4 text-sm font-bold text-on-brand"
        >
          <Plus className="size-4" />
          {t("announcements.create")}
        </Link>
      }
    >
      <Tabs<StatusFilter | "all">
        className="mb-3"
        value={search.status ?? "all"}
        onChange={(status) =>
          navigate({
            to: "/admin/announcements",
            search: { status: status === "all" ? undefined : status },
            replace: true,
          })
        }
        items={[
          { value: "all", label: t("common.all") },
          { value: "draft", label: t("enums.announcementStatus.DRAFT") },
          { value: "scheduled", label: t("enums.announcementStatus.SCHEDULED") },
          { value: "published", label: t("enums.announcementStatus.PUBLISHED") },
        ]}
      />
      {list.isPending ? (
        <RowsSkeleton />
      ) : list.isError ? (
        <ErrorState error={list.error} onRetry={() => list.refetch()} />
      ) : announcements.length === 0 ? (
        <EmptyState
          emoji="📣"
          title={t(search.status ? "announcements.emptyFiltered" : "announcements.empty")}
          body={search.status ? undefined : t("announcements.emptyBody")}
        />
      ) : (
        <>
          <DataTable
            label={t("announcements.title")}
            columns={columns}
            rows={announcements}
            rowKey={(row) => row.id}
            onRowClick={(row) =>
              navigate({
                to: "/admin/announcements/$announcementId",
                params: { announcementId: row.id },
              })
            }
          />
          <LoadMore query={list} />
        </>
      )}
    </ConsolePage>
  );
}
