import { useQueryClient } from "@tanstack/react-query";
import { Link, useRouter } from "@tanstack/react-router";
import { type notify, toMillis } from "@tokuly/api";
import { entriesOf, type InboxFilter } from "@tokuly/core/notifications";
import {
  CheckCheck,
  Inbox,
  MailCheck,
  MailOpen,
  MoreHorizontal,
  Settings,
  Trash2,
} from "lucide-react";
import { type ReactNode, useState } from "react";
import { CountBadge } from "#/components/ui/badge";
import { Button } from "#/components/ui/button";
import { ConfirmDialog } from "#/components/ui/confirm";
import { LoadMore } from "#/components/ui/load-more";
import { Menu } from "#/components/ui/menu";
import { EmptyState, Page, Skeleton } from "#/components/ui/page";
import { Tabs } from "#/components/ui/tabs";
import { cn } from "#/lib/cn";
import { errorMessage } from "#/lib/errors";
import { useTranslation } from "#/lib/use-translation";
import { useInbox, usePreferences, useUnread } from "./api";
import { CategoryIcon } from "./category";
import { openNotification, useInboxActions } from "./inbox-actions";
import { NotificationRow } from "./notification-row";
import { PushPrompt } from "./push-prompt";
import { type DayBucket, dayBucket, useNow } from "./when";

/** The filter tabs: everything, unread only, or one category (`c:<id>`). */
export type InboxTab = "all" | "unread" | `c:${string}`;

function filterOf(tab: InboxTab): InboxFilter {
  if (tab === "all") return { category: "", unreadOnly: false };
  if (tab === "unread") return { category: "", unreadOnly: true };
  return { category: tab.slice(2), unreadOnly: false };
}

interface TabEntry {
  value: InboxTab;
  label: string;
  icon: ReactNode;
  /** Unread entries behind the tab. */
  unread: number;
}

