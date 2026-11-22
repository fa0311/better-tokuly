import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { Link } from "@tanstack/react-router";
import { type common, errorReason, school, toMillis } from "@tokuly/api";
import { ChevronLeft, ChevronRight, DoorOpen, Megaphone } from "lucide-react";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { Avatar } from "#/components/ui/avatar";
import { Button } from "#/components/ui/button";
import { Dialog } from "#/components/ui/dialog";
import { Skeleton } from "#/components/ui/page";
import { EmojiText } from "#/features/emoji";
import { useAccountToday } from "#/lib/account-time";
import { cn } from "#/lib/cn";
import { keys, refresh, settingsClient, studyClient, toastError } from "./api";
import { addDays, dayLabel, studyTime } from "./dates";
import { Heatmap } from "./heatmap";
import { RowsSkeleton, SubjectPill, useNow } from "./parts";
import { isBreak, serverNow } from "./timer";

/** Sends "がんばれ!" to a friend who is studying. */
export function CheerButton({
  userId,
  name,
  size = "sm",
}: {
  userId: string;
  name: string;
  size?: "sm" | "md";
}) {
  const { t } = useTranslation("school");
  const [sent, setSent] = useState(false);
  const cheer = useMutation({
    mutationFn: () => studyClient().cheerStudy({ userId }),
    onSuccess: () => {
      setSent(true);
      toast.success(t("study.cheerSent", { name }), { icon: "📣" });
    },
    onError: (error) => {
      // One cheer per friend every few minutes: the last one still counts.
      if (errorReason(error) === "RATE_LIMITED") {
        setSent(true);
        toast(t("study.cheerTooSoon"));
      } else toastError(error);
    },
  });
  return (
    <Button
      size={size}
      variant={sent ? "secondary" : "lime"}
      icon={<Megaphone className="size-4" />}
      loading={cheer.isPending}
      disabled={sent}
      onClick={() => cheer.mutate()}
      aria-label={t("study.cheerFor", { name })}
    >
      {sent ? t("study.cheered_") : t("study.cheer")}
    </Button>
  );
}

/** Friends who are studying right now, longest session first, each with a cheer button. */
export function StudyingFriends() {
  const { t, i18n } = useTranslation("school");
  const { data, isPending } = useQuery({
    queryKey: keys.studying,
    queryFn: async () => (await studyClient().listStudyingFriends({})).friends,
  });
  const now = useNow(30_000);
  if (isPending) return <RowsSkeleton rows={2} />;
  const friends = data ?? [];
  if (friends.length === 0)
    return (
      <p className="rounded-card bg-surface-2 px-4 py-5 text-center text-sm font-bold text-ink-soft">
        {t("study.nobodyStudying")}
      </p>
    );
  return (
    <ul className="flex flex-col gap-1.5">
      {friends.map((friend) => {
        const running = friend.state === school.SessionState.RUNNING && !isBreak(friend.phase);
        const seconds =
          friend.studiedSeconds +
          (running
            ? Math.max(0, Math.floor((serverNow(now) - toMillis(friend.serverTime)) / 1000))
            : 0);
        const name = friend.user?.displayName ?? "";
        return (
          <li
            key={friend.sessionId}
            className="flex items-center gap-3 rounded-2xl bg-surface-2 px-3 py-2.5"
          >
            <Link
              to="/u/$handle"
              params={{ handle: friend.user?.handle ?? "" }}
              className="shrink-0"
            >
              <Avatar user={friend.user} />
            </Link>
            <span className="min-w-0 flex-1">
              <span className="block truncate font-bold">{name}</span>
              <span className="flex flex-wrap items-center gap-1.5 text-xs font-bold text-ink-soft">
                <span
                  className={cn(
                    "rounded-full px-2 py-0.5 text-[11px] font-black",
                    running ? "bg-brand text-on-brand" : "bg-sky text-[#17122b]",
                  )}
                >
                  {running ? t("study.studying") : t("study.resting")}
                </span>
                {friend.subject && (
                  <SubjectPill subject={friend.subject} className="h-5 border px-1.5 text-[11px]" />
                )}
                <span>{studyTime(seconds, i18n.language)}</span>
              </span>
            </span>
            {friend.roomId && (
              <Link
                to="/school/rooms/$roomId"
                params={{ roomId: friend.roomId }}
                aria-label={t("study.joinRoom", { title: friend.roomTitle })}
                title={friend.roomTitle}
                className="grid size-9 shrink-0 place-items-center rounded-full hover:bg-surface-3"
              >
                <DoorOpen className="size-5" />
              </Link>
            )}
            {friend.user && <CheerButton userId={friend.user.id} name={name} />}
          </li>
        );
      })}
    </ul>
  );
}

