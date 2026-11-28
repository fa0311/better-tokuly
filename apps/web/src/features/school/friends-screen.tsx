import { useQuery } from "@tanstack/react-query";
import { Link } from "@tanstack/react-router";
import { errorReason, toMillis } from "@tokuly/api";
import { clock, dayLabel, schoolKeys } from "@tokuly/core/school";
import { ChevronRight, Copy, MessageCircle } from "lucide-react";
import { useState } from "react";
import { Avatar } from "#/components/ui/avatar";
import { Button } from "#/components/ui/button";
import { Chip } from "#/components/ui/card";
import { EmptyState, Skeleton } from "#/components/ui/page";
import { SelectField } from "#/components/ui/select";
import { Tabs } from "#/components/ui/tabs";
import { openDirectChat } from "#/features/chat";
import { EmojiText } from "#/features/emoji";
import { useAccountToday } from "#/lib/account-time";
import { cn } from "#/lib/cn";
import { useTranslation } from "#/lib/use-translation";
import { timetableClient, useTerms } from "./api";

import { DayView } from "./day-view";
import { TimetableGrid } from "./grid";
import { Block, RowsSkeleton, SchoolPage, SubjectPill, useNow } from "./parts";
import { CopyDialog } from "./term-dialogs";

export type FriendsTab = "timetables" | "free";

function useFriendTimetables() {
  return useQuery({
    queryKey: schoolKeys.friendTimetables,
    queryFn: async () =>
      (await timetableClient().listFriendTimetables({ page: { limit: 100 } })).timetables,
  });
}

/** Friends who share their timetable (with what they are in right now), and the free periods shared with them. */
export function SchoolFriendsScreen({
  tab,
  onTabChange,
}: {
  tab: FriendsTab;
  onTabChange(tab: FriendsTab): void;
}) {
  const { t } = useTranslation("school");
  return (
    <SchoolPage back="/school/timetable" title={t("friends.title")}>
      <Tabs
        className="mb-4"
        value={tab}
        onChange={onTabChange}
        items={[
          { value: "timetables", label: t("friends.tabs.timetables") },
          { value: "free", label: t("friends.tabs.free") },
        ]}
      />
      {tab === "timetables" ? <FriendTimetables /> : <FreePeriods />}
    </SchoolPage>
  );
}

function FriendTimetables() {
  const { t } = useTranslation("school");
  const timetables = useFriendTimetables();
  const now = useNow(30_000);
  const today = useAccountToday();
  if (timetables.isPending) return <RowsSkeleton />;
  const list = timetables.data ?? [];
  if (list.length === 0)
    return <EmptyState emoji="🤝" title={t("friends.empty")} body={t("friends.emptyBody")} />;
  return (
    <ul className="flex flex-col gap-2">
      {list.map((item) => {
        const current =
          item.now?.current && toMillis(item.now.current.endsAt) > now
            ? item.now.current
            : undefined;
        const next = item.now?.next;
        return (
          <li key={item.user?.id}>
            <Link
              to="/school/timetable/$userId"
              params={{ userId: item.user?.id ?? "" }}
              className="pressable sticker flex items-center gap-3 rounded-card bg-surface p-3"
            >
              <Avatar user={item.user} outlined />
              <span className="min-w-0 flex-1">
                <span className="block truncate font-black">
                  <EmojiText text={item.user?.displayName ?? ""} />
                </span>
                <span className="flex flex-wrap items-center gap-1.5 text-xs font-bold text-ink-soft">
                  {current ? (
                    <>
                      <span className="rounded-full bg-live px-2 py-0.5 text-[11px] font-black text-white">
                        {t("now.current")}
                      </span>
                      <SubjectPill
                        subject={current.subject}
                        className="h-5 border px-1.5 text-[11px]"
                      />
                      <span>〜{clock(current.endMinute)}</span>
                    </>
                  ) : next && next.date === today ? (
                    <>
                      <span>{t("now.next")}</span>
                      <SubjectPill
                        subject={next.subject}
                        className="h-5 border px-1.5 text-[11px]"
                      />
                      <span>{clock(next.startMinute)}〜</span>
                    </>
                  ) : (
                    <span>{t("friends.noClassNow")}</span>
                  )}
                </span>
              </span>
              <ChevronRight className="size-5 shrink-0 text-ink-faint" />
            </Link>
          </li>
        );
      })}
    </ul>
  );
}

