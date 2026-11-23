import { Link } from "@tanstack/react-router";
import { type common, shorts } from "@tokuly/api";
import type { TFunction } from "i18next";
import { Heart, Music2, Play, Shuffle, Volume2 } from "lucide-react";
import { useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { Dialog } from "#/components/ui/dialog";
import { Spinner } from "#/components/ui/spinner";
import { CommentThread } from "#/features/comments";
import { EmojiText } from "#/features/emoji";
import { ChannelAvatar, ChannelName, SubscribeButton } from "#/features/video";
import { cn } from "#/lib/cn";
import { useShortLike } from "../actions";
import { Caption } from "../caption";
import type { FeedEntry } from "../lists";
import { imageUrl, type SlideLoad, shortSubject, soundTitle } from "../model";
import { useSurfaceGestures } from "./gestures";
import { ActionRail } from "./rail";
import { SlideVideo, type SlideVideoHandle } from "./slide-video";
import { type ShortVideoState, setFeedMuted, useFeedSound } from "./use-short-video";

const { FeedReason } = shorts;

/** The line above the channel saying why the short is here. */
function reasonText(entry: FeedEntry, t: TFunction<"shorts">): string {
  if (entry.repeat) return t("reason.repeat");
  if (entry.friend) return t("reason.friend", { name: entry.friend.displayName });
  switch (entry.reason) {
    case FeedReason.FOLLOWING:
      return t("reason.following");
    case FeedReason.HASHTAG:
      return entry.hashtag ? `#${entry.hashtag}` : "";
    case FeedReason.TRENDING:
      return t("reason.trending");
    case FeedReason.NEW:
      return t("reason.new");
    default:
      return "";
  }
}

const shadow = "[text-shadow:0_1px_3px_rgb(0_0_0/0.75)]";

/**
 * One short of the swipe feed, filling its slide: the video (for the short
 * on screen and the next one), the cover underneath, and on top the
 * creator, the caption, the sound and the actions. Tap pauses, a double tap
 * likes, holding plays at double speed.
 */
export function ShortSlide({
  entry,
  load,
  source,
}: {
  entry: FeedEntry;
  load: Exclude<SlideLoad, "empty">;
  source: shorts.PlaybackSource;
}) {
  const { t } = useTranslation("shorts");
  const { short } = entry;
  const active = load === "playing";
  const video = useRef<SlideVideoHandle>(null);
  const progress = useRef<HTMLDivElement>(null);
  const [state, setState] = useState<ShortVideoState>({
    paused: false,
    waiting: true,
    failed: false,
  });
  const [fast, setFast] = useState(false);
  const [expanded, setExpanded] = useState(false);
  const [commentsOpen, setCommentsOpen] = useState(false);
  const [hearts, setHearts] = useState<{ id: number; x: number; y: number }[]>([]);
  const heartId = useRef(0);
  const blocked = useFeedSound((sound) => sound.blocked);
  const like = useShortLike(short);
  const channel = short.channel;
  const own = short.viewer?.isOwner ?? false;

  const gestures = useSurfaceGestures({
    onTap: () => {
      if (!active) return;
      // The first tap on a page that started silent is for the sound.
      if (blocked) setFeedMuted(false);
      else video.current?.toggle();
    },
    onDoubleTap: (x, y) => {
      if (!active) return;
      if (like.enabled) like.like();
      const id = ++heartId.current;
      setHearts((current) => [...current, { id, x, y }]);
      setTimeout(() => setHearts((current) => current.filter((heart) => heart.id !== id)), 800);
    },
    onHoldStart: () => {
      if (!active || state.paused) return;
      video.current?.setFast(true);
      setFast(true);
    },
    onHoldEnd: () => {
      video.current?.setFast(false);
      setFast(false);
    },
  });

  const cover = imageUrl(short.cover, 400);
  const reason = reasonText(entry, t);
  const playState = !active
    ? "idle"
    : state.failed
      ? "failed"
      : state.paused
        ? "paused"
        : state.waiting
          ? "loading"
          : "playing";

  return (
    <article
      data-testid="short-slide"
      data-short-id={short.shortId}
      data-state={playState}
      aria-label={t("slide.label", { name: channel?.name ?? "" })}
      className="relative size-full overflow-hidden bg-black text-white"
      style={short.cover?.dominantColor ? { background: short.cover.dominantColor } : undefined}
    >
      {cover && (
        <img
          src={cover}
          alt=""
          decoding="async"
          className="absolute inset-0 size-full object-cover"
        />
      )}
      {load !== "cover" && (
        <SlideVideo
          ref={video}
          short={short}
          active={active}
          source={source}
          progress={progress}
          onState={setState}
        />
      )}

      {/* The surface under everything else: tap, double tap, hold. Vertical drags stay the scroller's. */}
      <button
        type="button"
        data-surface
        aria-label={state.paused ? t("player.play") : t("player.pause")}
        className="absolute inset-0 cursor-default touch-pan-y select-none outline-none [-webkit-touch-callout:none]"
        {...gestures}
        // From the keyboard there is no pointer: Enter and Space pause.
        onClick={(event) => event.detail === 0 && active && video.current?.toggle()}
        onContextMenu={(event) => event.preventDefault()}
      />

      {active && state.paused && (
        <span className="pointer-events-none absolute inset-0 grid place-items-center">
          <span className="grid size-20 animate-pop-in place-items-center rounded-full bg-black/45">
            <Play className="size-10 translate-x-0.5 fill-white" aria-hidden />
          </span>
        </span>
      )}
      {active && state.waiting && !state.paused && !state.failed && (
        <span className="pointer-events-none absolute inset-0 grid place-items-center">
          <Spinner className="size-9 text-white/80" />
        </span>
      )}
      {active && state.failed && (
        <span className="pointer-events-none absolute inset-0 grid place-items-center px-10 text-center">
          <span className={cn("font-black", shadow)}>{t("player.failed")}</span>
        </span>
      )}
      {fast && (
        <span className="pointer-events-none absolute inset-x-0 top-20 flex justify-center">
          <span className="rounded-full bg-black/55 px-3 py-1 text-sm font-black">
            {t("player.fast")}
          </span>
        </span>
      )}
      {active && blocked && (
        <span className="pointer-events-none absolute inset-x-0 top-20 flex justify-center">
          <span className="flex animate-pop-in items-center gap-1.5 rounded-full bg-white px-3 py-1.5 text-sm font-black text-[#17122b]">
            <Volume2 className="size-4" aria-hidden />
            {t("player.tapForSound")}
          </span>
        </span>
      )}
      {hearts.map((heart) => (
        <Heart
          key={heart.id}
          aria-hidden
          data-testid="like-burst"
          className="pointer-events-none absolute size-24 -translate-x-1/2 -translate-y-1/2 animate-pop-in fill-pink text-pink drop-shadow-lg"
          style={{ left: heart.x, top: heart.y }}
        />
      ))}

      <div className="pointer-events-none absolute inset-x-0 bottom-0 h-2/5 bg-gradient-to-t from-black/70 to-transparent" />

      <div className="pointer-events-none absolute inset-x-0 bottom-0 flex items-end gap-2 px-3 pb-5">
        <div className={cn("pointer-events-auto min-w-0 flex-1", shadow)}>
          {reason && (
            <p className="mb-1.5 inline-block max-w-full truncate rounded-full bg-black/40 px-2.5 py-0.5 text-xs font-black [text-shadow:none]">
              <EmojiText text={reason} />
            </p>
          )}
          {channel && (
            <div className="flex items-center gap-2">
              <Link
                to="/c/$handle"
                params={{ handle: channel.handle }}
                className="flex min-w-0 items-center gap-2 font-black"
              >
                <ChannelAvatar channel={channel} size="sm" outlined />
                <ChannelName channel={channel} className="[&_svg]:text-white" />
              </Link>
              {!own && !short.viewer?.subscribed && (
                <span className="[text-shadow:none]">
                  <SubscribeButton channel={channel} subscribed={false} size="sm" />
                </span>
              )}
            </div>
          )}
          {short.caption && (
            <div className="mt-2 text-sm leading-relaxed">
              <p
                data-testid="short-caption"
                className={cn(
                  "whitespace-pre-wrap break-words",
                  expanded ? "max-h-[38dvh] overflow-y-auto" : "line-clamp-2",
                )}
              >
                <Caption short={short} />
              </p>
              {(short.caption.length > 60 || short.caption.includes("\n")) && (
                <button
                  type="button"
                  className="mt-0.5 font-black text-white/80"
                  aria-expanded={expanded}
                  onClick={() => setExpanded((open) => !open)}
                >
                  {expanded ? t("caption.less") : t("caption.more")}
                </button>
              )}
            </div>
          )}
          {short.remixOf?.shortId && (
            <Link
              to="/shorts/$shortId"
              params={{ shortId: short.remixOf.shortId }}
              className="mt-2 flex items-center gap-1.5 text-sm font-bold"
            >
              <Shuffle className="size-4 shrink-0" aria-hidden />
              <span className="truncate">
                {t(
                  short.remixOf.kind === shorts.RemixKind.DUET ? "slide.duetOf" : "slide.remixOf",
                  { name: short.remixOf.channel?.name ?? "" },
                )}
              </span>
            </Link>
          )}
          {short.sound && (
            <Link
              to="/sound/$soundId"
              params={{ soundId: short.sound.id }}
              data-testid="short-sound"
              className="mt-2 flex items-center gap-1.5 text-sm font-bold"
            >
              <Music2 className="size-4 shrink-0" aria-hidden />
              <span className="truncate">
                <EmojiText
                  text={soundTitle(short.sound, (name) => t("sound.original", { name }))}
                />
              </span>
            </Link>
          )}
        </div>
        <ActionRail
          short={short}
          active={active}
          onComments={() => setCommentsOpen(true)}
          className="pointer-events-auto shrink-0"
        />
      </div>

      <div className="pointer-events-none absolute inset-x-0 bottom-0 h-1 bg-white/25">
        <div ref={progress} className="h-full origin-left scale-x-0 bg-white" />
      </div>

      {/* Comments rise over the video, which keeps playing behind them. */}
      <Dialog
        open={commentsOpen}
        onOpenChange={setCommentsOpen}
        title={t("comments.title")}
        className="max-sm:h-[72dvh]"
      >
        {commentsOpen && (
          <CommentThread
            subject={shortSubject(short)}
            subjectAuthor={channel ? ({ channel } as common.Author) : undefined}
            defaultChannelId={own ? channel?.id : undefined}
          />
        )}
      </Dialog>
    </article>
  );
}
