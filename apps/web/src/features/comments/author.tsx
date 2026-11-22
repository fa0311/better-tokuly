import { Link } from "@tanstack/react-router";
import type { common } from "@tokuly/api";
import { BadgeCheck } from "lucide-react";
import type { ReactNode } from "react";
import { Avatar, type AvatarProps } from "#/components/ui/avatar";
import { cn } from "#/lib/cn";

/** A channel shaped like the user `Avatar` draws. */
export function channelFace(channel: common.ChannelSummary): NonNullable<AvatarProps["user"]> {
  return { id: channel.id, displayName: channel.name, handle: channel.handle, avatar: channel.icon, deleted: false };
}

/** The name a comment is signed with: the channel's when it was written as one. */
export function authorName(author: common.Author | undefined): string {
  return author?.channel?.name || author?.user?.displayName || author?.user?.handle || "";
}

export function authorHandle(author: common.Author | undefined): string {
  return author?.channel?.handle || author?.user?.handle || "";
}

/** Whether two identities are the same public author (the same channel, or the same user writing as themselves). */
export function sameAuthor(a: common.Author | undefined, b: common.Author | undefined): boolean {
  if (!a || !b) return false;
  if (a.channel || b.channel) return Boolean(a.channel && b.channel && a.channel.id === b.channel.id);
  return Boolean(a.user && b.user && a.user.id === b.user.id);
}

/** Wraps `children` in a link to the author's page: the channel page, or the profile. */
export function AuthorLink({ author, className, children }: { author: common.Author | undefined; className?: string; children: ReactNode }) {
  if (author?.channel) {
    return (
      <Link to="/c/$handle" params={{ handle: author.channel.handle }} className={className}>
        {children}
      </Link>
    );
  }
  if (author?.user && !author.user.deleted) {
    return (
      <Link to="/u/$handle" params={{ handle: author.user.handle }} className={className}>
        {children}
      </Link>
    );
  }
  return <span className={className}>{children}</span>;
}

export function AuthorAvatar({ author, size = "sm", className }: { author: common.Author | undefined; size?: AvatarProps["size"]; className?: string }) {
  return <Avatar user={author?.channel ? channelFace(author.channel) : author?.user} size={size} className={className} />;
}

/** The verified mark next to a name; the label says what the colour means. */
export function VerifiedMark({ author, label, className }: { author: common.Author | undefined; label: string; className?: string }) {
  const verified = author?.channel ? author.channel.verified : author?.user?.verified;
  if (!verified) return null;
  return <BadgeCheck className={cn("size-4 shrink-0 text-brand", className)} role="img" aria-label={label} />;
}
