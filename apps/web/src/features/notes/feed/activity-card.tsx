import { Link } from "@tanstack/react-router";
import { common, feed, toMillis } from "@tokuly/api";
import { ChevronRight, Globe, Lock, UsersRound } from "lucide-react";
import type { ReactNode } from "react";
import type { Subject } from "#/features/engagement";
import { useTranslation } from "react-i18next";
import { Avatar } from "#/components/ui/avatar";
import { Skeleton } from "#/components/ui/page";
import { BookmarkButton, ReactionBar, ShareButton, useEngagement, useOpenPath } from "#/features/engagement";
import { cn } from "#/lib/cn";
import { useIsDeleted, useNote } from "../api";
import { NoteCard } from "../note-card";
import { noteAge, useClock } from "../time";

/** The emoji that leads an activity, by what kind of thing happened. */
const KIND_EMOJI: [prefix: string, emoji: string][] = [
  ["social.bumped", "👊"],
  ["social.friend_added", "🤝"],
  ["account.joined", "🎉"],
  ["geo.new_region", "🗺️"],
  ["note", "📝"],
  ["story", "📸"],
  ["video", "🎬"],
  ["short", "⚡"],
  ["live", "🔴"],
  ["clip", "✂️"],
  ["playlist", "🎶"],
  ["channel", "📺"],
  ["stamp", "🧸"],
  ["chat", "💬"],
  ["wallet", "🪙"],
  ["geo", "📍"],
  ["social", "💜"],
  ["account", "🙋"],
];

export function kindEmoji(kind: string): string {
  return KIND_EMOJI.find(([prefix]) => kind === prefix || kind.startsWith(`${prefix}.`))?.[1] ?? "✨";
}

/** Extra data a producing domain attached to an activity; `{}` when there is none or it does not parse. */
export function activityData(activity: feed.Activity): Record<string, unknown> {
  if (!activity.dataJson) return {};
  try {
    const parsed: unknown = JSON.parse(activity.dataJson);
    return parsed && typeof parsed === "object" ? (parsed as Record<string, unknown>) : {};
  } catch {
    return {};
  }
}

/** Who did it, what kind of thing it was, and when: the line above every feed card. */
function CardHeader({ activity, trailing }: { activity: feed.Activity; trailing?: ReactNode }) {
  const { i18n } = useTranslation();
  useClock();
  const actor = activity.actor;
  return (
    <div className="flex items-center gap-2.5">
      {actor && (
        <Link to="/u/$handle" params={{ handle: actor.handle }} aria-label={actor.displayName}>
          <Avatar user={actor} size="sm" />
        </Link>
      )}
      <div className="min-w-0 flex-1">
        <p className="flex items-baseline gap-1.5">
          {actor && (
            <Link to="/u/$handle" params={{ handle: actor.handle }} className="truncate text-sm font-black hover:underline">
              {actor.displayName}
            </Link>
          )}
          <span className="shrink-0 text-xs text-ink-faint">{noteAge(toMillis(activity.occurredAt), i18n.language)}</span>
        </p>
        <p className="truncate text-xs font-bold text-ink-faint">
          <span aria-hidden>{kindEmoji(activity.kind)}</span> {activity.kindLabel}
        </p>
      </div>
      {trailing}
    </div>
  );
}

/** "+3" for the rest of a burst the card stands for. */
function BurstCount({ card }: { card: feed.ActivityCard }) {
  const { t } = useTranslation("notes");
  if (card.count <= 1) return null;
  return <span className="shrink-0 rounded-full bg-brand-soft px-2 py-0.5 text-xs font-black text-brand">{t("feed.more", { count: card.count - 1 })}</span>;
}

/**
 * One card of the home feed or the friends' feed. Notes are drawn as notes;
 * stories and videos as a picture card; everything else as a compact row
 * that says what happened and leads to it.
 */
export function ActivityCardView({ card }: { card: feed.ActivityCard }) {
  const activity = card.activity;
  if (!activity) return null;
  switch (card.type) {
    case feed.ActivityCardType.NOTE:
      return <FeedNote card={card} activity={activity} />;
    case feed.ActivityCardType.STORY:
    case feed.ActivityCardType.VIDEO:
    case feed.ActivityCardType.SHORT:
    case feed.ActivityCardType.LIVE:
    case feed.ActivityCardType.CLIP:
    case feed.ActivityCardType.PLAYLIST:
      return <PictureCard card={card} activity={activity} />;
    default:
      return <CompactCard card={card} activity={activity} />;
  }
}

