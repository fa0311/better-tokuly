import { Link } from "@tanstack/react-router";
import { community } from "@tokuly/api";
import {
  BarChart3,
  Heart,
  Lock,
  Megaphone,
  MessageCircle,
  Pin,
  ShieldQuestion,
} from "lucide-react";
import { useTranslation } from "react-i18next";
import { Chip } from "#/components/ui/card";
import { EmojiText } from "#/features/emoji";
import { compactCount } from "#/features/engagement";
import { cn } from "#/lib/cn";
import { NS } from "./api";
import { imageUrl } from "./model";
import { PostAuthor } from "./parts";

/** The chips that say what state a thread is in. */
export function ThreadFlags({ thread }: { thread: community.Thread }) {
  const { t } = useTranslation(NS);
  return (
    <>
      {thread.pinned && (
        <Chip className="bg-sun text-[#17122b]">
          <Pin className="size-3" />
          {t("thread.pinned")}
        </Chip>
      )}
      {thread.announcement && (
        <Chip className="bg-sky text-[#17122b]">
          <Megaphone className="size-3" />
          {t("thread.announcement")}
        </Chip>
      )}
      {thread.locked && (
        <Chip>
          <Lock className="size-3" />
          {t("thread.locked")}
        </Chip>
      )}
      {thread.state === community.PostState.HIDDEN && (
        <Chip className="bg-coral text-white">
          <ShieldQuestion className="size-3" />
          {t("thread.hidden")}
        </Chip>
      )}
    </>
  );
}

/** A thread in a list: who, the title, a glimpse of it, how busy it is. */
export function ThreadCard({
  thread,
  handle,
  channel,
}: {
  thread: community.Thread;
  /** Handle of the thread's community. */
  handle: string;
  /** Shown when the list mixes channels. */
  channel?: community.Channel;
}) {
  const { t, i18n } = useTranslation(NS);
  const unread = thread.viewer?.unread ?? false;
  const thumb = imageUrl(thread.images[0], 72);
  const flagged =
    thread.pinned ||
    thread.announcement ||
    thread.locked ||
    thread.state === community.PostState.HIDDEN;
  return (
    <article className="relative rounded-card bg-surface p-4 transition-colors hover:bg-surface-2">
      <div className="flex items-center gap-2">
        <div className="min-w-0 flex-1">
          <PostAuthor
            size="sm"
            author={thread.author}
            pseudonym={thread.pseudonym}
            mine={thread.viewer?.mine}
            createdAtMs={Number(thread.createdAt?.seconds ?? 0n) * 1000}
          />
        </div>
        {channel && (
          <Chip className="shrink-0">
            {channel.emoji} {channel.name}
          </Chip>
        )}
      </div>
      <div className="mt-2 flex gap-3">
        <div className="min-w-0 flex-1">
          {flagged && (
            <div className="mb-1.5 flex flex-wrap gap-1.5">
              <ThreadFlags thread={thread} />
            </div>
          )}
          <h3
            className={cn(
              "break-words text-[17px] leading-snug",
              unread ? "font-black" : "font-bold",
            )}
          >
            {/* The whole card opens the thread; the author's link above stays its own target. */}
            <Link
              to="/g/$handle/t/$threadId"
              params={{ handle, threadId: thread.shortId || thread.id }}
              className="after:absolute after:inset-0"
            >
              <EmojiText text={thread.title} />
            </Link>
          </h3>
          {thread.body && (
            <p className="mt-1 line-clamp-2 break-words text-sm leading-relaxed text-ink-soft">
              <EmojiText text={thread.body} />
            </p>
          )}
        </div>
        {thumb && (
          <img
            src={thumb}
            alt=""
            loading="lazy"
            className="size-18 shrink-0 rounded-2xl border-2 border-line object-cover"
          />
        )}
      </div>
      <div className="mt-2.5 flex flex-wrap items-center gap-x-3 gap-y-1 text-xs font-bold text-ink-soft">
        <span className="inline-flex items-center gap-1">
          <MessageCircle className="size-3.5" />
          {t("thread.replies", { count: thread.replyCount })}
        </span>
        <span className="inline-flex items-center gap-1">
          <Heart className="size-3.5" />
          {compactCount(Number(thread.likeCount), i18n.language)}
        </span>
        {thread.poll && (
          <span className="inline-flex items-center gap-1">
            <BarChart3 className="size-3.5" />
            {t("poll.voters", { count: thread.poll.voterCount })}
          </span>
        )}
        {thread.tags.slice(0, 3).map((tag) => (
          <span key={tag} className="text-ink-faint">
            #{tag}
          </span>
        ))}
        {unread && (
          <span className="ml-auto inline-flex items-center gap-1 text-pink">
            <span className="size-2 rounded-full bg-pink" />
            {(thread.viewer?.unreadReplyCount ?? 0) > 0
              ? t("thread.unreadReplies", { count: thread.viewer?.unreadReplyCount })
              : t("thread.new")}
          </span>
        )}
      </div>
    </article>
  );
}
