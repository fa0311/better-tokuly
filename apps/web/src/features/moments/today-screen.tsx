import { Link } from "@tanstack/react-router";
import type { moments } from "@tokuly/api";
import { countdownText, momentPhase } from "@tokuly/core/moments";
import { CalendarDays, Camera, Settings2, Smile } from "lucide-react";
import { useState } from "react";
import { Button } from "#/components/ui/button";
import { LoadMore } from "#/components/ui/load-more";
import { EmptyState, Page, Skeleton } from "#/components/ui/page";
import { cn } from "#/lib/cn";
import { useTranslation } from "#/lib/use-translation";
import { useFriendMoments, useToday } from "./api";
import { RealmojiLibraryDialog } from "./library-dialog";
import { MomentCard } from "./moment-card";
import { OwnMoment } from "./own-moment";
import { MomentSettingsDialog } from "./settings-dialog";
import { useNow } from "./use-now";

/**
 * いまの瞬間: where today stands (waiting for the prompt, the two minutes
 * running, late, shared), the caller's own moment, and friends' moments of
 * their day, blurred until the caller shares theirs.
 */
export function MomentsTodayScreen() {
  const { t } = useTranslation("moments");
  const today = useToday();
  const friends = useFriendMoments();
  const [dialog, setDialog] = useState<"library" | "settings" | null>(null);
  const list = friends.data?.pages.flatMap((page) => page.moments) ?? [];

  return (
    <Page
      title={t("title")}
      back="/me"
      actions={
        <div className="flex items-center gap-1">
          <Link
            to="/moments/calendar"
            aria-label={t("today.calendar")}
            className="pressable grid size-9 place-items-center rounded-full hover:bg-surface-2"
          >
            <CalendarDays className="size-5" />
          </Link>
          <Button
            variant="ghost"
            size="icon-sm"
            aria-label={t("today.library")}
            onClick={() => setDialog("library")}
          >
            <Smile className="size-5" />
          </Button>
          <Button
            variant="ghost"
            size="icon-sm"
            aria-label={t("today.settings")}
            onClick={() => setDialog("settings")}
          >
            <Settings2 className="size-5" />
          </Button>
        </div>
      }
    >
      {today.isPending ? <Skeleton className="h-40" /> : <DayCard today={today.data} />}

      <h2 className="mt-6 mb-3 text-lg font-black">{t("today.friends")}</h2>
      {friends.isPending ? (
        <div className="grid gap-4 sm:grid-cols-2">
          <Skeleton className="aspect-[3/4]" />
          <Skeleton className="aspect-[3/4]" />
        </div>
      ) : list.length === 0 ? (
        <EmptyState emoji="🫧" title={t("today.friendsEmpty")} body={t("today.friendsEmptyBody")} />
      ) : (
        <>
          <ul className="grid gap-4 sm:grid-cols-2" data-testid="friend-moments">
            {list.map((moment) => (
              <li key={moment.id}>
                <MomentCard moment={moment} canTake={Boolean(today.data?.day)} />
              </li>
            ))}
          </ul>
          <LoadMore query={friends} />
        </>
      )}

      <RealmojiLibraryDialog open={dialog === "library"} onClose={() => setDialog(null)} />
      <MomentSettingsDialog open={dialog === "settings"} onClose={() => setDialog(null)} />
    </Page>
  );
}

function DayCard({ today }: { today: moments.GetTodayResponse | undefined }) {
  const { t } = useTranslation("moments");
  const running = Boolean(today?.day && !today.posted);
  const now = useNow(250, running);
  const phase = momentPhase(today, now);

  if (phase.kind === "posted")
    return today?.mine ? (
      <OwnMoment moment={today.mine} />
    ) : (
      <section className="sticker rounded-card bg-surface p-5 text-center" data-testid="moment-day">
        <p className="text-4xl">🌙</p>
        <p className="mt-2 font-black">{t("today.postedDeleted")}</p>
      </section>
    );

  const open = phase.kind === "open";
  return (
    <section
      className={cn(
        "sticker rounded-card p-5",
        open
          ? "bg-sun text-[#17122b]"
          : phase.kind === "late"
            ? "bg-pink text-[#17122b]"
            : "bg-surface",
      )}
      data-testid="moment-day"
      data-phase={phase.kind}
    >
      <h2 className="text-xl font-black">
        {open
          ? t("today.openTitle")
          : phase.kind === "late"
            ? t("today.lateTitle")
            : t("today.waitingTitle")}
      </h2>
      <p
        className={cn(
          "mt-1.5 text-sm font-bold",
          !open && phase.kind !== "late" && "text-ink-soft",
        )}
      >
        {open
          ? t("today.openBody")
          : phase.kind === "late"
            ? t("today.lateBody")
            : t("today.waitingBody")}
      </p>
      {open && (
        <p
          className="mt-3 text-5xl font-black tabular-nums motion-safe:animate-pulse"
          role="timer"
          data-testid="moment-countdown"
        >
          {countdownText(phase.leftMs)}
        </p>
      )}
      {(open || phase.kind === "late") && (
        <Link
          to="/moments/camera"
          className="pressable sticker mt-4 inline-flex h-14 items-center gap-2 rounded-full bg-surface px-7 text-lg font-black text-ink"
        >
          <Camera className="size-5" />
          {open ? t("today.take") : t("today.takeLate")}
        </Link>
      )}
    </section>
  );
}