/** A friend's note in the feed: the note itself, loaded with the other notes of the page in one request. */
function FeedNote({ card, activity }: { card: feed.ActivityCard; activity: feed.Activity }) {
  const { t } = useTranslation("notes");
  const noteId = activity.subject?.type === common.SubjectType.NOTE ? activity.subject.id : "";
  const note = useNote(noteId);
  const deleted = useIsDeleted(noteId);
  if (!noteId) return <CompactCard card={card} activity={activity} />;
  if (note.isPending) return <Skeleton className="h-32 rounded-[1.25rem]" />;
  // A note that is gone (or no longer for the viewer's eyes) simply leaves the feed.
  if (!note.data || deleted) return null;
  return (
    <div>
      <NoteCard note={note.data} />
      {card.count > 1 && activity.actor && (
        <Link to="/u/$handle" params={{ handle: activity.actor.handle }} className="mt-1 ml-4 inline-block text-xs font-black text-brand hover:underline">
          {t("feed.moreNotes", { name: activity.actor.displayName, count: card.count - 1 })}
        </Link>
      )}
    </div>
  );
}

/** A story, a video, a short, a live, a clip or a playlist: the picture first, then what it is. */
function PictureCard({ card, activity }: { card: feed.ActivityCard; activity: feed.Activity }) {
  const open = useOpenPath();
  const data = activityData(activity);
  const title = typeof data.title === "string" && data.title ? data.title : activity.summary;
  const isStory = card.type === feed.ActivityCardType.STORY;
  const thumbnails = card.more.filter((item) => item.imageUrl).slice(0, 4);

  return (
    <article className="rounded-[1.25rem] border-2 border-line bg-surface p-3.5" data-testid="feed-card" data-card-type={feed.ActivityCardType[card.type]}>
      <CardHeader activity={activity} trailing={<BurstCount card={card} />} />
      <button
        type="button"
        disabled={!activity.path}
        onClick={() => open(activity.path)}
        className={cn("mt-3 block w-full overflow-hidden rounded-2xl border-2 border-ink text-left", activity.path && "pressable shadow-pop-sm")}
      >
        {activity.imageUrl ? (
          <span className={cn("relative block bg-surface-3", isStory ? "aspect-[4/5] max-h-96 w-full" : "aspect-video")}>
            <img src={activity.imageUrl} alt="" loading="lazy" className="absolute inset-0 size-full object-cover" />
            <span className="absolute inset-x-0 bottom-0 bg-gradient-to-t from-black/75 to-transparent px-3 pt-8 pb-2.5 text-[15px] font-black leading-snug text-white">
              <span className="line-clamp-2">{title}</span>
            </span>
          </span>
        ) : (
          <span className="flex items-center gap-3 bg-surface-2 px-3 py-4">
            <span aria-hidden className="text-3xl">
              {kindEmoji(activity.kind)}
            </span>
            <span className="line-clamp-2 min-w-0 flex-1 font-black leading-snug">{title}</span>
            {activity.path && <ChevronRight className="size-5 shrink-0 text-ink-faint" />}
          </span>
        )}
      </button>
      {thumbnails.length > 0 && (
        <div className="mt-2 flex gap-1.5">
          {thumbnails.map((item) => (
            <button key={item.id} type="button" onClick={() => open(item.path)} className="size-14 overflow-hidden rounded-xl border-2 border-ink bg-surface-3">
              <img src={item.imageUrl} alt={item.summary} loading="lazy" className="size-full object-cover" />
            </button>
          ))}
        </div>
      )}
      {!isStory && activity.subject && <SubjectActions subject={{ type: activity.subject.type, id: activity.subject.id }} title={title} />}
    </article>
  );
}

/**
 * Like, react, save and share right from the feed, for content that takes
 * it (a video, a short, a clip…). Nothing is drawn for content that does
 * not, or whose domain is not there yet.
 */
function SubjectActions({ subject, title }: { subject: Subject; title: string }) {
  const { data } = useEngagement(subject);
  if (!data) return null;
  return (
    <div className="mt-2 flex items-center gap-1">
      {data.interactionsEnabled && <ReactionBar subject={subject} className="min-w-0 flex-1" />}
      <span className="flex-1" />
      <BookmarkButton subject={subject} />
      <ShareButton subject={subject} title={title} />
    </div>
  );
}

