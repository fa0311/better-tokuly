import { Link } from "@tanstack/react-router";
import { spaces, toMillis } from "@tokuly/api";
import { Bell, Headphones } from "lucide-react";
import type { ReactNode } from "react";
import { useTranslation } from "react-i18next";
import { Avatar } from "#/components/ui/avatar";
import { EmojiText } from "#/features/emoji";
import { useAccountTimeZone, useAccountToday } from "#/lib/account-time";
import { cn } from "#/lib/cn";
import { AudienceLabel, Faces, LiveBadge, PersonName, TagList } from "./parts";
import { formatWhen } from "./when";

/** A room in a list: what it is about, who is talking, how many listen. Leads to the room. */
export function SpaceCard({
  space,
  action,
  className,
}: {
  space: spaces.Space;
  /** A control under the card's text (remind me, start). */
  action?: ReactNode;
  className?: string;
}) {
  const { t, i18n } = useTranslation("spaces");
  const zone = useAccountTimeZone();
  const today = useAccountToday();
  const live = space.state === spaces.SpaceState.LIVE;
  const upcoming = space.state === spaces.SpaceState.SCHEDULED;
  const others = space.stage.flatMap((entry) =>
    entry.user && entry.user.id !== space.host?.id ? [entry.user] : [],
  );
  return (
    <article
      className={cn("sticker relative rounded-card bg-surface p-4", className)}
      data-testid="space-card"
    >
      <div className="flex items-center gap-2">
        {live ? (
          <LiveBadge />
        ) : upcoming ? (
          <span className="inline-flex h-6 items-center rounded-full bg-sun px-2.5 text-[11px] font-black text-[#17122b]">
            {formatWhen(toMillis(space.scheduledAt), zone, today, i18n.language)}
          </span>
        ) : (
          <span className="inline-flex h-6 items-center rounded-full bg-surface-2 px-2.5 text-[11px] font-black text-ink-soft">
            {t(space.state === spaces.SpaceState.CANCELLED ? "state.cancelled" : "state.ended")}
          </span>
        )}
        <AudienceLabel audience={space.audience} className="text-ink-faint" />
        {live && (
          <span className="ml-auto inline-flex items-center gap-1 text-xs font-black text-ink-soft">
            <Headphones className="size-3.5" />
            {space.listenerCount.toLocaleString()}
          </span>
        )}
        {upcoming && space.reminderCount > 0 && (
          <span className="ml-auto inline-flex items-center gap-1 text-xs font-black text-ink-soft">
            <Bell className="size-3.5" />
            {space.reminderCount.toLocaleString()}
          </span>
        )}
      </div>
      <h3 className="mt-2.5 line-clamp-2 text-lg leading-snug font-black">
        {/* The whole card is the link; controls inside stay above it. */}
        <Link
          to="/spaces/$spaceId"
          params={{ spaceId: space.id }}
          className="after:absolute after:inset-0 after:rounded-card"
        >
          <EmojiText text={space.title} />
        </Link>
      </h3>
      <TagList tags={space.tags} className="mt-2" />
      <div className="mt-3 flex items-center gap-2">
        <Avatar user={space.host} size="sm" outlined />
        <span className="min-w-0 flex-1 truncate text-sm font-bold">
          <PersonName user={space.host} />
        </span>
        <Faces users={others.slice(0, 3)} more={Math.max(0, others.length - 3)} size="xs" />
      </div>
      {space.friendsInside.length > 0 && (
        <p className="mt-3 flex items-center gap-2 rounded-2xl bg-brand-soft px-3 py-2 text-xs font-bold">
          <Faces users={space.friendsInside.slice(0, 3)} size="xs" />
          <span className="min-w-0 truncate">
            {t(space.friendsInside.length > 1 ? "card.friendsInside" : "card.friendInside", {
              name: space.friendsInside[0]?.displayName || space.friendsInside[0]?.handle,
              count: space.friendsInside.length,
            })}
          </span>
        </p>
      )}
      {action && <div className="relative z-10 mt-3 flex flex-wrap gap-2">{action}</div>}
    </article>
  );
}
