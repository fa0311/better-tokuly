import type { notes } from "@tokuly/api";
import { toMillis } from "@tokuly/api";
import { ChevronLeft, ChevronRight, Eye, Images, Plus, Sparkles, X } from "lucide-react";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { Button } from "#/components/ui/button";
import { Chip } from "#/components/ui/card";
import { LoadMore } from "#/components/ui/load-more";
import { EmptyState, Page, Skeleton } from "#/components/ui/page";
import { imageUrl } from "#/features/notes";
import { cn } from "#/lib/cn";
import { useMe } from "#/stores/session";
import { storyCover, useArchivedStories, useStoryCalendar } from "./api";
import { CreateStoryDialog } from "./create";
import { openStoryViewer } from "./viewer-store";

const pad = (value: number) => String(value).padStart(2, "0");

/**
 * The viewer's own stories, kept after their day is over: a calendar that
 * shows the picture of each day with stories, and the stories of the day
 * picked (or all of them, newest first).
 */
export function StoryArchiveScreen() {
  const { t, i18n } = useTranslation("stories");
  const me = useMe();
  const today = new Date();
  const [month, setMonth] = useState({ year: today.getFullYear(), month: today.getMonth() + 1 });
  const [date, setDate] = useState("");
  const [creating, setCreating] = useState(false);
  const calendar = useStoryCalendar(month.year, month.month);
  const archive = useArchivedStories(date);
  const stories = archive.data?.pages.flatMap((page) => page.stories) ?? [];

  const days = new Map(calendar.data?.map((day) => [day.date, day]));
  const first = new Date(month.year, month.month - 1, 1);
  const dayCount = new Date(month.year, month.month, 0).getDate();
  const isCurrentMonth = month.year === today.getFullYear() && month.month === today.getMonth() + 1;
  const move = (delta: number) => {
    const index = month.year * 12 + (month.month - 1) + delta;
    setMonth({ year: Math.floor(index / 12), month: (index % 12) + 1 });
    setDate("");
  };
  // Weekday names starting on Sunday, in the viewer's language.
  const weekdays = Array.from({ length: 7 }, (_, index) => new Date(2023, 0, 1 + index).toLocaleDateString(i18n.language, { weekday: "narrow" }));
  const play = (story: notes.Story) => me?.user && openStoryViewer([{ user: me.user, stories }], { storyId: story.id });

  return (
    <Page
      title={t("archive.title")}
      back="/me"
      actions={
        <Button size="sm" variant="lime" icon={<Plus className="size-4" strokeWidth={3} />} onClick={() => setCreating(true)}>
          {t("archive.add")}
        </Button>
      }
    >
      <section className="sticker rounded-card bg-surface p-4" aria-label={t("archive.calendar")} data-testid="story-calendar">
        <div className="flex items-center gap-2">
          <h2 className="flex-1 text-lg font-black">{first.toLocaleDateString(i18n.language, { year: "numeric", month: "long" })}</h2>
          <Button variant="ghost" size="icon-sm" aria-label={t("archive.previousMonth")} onClick={() => move(-1)}>
            <ChevronLeft className="size-5" />
          </Button>
          <Button variant="ghost" size="icon-sm" aria-label={t("archive.nextMonth")} disabled={isCurrentMonth} onClick={() => move(1)}>
            <ChevronRight className="size-5" />
          </Button>
        </div>
        <div className="mt-3 grid grid-cols-7 gap-1 text-center">
          {weekdays.map((weekday, index) => (
            <span key={index} className="pb-1 text-xs font-black text-ink-faint">
              {weekday}
            </span>
          ))}
          {Array.from({ length: first.getDay() }, (_, index) => (
            <span key={`pad:${index}`} />
          ))}
          {Array.from({ length: dayCount }, (_, index) => {
            const key = `${month.year}-${pad(month.month)}-${pad(index + 1)}`;
            const day = days.get(key);
            const selected = date === key;
            if (!day) {
              return (
                <span key={key} className="grid aspect-square place-items-center rounded-xl text-sm font-bold text-ink-faint">
                  {index + 1}
                </span>
              );
            }
            return (
              <button
                key={key}
                type="button"
                aria-pressed={selected}
                aria-label={t("archive.day", { date: new Date(month.year, month.month - 1, index + 1).toLocaleDateString(i18n.language, { month: "long", day: "numeric" }), count: day.storyCount })}
                onClick={() => setDate(selected ? "" : key)}
                className={cn("relative aspect-square overflow-hidden rounded-xl border-2 bg-surface-3 text-sm font-black text-white", selected ? "border-ink shadow-pop-sm" : "border-transparent")}
                data-testid="story-day"
              >
                {day.cover?.url && <img src={imageUrl(day.cover, 56)} alt="" loading="lazy" className="absolute inset-0 size-full object-cover" />}
                <span className="absolute inset-0 grid place-items-center bg-black/30">{index + 1}</span>
                {day.storyCount > 1 && <span className="absolute top-0.5 right-0.5 rounded-full bg-pink px-1 text-[10px] leading-4">{day.storyCount}</span>}
              </button>
            );
          })}
        </div>
        {calendar.isPending && <Skeleton className="mt-3 h-4 w-1/2 rounded-full" />}
      </section>

      <div className="mt-5 mb-2.5 flex items-center gap-2">
        <h2 className="flex-1 text-lg font-black">
          {date ? new Date(`${date}T00:00:00`).toLocaleDateString(i18n.language, { month: "long", day: "numeric", weekday: "short" }) : t("archive.all")}
        </h2>
        {date && (
          <Button variant="ghost" size="sm" icon={<X className="size-4" />} onClick={() => setDate("")}>
            {t("archive.showAll")}
          </Button>
        )}
      </div>

      {archive.isPending ? (
        <div className="grid grid-cols-2 gap-3 sm:grid-cols-3">
          <Skeleton className="aspect-[3/4]" />
          <Skeleton className="aspect-[3/4]" />
          <Skeleton className="aspect-[3/4]" />
        </div>
      ) : stories.length === 0 ? (
        <EmptyState
          emoji="📷"
          title={t("archive.empty")}
          body={t("archive.emptyBody")}
          action={
            <Button variant="lime" onClick={() => setCreating(true)}>
              {t("archive.first")}
            </Button>
          }
        />
      ) : (
        <>
          <ul className="grid grid-cols-2 gap-3 sm:grid-cols-3" data-testid="archived-stories">
            {stories.map((story) => (
              <li key={story.id}>
                <button type="button" onClick={() => play(story)} className="pressable sticker relative block aspect-[3/4] w-full overflow-hidden rounded-card bg-surface-3 text-left text-white" data-testid="archived-story">
                  <img src={imageUrl(storyCover(story), 240)} alt="" loading="lazy" className="absolute inset-0 size-full object-cover" />
                  <span className="absolute inset-x-0 top-0 flex flex-wrap gap-1 p-2">
                    {!story.expired && <Chip className="border-2 border-ink bg-lime text-[11px] font-black text-[#17122b]">{t("archive.live")}</Chip>}
                    {story.highlighted && (
                      <Chip className="border-2 border-ink bg-sun text-[11px] font-black text-[#17122b]">
                        <Sparkles className="size-3" />
                        {t("archive.highlighted")}
                      </Chip>
                    )}
                  </span>
                  <span className="absolute inset-x-0 bottom-0 bg-gradient-to-t from-black/80 to-transparent px-2.5 pt-8 pb-2">
                    {story.title && <span className="line-clamp-2 text-sm font-black leading-snug">{story.title}</span>}
                    <span className="mt-0.5 flex items-center gap-2 text-xs font-bold text-white/85">
                      <span>{new Date(toMillis(story.createdAt)).toLocaleDateString(i18n.language, { month: "short", day: "numeric" })}</span>
                      <span className="inline-flex items-center gap-0.5">
                        <Eye className="size-3.5" />
                        {story.viewCount}
                      </span>
                      {story.items.length > 1 && (
                        <span className="inline-flex items-center gap-0.5">
                          <Images className="size-3.5" />
                          {story.items.length}
                        </span>
                      )}
                    </span>
                  </span>
                </button>
              </li>
            ))}
          </ul>
          <LoadMore query={archive} />
        </>
      )}
      {creating && <CreateStoryDialog onClose={() => setCreating(false)} />}
    </Page>
  );
}