/** Everything without a layout of its own: one line saying what happened, the people involved, a way in. */
function CompactCard({ card, activity }: { card: feed.ActivityCard; activity: feed.Activity }) {
  const open = useOpenPath();
  return (
    <article className="rounded-[1.25rem] border-2 border-line bg-surface p-3.5" data-testid="feed-card" data-card-type={feed.ActivityCardType[card.type]}>
      <CardHeader activity={activity} trailing={<BurstCount card={card} />} />
      <button
        type="button"
        disabled={!activity.path}
        onClick={() => open(activity.path)}
        className={cn("mt-2.5 flex w-full items-center gap-3 rounded-2xl bg-surface-2 px-3 py-2.5 text-left", activity.path && "hover:bg-surface-3")}
      >
        {activity.imageUrl ? (
          <img src={activity.imageUrl} alt="" loading="lazy" className="size-12 shrink-0 rounded-xl border-2 border-ink object-cover" />
        ) : (
          <span aria-hidden className="grid size-12 shrink-0 place-items-center rounded-xl border-2 border-ink bg-surface text-2xl">
            {kindEmoji(activity.kind)}
          </span>
        )}
        <span className="line-clamp-2 min-w-0 flex-1 text-[15px] font-bold leading-snug">{activity.summary}</span>
        {activity.withUsers.length > 0 && (
          <span className="flex shrink-0 -space-x-2">
            {activity.withUsers.slice(0, 3).map((user) => (
              <Avatar key={user.id} user={user} size="xs" className="rounded-[0.5rem] ring-2 ring-surface-2" />
            ))}
          </span>
        )}
        {activity.path && <ChevronRight className="size-5 shrink-0 text-ink-faint" />}
      </button>
    </article>
  );
}

const AUDIENCE: Record<feed.ActivityAudience, { key: string; icon: typeof Globe }> = {
  [feed.ActivityAudience.UNSPECIFIED]: { key: "private", icon: Lock },
  [feed.ActivityAudience.PRIVATE]: { key: "private", icon: Lock },
  [feed.ActivityAudience.FRIENDS]: { key: "friends", icon: UsersRound },
  [feed.ActivityAudience.PUBLIC]: { key: "public", icon: Globe },
};

/**
 * One entry of an activity log (the viewer's own, or a profile's): the time,
 * what happened, who may see it, and a way to the thing it is about.
 */
export function ActivityRow({ activity, showAudience, trailing }: { activity: feed.Activity; /** Say who may see the entry (the viewer's own log). */ showAudience?: boolean; trailing?: ReactNode }) {
  const { t, i18n } = useTranslation("notes");
  const open = useOpenPath();
  const when = new Date(toMillis(activity.occurredAt));
  const audience = AUDIENCE[activity.audience] ?? AUDIENCE[feed.ActivityAudience.PRIVATE];
  const hidden = activity.hiddenFromOthers;
  const AudienceIcon = hidden ? Lock : audience.icon;
  const audienceLabel = hidden ? t("activity.audience.hidden") : t(`activity.audience.${audience.key}`);

  return (
    <div className="flex items-center gap-1 rounded-2xl bg-surface pr-1" data-testid="activity">
      <button type="button" disabled={!activity.path} onClick={() => open(activity.path)} className={cn("flex min-w-0 flex-1 items-center gap-3 rounded-2xl px-3 py-2.5 text-left", activity.path && "hover:bg-surface-2")}>
        {activity.imageUrl ? (
          <img src={activity.imageUrl} alt="" loading="lazy" className="size-11 shrink-0 rounded-xl border-2 border-ink object-cover" />
        ) : (
          <span aria-hidden className="grid size-11 shrink-0 place-items-center rounded-xl bg-surface-2 text-xl">
            {kindEmoji(activity.kind)}
          </span>
        )}
        <span className="min-w-0 flex-1">
          <span className="line-clamp-2 text-[15px] font-bold leading-snug">{activity.summary}</span>
          <span className="mt-0.5 flex items-center gap-1.5 text-xs font-bold text-ink-faint">
            <time dateTime={when.toISOString()}>{when.toLocaleTimeString(i18n.language, { hour: "2-digit", minute: "2-digit" })}</time>
            <span aria-hidden>·</span>
            <span className="truncate">{activity.kindLabel}</span>
            {showAudience && (
              <span className={cn("inline-flex shrink-0 items-center gap-0.5", hidden && "text-coral")} title={audienceLabel}>
                <AudienceIcon className="size-3.5" aria-hidden />
                <span className="sr-only sm:not-sr-only">{audienceLabel}</span>
              </span>
            )}
          </span>
        </span>
      </button>
      {trailing}
    </div>
  );
}