/** 空きコマ: the caller's free periods of the coming week, each with the friends who are free then too. */
function FreePeriods() {
  const { t, i18n } = useTranslation("school");
  const sharing = useFriendTimetables();
  const [only, setOnly] = useState<string[]>([]);
  const start = useAccountToday();
  const free = useQuery({
    queryKey: schoolKeys.free(start, only),
    queryFn: () =>
      timetableClient().findCommonFreePeriods({ startDate: start, days: 7, userIds: only }),
  });
  const people = (sharing.data ?? []).flatMap((item) => (item.user ? [item.user] : []));
  const periods = free.data?.periods ?? [];
  const dates = [...new Set(periods.map((period) => period.date))];

  return (
    <>
      <p className="mb-3 rounded-card bg-surface-2 px-4 py-3 text-sm text-ink-soft">
        {t("free.explain")}
      </p>
      {people.length > 0 && (
        <div className="scrollbar-none -mx-4 mb-4 flex gap-1.5 overflow-x-auto px-4">
          <button
            type="button"
            aria-pressed={only.length === 0}
            onClick={() => setOnly([])}
            className={cn(
              "h-9 shrink-0 rounded-full px-4 text-sm font-bold",
              only.length === 0 ? "sticker-sm bg-ink text-surface" : "bg-surface-2 text-ink-soft",
            )}
          >
            {t("free.everyone")}
          </button>
          {people.map((user) => {
            const on = only.includes(user.id);
            return (
              <button
                key={user.id}
                type="button"
                aria-pressed={on}
                onClick={() =>
                  setOnly(on ? only.filter((id) => id !== user.id) : [...only, user.id])
                }
                className={cn(
                  "flex h-9 shrink-0 items-center gap-1.5 rounded-full pr-3 pl-1 text-sm font-bold",
                  on ? "sticker-sm bg-lime text-[#17122b]" : "bg-surface-2 text-ink-soft",
                )}
              >
                <Avatar user={user} size="xs" />
                <EmojiText text={user.displayName} />
              </button>
            );
          })}
        </div>
      )}
      {free.isPending ? (
        <RowsSkeleton />
      ) : periods.length === 0 ? (
        <EmptyState emoji="🗓️" title={t("free.empty")} body={t("free.emptyBody")} />
      ) : (
        dates.map((date) => (
          <Block
            key={date}
            title={
              date === start
                ? t("free.today", { date: dayLabel(date, i18n.language) })
                : dayLabel(date, i18n.language)
            }
          >
            <ul className="flex flex-col gap-1.5">
              {periods
                .filter((period) => period.date === date)
                .map((period) => (
                  <li
                    key={period.period}
                    className={cn(
                      "flex items-center gap-3 rounded-2xl px-3 py-2.5",
                      period.freeFriends.length > 0 ? "sticker-sm bg-surface" : "bg-surface-2",
                    )}
                  >
                    <span className="flex w-[4.75rem] shrink-0 flex-col items-center leading-tight whitespace-nowrap">
                      <span className="font-black">
                        {t("day.period", { period: period.period })}
                      </span>
                      <span className="text-[11px] font-bold text-ink-faint">
                        {clock(period.startMinute)}〜{clock(period.endMinute)}
                      </span>
                    </span>
                    <span className="min-w-0 flex-1">
                      {period.freeFriends.length === 0 ? (
                        <span className="text-sm font-bold text-ink-faint">{t("free.alone")}</span>
                      ) : (
                        <span className="flex flex-wrap gap-1.5">
                          {period.freeFriends.map((user) => (
                            <button
                              key={user.id}
                              type="button"
                              onClick={() => void openDirectChat(user.id)}
                              aria-label={t("free.message", { name: user.displayName })}
                              className="flex h-8 items-center gap-1.5 rounded-full bg-brand-soft pr-2.5 pl-1 text-sm font-bold"
                            >
                              <Avatar user={user} size="xs" />
                              <span className="max-w-24 truncate">
                                <EmojiText text={user.displayName} />
                              </span>
                              <MessageCircle className="size-3.5 text-ink-soft" />
                            </button>
                          ))}
                        </span>
                      )}
                    </span>
                    {period.betweenClasses && (
                      <Chip className="shrink-0 bg-sun text-[#17122b]">{t("free.between")}</Chip>
                    )}
                  </li>
                ))}
            </ul>
          </Block>
        ))
      )}
      {(free.data?.notSharedUserIds.length ?? 0) > 0 && (
        <p className="mt-4 px-1 text-xs text-ink-faint">
          {t("free.notShared", { count: free.data?.notSharedUserIds.length })}
        </p>
      )}
    </>
  );
}

