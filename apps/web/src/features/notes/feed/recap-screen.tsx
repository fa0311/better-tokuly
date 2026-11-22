import { Link } from "@tanstack/react-router";
import { feed, isApiError } from "@tokuly/api";
import type { TFunction } from "i18next";
import { ChevronLeft, ChevronRight, Download, Share2 } from "lucide-react";
import { type ReactNode, useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { Avatar } from "#/components/ui/avatar";
import { Button } from "#/components/ui/button";
import { EmptyState, Page, Skeleton } from "#/components/ui/page";
import { Tabs } from "#/components/ui/tabs";
import { EmojiText } from "#/features/emoji";
import { copyText } from "#/features/engagement";
import { dopamineFactor } from "#/lib/appearance";
import { cn } from "#/lib/cn";
import { errorMessage } from "#/lib/errors";
import { kindEmoji } from "./activity-card";
import { useRecap } from "./api";
import { renderRecapImage } from "./recap-image";

/**
 * Which recap `/recap?p=…` shows: `2026-10` (a month), `2026` (a year);
 * absent = this month. A year is kept as a number, which is how the router
 * reads and writes a bare `2026` in the address.
 */
export interface RecapSearch {
  p?: string | number;
}

export function parseRecapSearch(search: Record<string, unknown>): RecapSearch {
  const p = search.p;
  if (typeof p === "number" && Number.isInteger(p) && p >= 2000 && p <= 9999) return { p };
  if (typeof p !== "string") return {};
  if (/^\d{4}$/.test(p)) return { p: Number(p) };
  return /^\d{4}-(0[1-9]|1[0-2])$/.test(p) ? { p } : {};
}

interface Period {
  period: feed.RecapPeriod;
  year: number;
  /** 1–12; 0 for a year. */
  month: number;
}

function toPeriod(p: string | number | undefined, now = new Date()): Period {
  if (!p)
    return { period: feed.RecapPeriod.MONTH, year: now.getFullYear(), month: now.getMonth() + 1 };
  const [year, month] = String(p).split("-").map(Number);
  return month
    ? { period: feed.RecapPeriod.MONTH, year: year!, month }
    : { period: feed.RecapPeriod.YEAR, year: year!, month: 0 };
}

const toParam = (period: Period): string | number =>
  period.month ? `${period.year}-${String(period.month).padStart(2, "0")}` : period.year;

function shift(period: Period, delta: number): Period {
  if (!period.month) return { ...period, year: period.year + delta };
  const index = period.year * 12 + (period.month - 1) + delta;
  return { ...period, year: Math.floor(index / 12), month: (index % 12) + 1 };
}

function isCurrent(period: Period, now = new Date()): boolean {
  return (
    period.year === now.getFullYear() && (!period.month || period.month === now.getMonth() + 1)
  );
}

/** Counts up to `value` once `active`, over a time that follows the ドパガキ度 (at once when motion is off). */
function useCountUp(value: number, active: boolean): number {
  const [shown, setShown] = useState(0);
  useEffect(() => {
    if (!active) return;
    const factor = window.matchMedia("(prefers-reduced-motion: reduce)").matches
      ? 0
      : dopamineFactor();
    const duration = 900 * factor;
    if (duration <= 0 || value <= 0) {
      setShown(value);
      return;
    }
    const started = performance.now();
    let frame = requestAnimationFrame(function step(now) {
      const progress = Math.min(1, (now - started) / duration);
      setShown(Math.round(value * (1 - (1 - progress) ** 3)));
      if (progress < 1) frame = requestAnimationFrame(step);
    });
    return () => cancelAnimationFrame(frame);
  }, [value, active]);
  return shown;
}

/**
 * "Your month / your year on Tokuly": the numbers of the viewer's own log as
 * a sequence of cards to swipe through, ending in a picture to share.
 */
export function RecapScreen({
  search,
  onSearchChange,
}: {
  search: RecapSearch;
  onSearchChange(next: RecapSearch): void;
}) {
  const { t } = useTranslation("notes");
  const period = toPeriod(search.p);
  const anchor = `${period.year}-${String(period.month || 1).padStart(2, "0")}-01`;
  const recap = useRecap(period.period, anchor);
  const go = (next: Period) =>
    onSearchChange(isCurrent(next) && next.month ? {} : { p: toParam(next) });
  const now = new Date();

  return (
    <Page title={t("recap.title")} back="/me">
      <div className="mb-4 flex flex-wrap items-center gap-2">
        <Tabs<"month" | "year">
          value={period.month ? "month" : "year"}
          onChange={(value) =>
            go(
              value === "year"
                ? { period: feed.RecapPeriod.YEAR, year: period.year, month: 0 }
                : {
                    period: feed.RecapPeriod.MONTH,
                    year: period.year,
                    month: period.year === now.getFullYear() ? now.getMonth() + 1 : 12,
                  },
            )
          }
          items={[
            { value: "month", label: t("recap.month") },
            { value: "year", label: t("recap.year") },
          ]}
        />
        <span className="flex-1" />
        <Button
          variant="ghost"
          size="icon-sm"
          aria-label={t("recap.previous")}
          onClick={() => go(shift(period, -1))}
        >
          <ChevronLeft className="size-5" />
        </Button>
        <Button
          variant="ghost"
          size="icon-sm"
          aria-label={t("recap.next")}
          disabled={isCurrent(period)}
          onClick={() => go(shift(period, 1))}
        >
          <ChevronRight className="size-5" />
        </Button>
      </div>

      {recap.isPending ? (
        <Skeleton className="mx-auto aspect-[4/5] w-full max-w-md rounded-[2rem]" />
      ) : recap.isError || !recap.data ? (
        <EmptyState
          emoji="🌧️"
          title={isApiError(recap.error) ? errorMessage(recap.error) : t("recap.failed")}
          action={<Button onClick={() => void recap.refetch()}>{t("timeline.retry")}</Button>}
        />
      ) : recap.data.activityCount === 0 ? (
        <EmptyState
          emoji="🌱"
          title={t("recap.empty", { title: recap.data.title })}
          body={t("recap.emptyBody")}
          action={
            <Link
              to="/notes"
              className="pressable sticker inline-flex h-11 items-center rounded-full bg-brand px-5 font-bold text-on-brand"
            >
              {t("recap.toNotes")}
            </Link>
          }
        />
      ) : (
        <RecapCards key={`${recap.data.startDate}:${recap.data.period}`} recap={recap.data} />
      )}
    </Page>
  );
}

interface Slide {
  key: string;
  /** Background of the card. */
  tint: string;
  render(active: boolean): ReactNode;
}

const HIGHLIGHT_EMOJI: Partial<Record<feed.RecapHighlightType, string>> = {
  [feed.RecapHighlightType.TOTAL_ACTIVITY]: "📚",
  [feed.RecapHighlightType.TOP_KIND]: "🏆",
  [feed.RecapHighlightType.MOST_BUMPED_FRIEND]: "👊",
  [feed.RecapHighlightType.NEW_FRIENDS]: "🤝",
  [feed.RecapHighlightType.PLACES]: "📍",
  [feed.RecapHighlightType.REGIONS]: "🗺️",
  [feed.RecapHighlightType.BUSIEST_DAY]: "🔥",
};
const TINTS = [
  "bg-lime",
  "bg-sun",
  "bg-sky",
  "bg-pink text-white",
  "bg-mint",
  "bg-coral text-white",
];

function buildSlides(recap: feed.Recap, t: TFunction<"notes">): Slide[] {
  const slides: Slide[] = [
    {
      key: "intro",
      tint: "bg-brand text-on-brand",
      render: (active) => (
        <>
          <p className="text-sm font-black opacity-80">
            {recap.startDate.replaceAll("-", "/")} – {recap.endDate.replaceAll("-", "/")}
          </p>
          <h2 className="mt-1 text-[1.6rem] font-black leading-tight">{recap.title}</h2>
          <BigNumber value={recap.activityCount} active={active} caption={t("recap.activities")} />
          <p className="text-lg font-black">
            {t("recap.activeDays", { count: recap.activeDayCount })}
          </p>
          {!recap.complete && (
            <p className="mt-auto rounded-full bg-black/15 px-3 py-1 text-xs font-black">
              {t("recap.soFar")}
            </p>
          )}
        </>
      ),
    },
  ];
  recap.highlights.slice(0, 5).forEach((highlight, index) => {
    slides.push({
      key: `highlight:${index}`,
      tint: TINTS[index % TINTS.length]!,
      render: (active) => (
        <>
          <span className="text-6xl" aria-hidden>
            {HIGHLIGHT_EMOJI[highlight.type] ?? "✨"}
          </span>
          {highlight.value > 0 && <BigNumber value={highlight.value} active={active} />}
          <p className="text-2xl font-black leading-snug">{highlight.text}</p>
        </>
      ),
    });
  });
  if (recap.kinds.length > 0) {
    const most = Math.max(...recap.kinds.map((kind) => kind.count));
    slides.push({
      key: "kinds",
      tint: "bg-surface",
      render: () => (
        <>
          <h3 className="text-xl font-black">{t("recap.kinds")}</h3>
          <ul className="mt-3 flex w-full flex-col gap-2.5">
            {recap.kinds.slice(0, 6).map((kind) => (
              <li key={kind.kind}>
                <p className="flex items-center justify-between text-sm font-black">
                  <span className="truncate">
                    <span aria-hidden>{kindEmoji(kind.kind)}</span> {kind.label}
                  </span>
                  <span className="tabular-nums">{kind.count}</span>
                </p>
                <div className="mt-1 h-3 overflow-hidden rounded-full border-2 border-ink bg-surface-2">
                  <div
                    className="h-full bg-brand"
                    style={{ width: `${Math.max(6, (kind.count / most) * 100)}%` }}
                  />
                </div>
              </li>
            ))}
          </ul>
        </>
      ),
    });
  }
  if (recap.bumpCount > 0 || recap.newFriendCount > 0) {
    slides.push({
      key: "friends",
      tint: "bg-sky",
      render: (active) => (
        <>
          <h3 className="text-xl font-black">{t("recap.friends")}</h3>
          {recap.mostBumpedFriend?.user && (
            <div className="mt-3 flex items-center gap-3 rounded-2xl border-2 border-ink bg-white/70 p-3 text-left">
              <Avatar user={recap.mostBumpedFriend.user} size="lg" outlined />
              <div className="min-w-0">
                <p className="text-xs font-black">{t("recap.mostBumped")}</p>
                <p className="truncate text-lg font-black">
                  <EmojiText text={recap.mostBumpedFriend.user.displayName} />
                </p>
                <p className="text-sm font-bold">
                  {t("recap.bumpTimes", { count: recap.mostBumpedFriend.count })}
                </p>
              </div>
            </div>
          )}
          {recap.bumpCount > 0 && (
            <BigNumber value={recap.bumpCount} active={active} caption={t("recap.bumps")} />
          )}
          {recap.newFriendCount > 0 && (
            <div className="mt-2">
              <p className="font-black">{t("recap.newFriends", { count: recap.newFriendCount })}</p>
              <div className="mt-2 flex flex-wrap justify-center gap-1.5">
                {recap.newFriends.map((friend) => (
                  <Avatar key={friend.id} user={friend} size="sm" outlined />
                ))}
              </div>
            </div>
          )}
        </>
      ),
    });
  }
  if (recap.placeCount > 0 || recap.regionCount > 0) {
    slides.push({
      key: "places",
      tint: "bg-mint",
      render: (active) => (
        <>
          <span className="text-5xl" aria-hidden>
            🗺️
          </span>
          <BigNumber
            value={recap.placeCount || recap.areaCount}
            active={active}
            caption={recap.placeCount ? t("recap.places") : t("recap.areas")}
          />
          <div className="mt-1 flex flex-wrap justify-center gap-1.5">
            {[...recap.regionNames, ...recap.placeNames].slice(0, 10).map((name) => (
              <span
                key={name}
                className="rounded-full border-2 border-ink bg-white/70 px-2.5 py-0.5 text-sm font-black"
              >
                {name}
              </span>
            ))}
          </div>
        </>
      ),
    });
  }
  if (recap.days.length > 0) {
    slides.push({ key: "days", tint: "bg-surface", render: () => <Heat recap={recap} /> });
  }
  slides.push({ key: "share", tint: "bg-sun", render: () => <ShareSlide recap={recap} /> });
  return slides;
}

function BigNumber({
  value,
  active,
  caption,
}: {
  value: number;
  active: boolean;
  caption?: string;
}) {
  const shown = useCountUp(value, active);
  return (
    <p className="my-2">
      <span className="block text-7xl font-black leading-none tabular-nums">
        {shown.toLocaleString()}
      </span>
      {caption && <span className="mt-1 block text-lg font-black">{caption}</span>}
    </p>
  );
}

/** One square per day of the period, darker the more happened: where the busy days were. */
function Heat({ recap }: { recap: feed.Recap }) {
  const { t, i18n } = useTranslation("notes");
  const counts = new Map(recap.days.map((day) => [day.date, day.count]));
  const most = Math.max(1, ...recap.days.map((day) => day.count));
  const isYear = recap.period === feed.RecapPeriod.YEAR;
  const cells: { date: string; count: number }[] = [];
  const end = new Date(`${recap.endDate}T00:00:00`);
  for (
    const day = new Date(`${recap.startDate}T00:00:00`);
    day <= end;
    day.setDate(day.getDate() + 1)
  ) {
    const date = `${day.getFullYear()}-${String(day.getMonth() + 1).padStart(2, "0")}-${String(day.getDate()).padStart(2, "0")}`;
    cells.push({ date, count: counts.get(date) ?? 0 });
  }
  // A month starts on its weekday, like a calendar; a year is a plain strip.
  const offset = isYear ? 0 : new Date(`${recap.startDate}T00:00:00`).getDay();
  const busiest = recap.busiestDay;

  return (
    <>
      <h3 className="text-xl font-black">{t("recap.calendar")}</h3>
      <div
        className={cn(
          "mt-3 grid w-full gap-1",
          isYear ? "grid-cols-[repeat(26,minmax(0,1fr))]" : "grid-cols-7",
        )}
        role="img"
        aria-label={t("recap.calendarLabel", { count: recap.activeDayCount })}
      >
        {Array.from({ length: offset }, (_, index) => (
          <span key={`pad:${index}`} />
        ))}
        {cells.map((cell) => (
          <span
            key={cell.date}
            title={`${cell.date}: ${cell.count}`}
            className={cn(
              "aspect-square rounded-[0.3rem]",
              isYear ? "" : "grid place-items-center rounded-lg text-[11px] font-black",
              cell.count === 0 ? "bg-surface-2 text-ink-faint" : "bg-brand text-on-brand",
            )}
            style={cell.count > 0 ? { opacity: 0.35 + 0.65 * (cell.count / most) } : undefined}
          >
            {!isYear && Number(cell.date.slice(-2))}
          </span>
        ))}
      </div>
      {busiest && (
        <p className="mt-3 text-sm font-black">
          {t("recap.busiest", {
            date: new Date(`${busiest.date}T00:00:00`).toLocaleDateString(i18n.language, {
              month: "long",
              day: "numeric",
            }),
            count: busiest.count,
          })}
        </p>
      )}
    </>
  );
}

/** The last card: send the recap out as text, or save it as a picture. */
function ShareSlide({ recap }: { recap: feed.Recap }) {
  const { t } = useTranslation("notes");
  const [rendering, setRendering] = useState(false);
  const text = [
    recap.title,
    ...recap.highlights.slice(0, 3).map((highlight) => highlight.text),
    "#Tokuly",
  ].join("\n");

  const shareText = async () => {
    if (typeof navigator.share === "function") {
      await navigator.share({ title: recap.title, text }).catch(() => {});
    } else if (await copyText(text)) {
      toast.success(t("recap.copied"));
    }
  };

  const savePicture = async () => {
    setRendering(true);
    try {
      const blob = await renderRecapImage(recap, {
        activities: t("recap.activities"),
        activeDays: t("recap.activeDaysShort"),
        bumps: t("recap.bumps"),
        newFriends: t("recap.newFriendsShort"),
      });
      const file = new File([blob], `tokuly-recap-${recap.startDate.slice(0, 7)}.png`, {
        type: "image/png",
      });
      if (typeof navigator.canShare === "function" && navigator.canShare({ files: [file] })) {
        await navigator.share({ files: [file], title: recap.title }).catch(() => {});
      } else {
        const url = URL.createObjectURL(file);
        const link = document.createElement("a");
        link.href = url;
        link.download = file.name;
        link.click();
        setTimeout(() => URL.revokeObjectURL(url), 10_000);
        toast.success(t("recap.saved"));
      }
    } catch (error) {
      toast.error(errorMessage(error));
    } finally {
      setRendering(false);
    }
  };

  return (
    <>
      <span className="animate-float text-6xl" aria-hidden>
        🎉
      </span>
      <h3 className="mt-2 text-2xl font-black leading-snug">{t("recap.shareTitle")}</h3>
      <p className="mt-1 text-sm font-bold opacity-80">{t("recap.shareBody")}</p>
      <div className="mt-4 flex w-full flex-col gap-2">
        <Button
          variant="secondary"
          block
          icon={<Share2 className="size-4" />}
          onClick={() => void shareText()}
        >
          {t("recap.share")}
        </Button>
        <Button
          variant="primary"
          block
          icon={<Download className="size-4" />}
          loading={rendering}
          onClick={() => void savePicture()}
        >
          {t("recap.savePicture")}
        </Button>
      </div>
    </>
  );
}

function RecapCards({ recap }: { recap: feed.Recap }) {
  const { t } = useTranslation("notes");
  const strip = useRef<HTMLDivElement>(null);
  const [index, setIndex] = useState(0);
  const slides = buildSlides(recap, t);

  const go = (next: number) => {
    const element = strip.current;
    if (!element) return;
    const clamped = Math.min(slides.length - 1, Math.max(0, next));
    element.scrollTo({
      left: clamped * element.clientWidth,
      behavior: document.documentElement.dataset.dopa === "0" ? "auto" : "smooth",
    });
  };

  return (
    <section aria-label={recap.title} aria-roledescription="carousel" data-testid="recap">
      <div className="relative mx-auto max-w-md">
        {/* biome-ignore lint/a11y/useSemanticElements: a strip of cards to swipe through, not a form group. */}
        <div
          ref={strip}
          role="group"
          aria-label={t("recap.cardOf", { index: index + 1, count: slides.length })}
          className="scrollbar-none flex snap-x snap-mandatory overflow-x-auto rounded-[2rem]"
          onScroll={(event) => {
            const element = event.currentTarget;
            const current = Math.round(element.scrollLeft / Math.max(1, element.clientWidth));
            if (current !== index) setIndex(current);
          }}
        >
          {slides.map((slide, position) => (
            <div
              key={slide.key}
              className="w-full shrink-0 snap-center p-1.5 pb-2.5"
              aria-hidden={position !== index}
            >
              <div
                className={cn(
                  "sticker-lg flex aspect-[4/5] flex-col items-center justify-center gap-1 overflow-hidden rounded-[2rem] p-6 text-center text-[#17122b]",
                  slide.tint,
                  slide.tint === "bg-surface" && "text-ink",
                )}
              >
                {slide.render(position === index)}
              </div>
            </div>
          ))}
        </div>
        <Button
          variant="secondary"
          size="icon-sm"
          aria-label={t("recap.previousCard")}
          disabled={index === 0}
          onClick={() => go(index - 1)}
          className="absolute top-1/2 -left-4 hidden -translate-y-1/2 disabled:invisible sm:inline-flex"
        >
          <ChevronLeft className="size-5" />
        </Button>
        <Button
          variant="secondary"
          size="icon-sm"
          aria-label={t("recap.nextCard")}
          disabled={index === slides.length - 1}
          onClick={() => go(index + 1)}
          className="absolute top-1/2 -right-4 hidden -translate-y-1/2 disabled:invisible sm:inline-flex"
        >
          <ChevronRight className="size-5" />
        </Button>
      </div>
      <div className="mt-3 flex justify-center gap-1.5">
        {slides.map((slide, position) => (
          <button
            key={slide.key}
            type="button"
            aria-label={t("recap.cardOf", { index: position + 1, count: slides.length })}
            aria-current={position === index}
            onClick={() => go(position)}
            className={cn(
              "h-2.5 rounded-full border-2 border-ink transition-all",
              position === index ? "w-6 bg-brand" : "w-2.5 bg-surface",
            )}
          />
        ))}
      </div>
    </section>
  );
}