/** The notification inbox: what happened, newest first, filterable by category. */
export function InboxScreen({
  tab,
  onTabChange,
}: {
  tab: InboxTab;
  onTabChange(tab: InboxTab): void;
}) {
  const { t } = useTranslation("notifications");
  const queryClient = useQueryClient();
  const router = useRouter();
  const filter = filterOf(tab);
  const inbox = useInbox(filter);
  const unread = useUnread();
  const preferences = usePreferences();
  const actions = useInboxActions();
  const now = useNow();
  const [clearing, setClearing] = useState<"read" | "all" | null>(null);

  const categories = preferences.data?.categories ?? [];
  const tabs: TabEntry[] = [
    { value: "all", label: t("tabs.all"), icon: <Inbox className="size-4" />, unread: 0 },
    {
      value: "unread",
      label: t("tabs.unread"),
      icon: <MailOpen className="size-4" />,
      unread: unread.total,
    },
    ...categories.map(
      (category): TabEntry => ({
        value: `c:${category.category}`,
        label: category.label,
        icon: <CategoryIcon category={category.category} />,
        unread: unread.byCategory[category.category] ?? 0,
      }),
    ),
  ];
  const categoryLabel =
    categories.find((category) => category.category === filter.category)?.label ?? "";

  const entries = entriesOf(inbox.data);
  const unreadHere = filter.category ? (unread.byCategory[filter.category] ?? 0) : unread.total;

  const sections: { bucket: DayBucket; entries: notify.Notification[] }[] = [];
  for (const entry of entries) {
    const bucket = dayBucket(toMillis(entry.occurredAt), now);
    const last = sections.at(-1);
    if (last?.bucket === bucket) last.entries.push(entry);
    else sections.push({ bucket, entries: [entry] });
  }

  const clear = () => {
    if (!clearing) return;
    actions.clear.mutate(
      { category: filter.category, readOnly: clearing === "read" },
      { onSettled: () => setClearing(null) },
    );
  };

  return (
    <Page
      wide
      title={t("title")}
      actions={
        <>
          <Button
            variant="ghost"
            size="icon-sm"
            aria-label={t("actions.markAllRead")}
            title={t("actions.markAllRead")}
            disabled={unreadHere === 0 || actions.readAll.isPending}
            onClick={() =>
              actions.readAll.mutate({ category: filter.category, upToId: entries[0]?.id })
            }
          >
            <CheckCheck className="size-5" />
          </Button>
          <Link
            to="/notifications/settings"
            aria-label={t("actions.settings")}
            title={t("actions.settings")}
            className="grid size-9 place-items-center rounded-full hover:bg-surface-2"
          >
            <Settings className="size-5" />
          </Link>
          <Menu
            trigger={
              <Button variant="ghost" size="icon-sm" aria-label={t("actions.more")}>
                <MoreHorizontal className="size-5" />
              </Button>
            }
            actions={[
              {
                label: t("actions.clearRead"),
                icon: <MailCheck />,
                onSelect: () => setClearing("read"),
              },
              {
                label: t("actions.clearAll"),
                icon: <Trash2 />,
                danger: true,
                onSelect: () => setClearing("all"),
              },
            ]}
          />
        </>
      }
    >
      <div className="lg:grid lg:grid-cols-[14rem_minmax(0,1fr)] lg:items-start lg:gap-8">
        {/* Phones: a strip of pills. Wide screens: the same filters as a side list. */}
        <Tabs<InboxTab>
          className="mb-4 lg:hidden"
          value={tab}
          onChange={onTabChange}
          items={tabs.map((entry) => ({
            value: entry.value,
            label: entry.label,
            badge: entry.unread,
          }))}
        />
        <div
          role="tablist"
          aria-orientation="vertical"
          aria-label={t("tabs.label")}
          className="sticky top-20 hidden flex-col gap-1 lg:flex"
        >
          {tabs.map((entry) => {
            const active = entry.value === tab;
            return (
              <button
                key={entry.value}
                type="button"
                role="tab"
                aria-selected={active}
                onClick={() => onTabChange(entry.value)}
                className={cn(
                  "flex h-11 items-center gap-3 rounded-2xl px-3 text-left text-[15px] font-bold transition-colors",
                  active
                    ? "sticker-sm bg-surface text-ink"
                    : "text-ink-soft hover:bg-surface-2 hover:text-ink",
                )}
              >
                <span className="grid size-8 shrink-0 place-items-center">{entry.icon}</span>
                <span className="min-w-0 flex-1 truncate">{entry.label}</span>
                <CountBadge count={entry.unread} className="border-0" />
              </button>
            );
          })}
        </div>

        <div className="min-w-0 lg:max-w-3xl">
          {inbox.isPending ? (
            <div className="flex flex-col gap-2" aria-busy>
              {[0, 1, 2, 3, 4].map((i) => (
                <Skeleton key={i} className="h-[4.75rem]" />
              ))}
            </div>
          ) : inbox.isError ? (
            <EmptyState
              emoji="😵‍💫"
              title={errorMessage(inbox.error)}
              action={
                <Button variant="secondary" onClick={() => inbox.refetch()}>
                  {t("actions.retry")}
                </Button>
              }
            />
          ) : entries.length === 0 ? (
            <Empty tab={tab} categoryLabel={categoryLabel} />
          ) : (
            <>
              <PushPrompt />
              {sections.map((section) => (
                <section
                  key={section.bucket}
                  aria-label={t(`sections.${section.bucket}`)}
                  className="mb-4"
                >
                  <h2 className="px-2 pb-1.5 text-sm font-black text-ink-soft">
                    {t(`sections.${section.bucket}`)}
                  </h2>
                  <ul className="flex flex-col gap-2">
                    {section.entries.map((entry) => (
                      <NotificationRow
                        key={entry.id}
                        entry={entry}
                        now={now}
                        onOpen={() => openNotification(queryClient, router, entry)}
                        onRead={() => !entry.read && actions.read.mutate([entry.id])}
                        onDelete={() => actions.remove.mutate([entry.id])}
                      />
                    ))}
                  </ul>
                </section>
              ))}
              <LoadMore query={inbox} showEnd={entries.length > 15} />
            </>
          )}
        </div>
      </div>

      <ConfirmDialog
        open={clearing !== null}
        onOpenChange={(open) => !open && setClearing(null)}
        title={clearing === "read" ? t("clear.readTitle") : t("clear.allTitle")}
        description={[
          clearing === "read" ? t("clear.readBody") : t("clear.allBody"),
          categoryLabel && t("clear.onlyCategory", { category: categoryLabel }),
        ]
          .filter(Boolean)
          .join(" ")}
        confirmLabel={t("clear.confirm")}
        danger
        loading={actions.clear.isPending}
        onConfirm={clear}
      />
    </Page>
  );
}

function Empty({ tab, categoryLabel }: { tab: InboxTab; categoryLabel: string }) {
  const { t } = useTranslation("notifications");
  if (tab === "unread")
    return <EmptyState emoji="🎉" title={t("empty.unreadTitle")} body={t("empty.unreadBody")} />;
  if (tab !== "all")
    return (
      <EmptyState
        emoji="📭"
        title={t("empty.categoryTitle", { category: categoryLabel })}
        body={t("empty.categoryBody")}
      />
    );
  return (
    <EmptyState
      emoji="🔔"
      title={t("empty.title")}
      body={t("empty.body")}
      action={
        <Link
          to="/friends"
          className="pressable sticker inline-flex h-11 items-center rounded-full bg-brand px-5 font-bold text-on-brand"
        >
          {t("empty.action")}
        </Link>
      }
    />
  );
}
