import { useMutation, useQueryClient } from "@tanstack/react-query";
import { type feed, fromDate, toMillis } from "@tokuly/api";
import { Ellipsis, EyeOff, Sparkles, Trash2 } from "lucide-react";
import { type ReactNode, useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { Button } from "#/components/ui/button";
import { ConfirmDialog } from "#/components/ui/confirm";
import { Dialog } from "#/components/ui/dialog";
import { LoadMore } from "#/components/ui/load-more";
import { Menu } from "#/components/ui/menu";
import { EmptyState, Page, Skeleton } from "#/components/ui/page";
import { SwitchRow } from "#/components/ui/switch";
import { Tabs } from "#/components/ui/tabs";
import { ReportsList } from "#/features/engagement";
import { cn } from "#/lib/cn";
import { errorMessage } from "#/lib/errors";
import { ActivityCardView, ActivityRow } from "./activity-card";
import type { ActivityTab } from "./activity-search";
import {
  activityClient,
  feedKeys,
  useActivityPreferences,
  useFriendsActivity,
  useMyActivity,
} from "./api";
import { dayLabel, groupByDay } from "./days";

/**
 * The log: everything the viewer did (which they can prune, clear and hide
 * from others kind by kind), what their friends have been doing, and the
 * reports the viewer filed.
 */
export function ActivityScreen({
  tab,
  onTabChange,
}: {
  tab: ActivityTab;
  onTabChange(tab: ActivityTab): void;
}) {
  const { t } = useTranslation("notes");
  const [sheet, setSheet] = useState<"clear" | "preferences">();

  return (
    <Page
      title={t("activity.title")}
      back="/me"
      actions={
        tab === "mine" && (
          <Menu
            trigger={
              <Button variant="ghost" size="icon-sm" aria-label={t("activity.menu")}>
                <Ellipsis className="size-5" />
              </Button>
            }
            actions={[
              {
                label: t("activity.preferences.open"),
                icon: <EyeOff />,
                onSelect: () => setSheet("preferences"),
              },
              {
                label: t("activity.clear.open"),
                icon: <Trash2 />,
                danger: true,
                onSelect: () => setSheet("clear"),
              },
            ]}
          />
        )
      }
    >
      <Tabs<ActivityTab>
        className="mb-4"
        value={tab}
        onChange={onTabChange}
        items={[
          { value: "mine", label: t("activity.tabs.mine") },
          { value: "friends", label: t("activity.tabs.friends") },
          { value: "reports", label: t("activity.tabs.reports") },
        ]}
      />
      {tab === "mine" && <MyLog />}
      {tab === "friends" && <FriendsLog />}
      {tab === "reports" && <ReportsList />}
      {sheet === "clear" && <ClearDialog onClose={() => setSheet(undefined)} />}
      {sheet === "preferences" && <PreferencesDialog onClose={() => setSheet(undefined)} />}
    </Page>
  );
}

/**
 * The kinds a filter offers: those the log actually holds. A family (`note`)
 * and its members (`note.posted`, `note.replied`) can carry the same name;
 * each name is offered once, as its widest kind.
 */
function filterKinds(
  kinds: feed.ActivityKindPreference[] | undefined,
): feed.ActivityKindPreference[] {
  const byLabel = new Map<string, feed.ActivityKindPreference>();
  for (const entry of kinds ?? []) {
    if (entry.entryCount <= 0) continue;
    const known = byLabel.get(entry.label);
    if (!known || entry.kind.length < known.kind.length) byLabel.set(entry.label, entry);
  }
  return [...byLabel.values()];
}

function RowsSkeleton() {
  return (
    <div className="flex flex-col gap-2">
      {[0, 1, 2, 3].map((index) => (
        <Skeleton key={index} className="h-16" />
      ))}
    </div>
  );
}

function MyLog() {
  const { t, i18n } = useTranslation("notes");
  const queryClient = useQueryClient();
  const [kind, setKind] = useState("");
  const query = useMyActivity(kind);
  const preferences = useActivityPreferences();
  const [removing, setRemoving] = useState<feed.Activity>();
  const activities = query.data?.pages.flatMap((page) => page.activities) ?? [];
  const kinds = filterKinds(preferences.data);

  const remove = useMutation({
    mutationFn: (activity: feed.Activity) =>
      activityClient().deleteActivity({ activityId: activity.id }),
    onSuccess: async () => {
      setRemoving(undefined);
      await queryClient.invalidateQueries({ queryKey: feedKeys.all });
      toast(t("activity.deleted"));
    },
    onError: (error) => toast.error(errorMessage(error)),
  });

  return (
    <>
      {kinds.length > 1 && (
        // biome-ignore lint/a11y/useSemanticElements: a row of filter chips, not a form group.
        <div
          className="scrollbar-none -mx-4 mb-3 flex gap-1.5 overflow-x-auto px-4"
          role="group"
          aria-label={t("activity.filter")}
        >
          <KindChip active={kind === ""} onClick={() => setKind("")}>
            {t("activity.allKinds")}
          </KindChip>
          {kinds.map((entry) => (
            <KindChip
              key={entry.kind}
              active={kind === entry.kind}
              onClick={() => setKind(entry.kind)}
            >
              {entry.label}
              <span className="opacity-60 tabular-nums">{entry.entryCount}</span>
            </KindChip>
          ))}
        </div>
      )}
      {query.isPending ? (
        <RowsSkeleton />
      ) : activities.length === 0 ? (
        <EmptyState
          emoji="🐾"
          title={t("activity.empty.mine")}
          body={t("activity.empty.mineBody")}
        />
      ) : (
        <div className="flex flex-col gap-4">
          {groupByDay(activities, (activity) => toMillis(activity.occurredAt)).map((day) => (
            <section key={day.key}>
              <h2 className="mb-1.5 px-1 text-sm font-black text-ink-soft">
                {dayLabel(day.date, i18n.language, t)}
              </h2>
              <div className="flex flex-col gap-1.5">
                {day.items.map((activity) => (
                  <ActivityRow
                    key={activity.id}
                    activity={activity}
                    showAudience
                    trailing={
                      <button
                        type="button"
                        aria-label={t("activity.delete")}
                        onClick={() => setRemoving(activity)}
                        className="grid size-9 shrink-0 place-items-center rounded-full text-ink-faint hover:bg-surface-2 hover:text-danger"
                      >
                        <Trash2 className="size-4" />
                      </button>
                    }
                  />
                ))}
              </div>
            </section>
          ))}
          <LoadMore query={query} showEnd />
        </div>
      )}
      <ConfirmDialog
        open={removing !== undefined}
        onOpenChange={(open) => !open && setRemoving(undefined)}
        title={t("activity.deleteTitle")}
        description={t("activity.deleteBody")}
        confirmLabel={t("activity.deleteConfirm")}
        danger
        loading={remove.isPending}
        onConfirm={() => removing && remove.mutate(removing)}
      >
        {removing && (
          <p className="rounded-2xl bg-surface-2 px-3 py-2.5 text-[15px] font-bold">
            {removing.summary}
          </p>
        )}
      </ConfirmDialog>
    </>
  );
}

function KindChip({
  active,
  onClick,
  children,
}: {
  active: boolean;
  onClick(): void;
  children: ReactNode;
}) {
  return (
    <button
      type="button"
      aria-pressed={active}
      onClick={onClick}
      className={cn(
        "inline-flex h-8 shrink-0 items-center gap-1.5 rounded-full border-2 px-3 text-sm font-black",
        active
          ? "border-ink bg-ink text-surface"
          : "border-line bg-surface text-ink-soft hover:border-ink-faint",
      )}
    >
      {children}
    </button>
  );
}

function FriendsLog() {
  const { t } = useTranslation("notes");
  const query = useFriendsActivity("");
  const cards = query.data?.pages.flatMap((page) => page.cards) ?? [];
  if (query.isPending) return <RowsSkeleton />;
  if (cards.length === 0)
    return (
      <EmptyState
        emoji="👀"
        title={t("activity.empty.friends")}
        body={t("activity.empty.friendsBody")}
      />
    );
  return (
    <div className="flex flex-col gap-2.5">
      {cards.map(
        (card) => card.activity && <ActivityCardView key={card.activity.id} card={card} />,
      )}
      <LoadMore query={query} showEnd />
    </div>
  );
}

type ClearRange = "today" | "week" | "month" | "everything";

/** Where a range starts (it always runs up to now); `undefined` = the whole log. */
function rangeStart(range: ClearRange, now = new Date()): Date | undefined {
  const start = new Date(now.getFullYear(), now.getMonth(), now.getDate());
  if (range === "week") start.setDate(start.getDate() - 6);
  else if (range === "month") start.setDate(start.getDate() - 29);
  else if (range === "everything") return undefined;
  return start;
}

/** Removes the viewer's entries of a time range, optionally of one kind only. */
function ClearDialog({ onClose }: { onClose(): void }) {
  const { t } = useTranslation("notes");
  const queryClient = useQueryClient();
  const preferences = useActivityPreferences();
  const [range, setRange] = useState<ClearRange>("today");
  const [kind, setKind] = useState("");
  const kinds = filterKinds(preferences.data);

  const clear = useMutation({
    mutationFn: () => {
      const from = rangeStart(range);
      return activityClient().clearActivity({
        from: from ? fromDate(from) : undefined,
        kindPrefix: kind,
        everything: !from,
      });
    },
    onSuccess: async (response) => {
      await queryClient.invalidateQueries({ queryKey: feedKeys.all });
      toast(t("activity.clear.done", { count: response.deletedCount }));
      onClose();
    },
    onError: (error) => toast.error(errorMessage(error)),
  });

  return (
    <ConfirmDialog
      open
      onOpenChange={(open) => !open && onClose()}
      title={t("activity.clear.title")}
      description={t("activity.clear.body")}
      confirmLabel={t("activity.clear.confirm")}
      danger
      loading={clear.isPending}
      onConfirm={() => clear.mutate()}
    >
      <div
        role="radiogroup"
        aria-label={t("activity.clear.range")}
        className="grid grid-cols-2 gap-1.5"
      >
        {(["today", "week", "month", "everything"] as const).map((value) => (
          // biome-ignore lint/a11y/useSemanticElements: a radio drawn as a tile, not a bare radio input.
          <button
            key={value}
            type="button"
            role="radio"
            aria-checked={range === value}
            onClick={() => setRange(value)}
            className={cn(
              "rounded-2xl border-2 px-3 py-2.5 text-left text-[15px] font-bold",
              range === value ? "border-ink bg-brand-soft" : "border-line hover:border-ink-faint",
            )}
          >
            {t(`activity.clear.ranges.${value}`)}
          </button>
        ))}
      </div>
      {kinds.length > 1 && (
        <div className="mt-4">
          <p className="mb-1.5 px-1 text-sm font-bold text-ink-soft">{t("activity.clear.kind")}</p>
          <div className="flex flex-wrap gap-1.5">
            <KindChip active={kind === ""} onClick={() => setKind("")}>
              {t("activity.allKinds")}
            </KindChip>
            {kinds.map((entry) => (
              <KindChip
                key={entry.kind}
                active={kind === entry.kind}
                onClick={() => setKind(entry.kind)}
              >
                {entry.label}
              </KindChip>
            ))}
          </div>
        </div>
      )}
    </ConfirmDialog>
  );
}

/** Which kinds of the viewer's activity other people get to see. Hiding a kind hides its past entries too. */
function PreferencesDialog({ onClose }: { onClose(): void }) {
  const { t } = useTranslation("notes");
  const queryClient = useQueryClient();
  const preferences = useActivityPreferences();

  const change = useMutation({
    mutationFn: (next: { kind: string; visibleToOthers: boolean }) =>
      activityClient().setActivityKindVisibility(next),
    onMutate: (next) => {
      queryClient.setQueryData<feed.ActivityKindPreference[]>(feedKeys.preferences, (current) =>
        current?.map((entry) =>
          entry.kind === next.kind ? { ...entry, visibleToOthers: next.visibleToOthers } : entry,
        ),
      );
    },
    onSuccess: (response) => {
      queryClient.setQueryData(feedKeys.preferences, response.kinds);
      // The viewer's own log marks hidden entries.
      void queryClient.invalidateQueries({ queryKey: feedKeys.activity });
    },
    onError: (error) => {
      void queryClient.invalidateQueries({ queryKey: feedKeys.preferences });
      toast.error(errorMessage(error));
    },
  });

  return (
    <Dialog
      open
      onOpenChange={(open) => !open && onClose()}
      title={t("activity.preferences.title")}
      description={t("activity.preferences.body")}
    >
      {preferences.isPending ? (
        <RowsSkeleton />
      ) : (
        <div className="flex flex-col">
          {preferences.data?.map((entry) => (
            <SwitchRow
              key={entry.kind}
              label={entry.label}
              // Several kinds can share a name (a family and its members): the kind itself tells them apart.
              hint={
                [
                  sameLabel(preferences.data, entry) ? entry.kind : "",
                  entry.entryCount > 0
                    ? t("activity.preferences.count", { count: entry.entryCount })
                    : "",
                ]
                  .filter(Boolean)
                  .join(" · ") || undefined
              }
              checked={entry.visibleToOthers}
              disabled={change.isPending}
              onChange={(visibleToOthers) => change.mutate({ kind: entry.kind, visibleToOthers })}
            />
          ))}
          <p className="mt-3 flex items-start gap-2 rounded-2xl bg-surface-2 px-3 py-2.5 text-sm leading-relaxed text-ink-soft">
            <Sparkles className="mt-0.5 size-4 shrink-0 text-brand" />
            {t("activity.preferences.note")}
          </p>
        </div>
      )}
    </Dialog>
  );
}

function sameLabel(
  kinds: feed.ActivityKindPreference[] | undefined,
  entry: feed.ActivityKindPreference,
): boolean {
  return (kinds ?? []).some((other) => other.kind !== entry.kind && other.label === entry.label);
}