/** A friend's timetable as they share it: what they are in now, their day, the weekly grid, and copying it. */
export function FriendTimetableScreen({ userId }: { userId: string }) {
  const { t, i18n } = useTranslation("school");
  const [termId, setTermId] = useState("");
  const [copying, setCopying] = useState(false);
  const now = useNow(30_000);
  const date = useAccountToday();
  const timetable = useQuery({
    queryKey: schoolKeys.friendTimetable(userId, termId),
    queryFn: () => timetableClient().getFriendTimetable({ userId, termId }),
    retry: false,
  });
  const friend = useQuery({
    queryKey: schoolKeys.friendTimetables,
    queryFn: async () =>
      (await timetableClient().listFriendTimetables({ page: { limit: 100 } })).timetables,
    select: (list) => list.find((item) => item.user?.id === userId)?.user,
  });
  const classNow = useQuery({
    queryKey: schoolKeys.now(userId),
    queryFn: async () => (await timetableClient().getNow({ userId })).now,
    refetchInterval: 60_000,
    retry: false,
  });
  const today = useQuery({
    queryKey: schoolKeys.day(date, userId),
    queryFn: async () => (await timetableClient().getDay({ userId, date })).day,
    retry: false,
  });
  const ownTerms = useTerms();
  const name = friend.data?.displayName ?? t("friends.someone");

  if (timetable.isPending) {
    return (
      <SchoolPage back="/school/friends" title={name} wide>
        <Skeleton className="h-96" />
      </SchoolPage>
    );
  }
  if (timetable.isError || !timetable.data?.term) {
    const hidden = errorReason(timetable.error) === "TIMETABLE_NOT_SHARED" || !timetable.error;
    return (
      <SchoolPage back="/school/friends" title={t("friends.title")}>
        <EmptyState
          emoji="🙈"
          title={hidden ? t("errors:TIMETABLE_NOT_SHARED") : t("loadFailed")}
          body={hidden ? t("friends.notSharedBody") : undefined}
        />
      </SchoolPage>
    );
  }
  const term = timetable.data.term;
  const terms = [term, ...timetable.data.otherTerms.filter((other) => other.id !== term.id)];
  const current =
    classNow.data?.current && toMillis(classNow.data.current.endsAt) > now
      ? classNow.data.current
      : undefined;

  return (
    <SchoolPage
      back="/school/friends"
      title={t("friends.timetableOf", { name })}
      wide
      actions={
        <Button
          size="sm"
          variant="lime"
          icon={<Copy className="size-4" />}
          onClick={() => setCopying(true)}
        >
          {t("copy.title")}
        </Button>
      }
    >
      <div className="lg:grid lg:grid-cols-[minmax(0,1fr)_22rem] lg:items-start lg:gap-6">
        <div className="min-w-0">
          <div className="mb-3 flex flex-wrap items-center gap-2">
            {friend.data && <Avatar user={friend.data} size="sm" />}
            {terms.length > 1 ? (
              <SelectField
                className="min-w-44 flex-1 sm:max-w-xs"
                value={term.id}
                onChange={setTermId}
                options={terms.map((item) => ({ value: item.id, label: item.name }))}
              />
            ) : (
              <h2 className="min-w-0 flex-1 truncate text-lg font-black">{term.name}</h2>
            )}
            {current && (
              <span className="flex items-center gap-1.5 text-xs font-bold text-ink-soft">
                <span className="rounded-full bg-live px-2 py-0.5 text-[11px] font-black text-white">
                  {t("now.current")}
                </span>
                <SubjectPill subject={current.subject} />
              </span>
            )}
          </div>
          <TimetableGrid layout={term.layout} entries={timetable.data.entries} />
        </div>
        <aside className="mt-6 lg:mt-0">
          <Block title={t("friends.todayOf", { date: dayLabel(date, i18n.language) })}>
            {today.isPending ? (
              <Skeleton className="h-40" />
            ) : today.data ? (
              <DayView day={today.data} nowMs={now} />
            ) : (
              <p className="text-sm text-ink-soft">{t("day.noClasses")}</p>
            )}
          </Block>
        </aside>
      </div>
      {copying && (
        <CopyDialog
          open
          onOpenChange={() => setCopying(false)}
          friend={{ id: userId, displayName: name }}
          term={term}
          ownTerms={ownTerms.data ?? []}
        />
      )}
    </SchoolPage>
  );
}