const MEDALS = ["🥇", "🥈", "🥉"];

/**
 * The week's ranking among friends who joined it. Only time the server
 * timed counts (the timer and study rooms); hand-logged time does not.
 */
export function Ranking() {
  const { t, i18n } = useTranslation("school");
  const queryClient = useQueryClient();
  const [anchor, setAnchor] = useState("");
  const today = useAccountToday();
  const [viewing, setViewing] = useState<common.UserSummary | undefined>(undefined);
  const { data, isPending } = useQuery({
    queryKey: keys.ranking(anchor),
    queryFn: () => studyClient().getStudyRanking({ anchorDate: anchor }),
  });
  const join = useMutation({
    mutationFn: () => settingsClient().updateSchoolSettings({ joinRanking: true }),
    onSuccess: () => {
      toast.success(t("ranking.joined"));
      refresh(queryClient, "settings", "study");
    },
    onError: toastError,
  });
  if (isPending || !data) return <Skeleton className="h-32" />;

  if (!data.joined) {
    return (
      <div className="flex flex-col items-center gap-3 rounded-card bg-surface-2 px-5 py-6 text-center">
        <span className="text-4xl">🏆</span>
        <p className="font-black">{t("ranking.joinTitle")}</p>
        <p className="max-w-xs text-sm text-ink-soft">{t("ranking.joinBody")}</p>
        <Button variant="lime" loading={join.isPending} onClick={() => join.mutate()}>
          {t("ranking.join")}
        </Button>
      </div>
    );
  }

  const thisWeek = data.endDate >= today;
  return (
    <div className="rounded-card bg-surface p-4">
      <div className="mb-2 flex items-center gap-1">
        <Button
          size="icon-sm"
          variant="ghost"
          aria-label={t("ranking.previousWeek")}
          onClick={() => setAnchor(addDays(data.startDate, -7))}
        >
          <ChevronLeft className="size-5" />
        </Button>
        <p className="min-w-0 flex-1 text-center text-sm font-black">
          {thisWeek
            ? t("ranking.thisWeek")
            : `${dayLabel(data.startDate, i18n.language)} 〜 ${dayLabel(data.endDate, i18n.language)}`}
        </p>
        <Button
          size="icon-sm"
          variant="ghost"
          aria-label={t("ranking.nextWeek")}
          disabled={thisWeek}
          onClick={() =>
            setAnchor(addDays(data.startDate, 7) >= today ? "" : addDays(data.startDate, 7))
          }
        >
          <ChevronRight className="size-5" />
        </Button>
      </div>
      {data.entries.length === 0 ? (
        <p className="py-4 text-center text-sm text-ink-soft">{t("ranking.empty")}</p>
      ) : (
        <ol className="flex flex-col gap-1" aria-label={t("ranking.title")}>
          {data.entries.map((entry) => (
            <li
              key={entry.user?.id}
              className={cn(
                "flex items-center gap-3 rounded-2xl px-2 py-2",
                entry.me && "sticker-sm bg-lime text-[#17122b]",
              )}
            >
              <span className="w-8 shrink-0 text-center text-lg font-black">
                {MEDALS[entry.rank - 1] ?? entry.rank}
              </span>
              <Avatar user={entry.user} size="sm" />
              {entry.me ? (
                <span className="min-w-0 flex-1 truncate font-bold">{t("ranking.me")}</span>
              ) : (
                <button
                  type="button"
                  onClick={() => setViewing(entry.user)}
                  className="min-w-0 flex-1 truncate text-left font-bold hover:underline"
                  aria-label={t("heatmap.of", { name: entry.user?.displayName ?? "" })}
                >
                  <EmojiText text={entry.user?.displayName ?? ""} />
                </button>
              )}
              <span className="shrink-0 text-sm font-black">
                {studyTime(entry.seconds, i18n.language)}
              </span>
            </li>
          ))}
        </ol>
      )}
      <p className="mt-3 text-xs text-ink-faint">{t("ranking.rule")}</p>
      {viewing && (
        <Dialog
          open
          onOpenChange={(open) => !open && setViewing(undefined)}
          title={t("heatmap.of", { name: viewing.displayName })}
        >
          <Heatmap userId={viewing.id} />
        </Dialog>
      )}
    </div>
  );
}
