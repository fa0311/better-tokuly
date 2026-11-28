import { Link } from "@tanstack/react-router";
import type { spaces } from "@tokuly/api";
import { nameOf } from "@tokuly/core/spaces";
import { Mic } from "lucide-react";
import { Avatar } from "#/components/ui/avatar";
import { EmojiText } from "#/features/emoji";
import { cn } from "#/lib/cn";
import { useTranslation } from "#/lib/use-translation";
import { useFriendsInRooms } from "./api";

/** Where a friend's room or hangout is in the app. */
function VenueLink({
  friend,
  className,
  children,
  label,
}: {
  friend: spaces.FriendInRoom;
  className?: string;
  children: React.ReactNode;
  label?: string;
}) {
  return friend.hangoutId ? (
    <Link
      to="/spaces/hangouts/$hangoutId"
      params={{ hangoutId: friend.hangoutId }}
      className={className}
      aria-label={label}
    >
      {children}
    </Link>
  ) : (
    <Link
      to="/spaces/$spaceId"
      params={{ spaceId: friend.spaceId }}
      className={className}
      aria-label={label}
    >
      {children}
    </Link>
  );
}

export interface HangoutPresenceProps {
  /** The friend in question. */
  userId: string;
  className?: string;
}

/**
 * Says where a friend is right now, for a row of a friend list or a map
 * marker: "🛋️ いつメン" for a hangout, "🎙 title" for a room, each leading
 * there. Renders nothing while the friend is in no room the viewer could
 * enter (or keeps their online status to themselves).
 */
export function HangoutPresence({ userId, className }: HangoutPresenceProps) {
  const { t } = useTranslation("spaces");
  const friend = useFriendsInRooms().data?.find((entry) => entry.user?.id === userId);
  if (!friend) return null;
  return (
    <VenueLink
      friend={friend}
      label={t("presence.label", { name: nameOf(friend.user), title: friend.title })}
      className={cn(
        "inline-flex h-6 max-w-full items-center gap-1 rounded-full bg-lime px-2.5 text-xs font-black text-[#17122b]",
        className,
      )}
    >
      {friend.hangoutId ? (
        <span aria-hidden>{friend.emoji || "🛋️"}</span>
      ) : (
        <Mic className="size-3.5 shrink-0" />
      )}
      <span className="truncate" data-testid="hangout-presence">
        <EmojiText text={friend.title} />
      </span>
    </VenueLink>
  );
}

/**
 * Every friend who is in a room or hangout right now, as a row of faces to
 * drop in on. Renders nothing while nobody is.
 */
export function FriendsInRoomsStrip({ className }: { className?: string }) {
  const { t } = useTranslation("spaces");
  const friends = useFriendsInRooms().data ?? [];
  if (friends.length === 0) return null;
  return (
    <section className={className} aria-label={t("presence.title")} data-testid="friends-in-rooms">
      <h2 className="mb-2 text-lg font-black">{t("presence.title")}</h2>
      <ul className="scrollbar-none -mx-4 flex gap-3 overflow-x-auto px-4 pb-1">
        {friends.map((friend) => (
          <li key={`${friend.user?.id}:${friend.spaceId}${friend.hangoutId}`} className="shrink-0">
            <VenueLink
              friend={friend}
              className="flex w-20 flex-col items-center gap-1 text-center"
              label={t("presence.label", { name: nameOf(friend.user), title: friend.title })}
            >
              <span className="relative">
                <Avatar user={friend.user} size="lg" outlined className="rounded-[1.25rem]" />
                <span className="absolute -right-1.5 -bottom-1.5 grid size-7 place-items-center rounded-full border-2 border-ink bg-lime text-sm leading-none text-[#17122b]">
                  {friend.hangoutId ? friend.emoji || "🛋️" : <Mic className="size-3.5" />}
                </span>
              </span>
              <span className="w-full truncate text-xs font-bold">
                <EmojiText text={nameOf(friend.user)} />
              </span>
              <span className="w-full truncate text-[11px] font-bold text-ink-faint">
                <EmojiText text={friend.title} />
              </span>
            </VenueLink>
          </li>
        ))}
      </ul>
    </section>
  );
}
