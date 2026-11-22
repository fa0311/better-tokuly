import { Dialog as BaseDialog } from "@base-ui/react/dialog";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { Link } from "@tanstack/react-router";
import { common, type notes, toMillis } from "@tokuly/api";
import { ChevronLeft, ChevronRight, Ellipsis, Eye, Flag, Pause, Pencil, SmilePlus, Sparkles, Trash2, Volume2, VolumeX, X } from "lucide-react";
import { type PointerEvent as ReactPointerEvent, type RefObject, useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { Avatar } from "#/components/ui/avatar";
import { ConfirmDialog } from "#/components/ui/confirm";
import { Menu, type MenuAction } from "#/components/ui/menu";
import { Spinner } from "#/components/ui/spinner";
import { EmojiPicker, ReportDialog, useBurst } from "#/features/engagement";
import { imageUrl, isVideo, useVideoSource } from "#/features/notes";
import { cn } from "#/lib/cn";
import { errorMessage } from "#/lib/errors";
import { compactAge } from "#/lib/format";
import { useMe } from "#/stores/session";
import { patchStory, removeStory, storyClient, storyKeys } from "./api";
import { HighlightEditor, HighlightPicker, useDeleteHighlight } from "./highlights";
import { closeStoryViewer, useStoryViewer, type ViewerReel } from "./viewer-store";
import { ViewersDialog } from "./viewers";

/** How long a photo stays before the next one. */
const PHOTO_MS = 5000;
/** A press longer than this pauses instead of tapping. */
const HOLD_MS = 220;
/** Finger travel that makes a press a swipe. */
const SWIPE_SLOP = 14;
const SWIPE_REEL = 56;
const SWIPE_CLOSE = 90;
const QUICK_REACTIONS = ["❤️", "😂", "😮", "😢", "👏", "🔥"];

interface Segment {
  story: notes.Story;
  item: notes.StoryItem;
  key: string;
}

function segmentsOf(reel: ViewerReel): Segment[] {
  return reel.stories.flatMap((story) => story.items.map((item, index) => ({ story, item, key: `${story.id}:${index}` })));
}

/** Where a reel starts: at the story asked for, else at the first one not watched yet, else at the beginning. */
function startOf(reel: ViewerReel, storyId?: string): number {
  const segments = segmentsOf(reel);
  const wanted = storyId ? segments.findIndex((segment) => segment.story.id === storyId) : segments.findIndex((segment) => !segment.story.viewed);
  return Math.max(0, wanted);
}

function motionOff(): boolean {
  return document.documentElement.dataset.dopa === "0" || window.matchMedia("(prefers-reduced-motion: reduce)").matches;
}

/** The viewer, mounted once with the stories background. */
export function StoryViewerHost() {
  const reels = useStoryViewer((state) => state.reels);
  const reelIndex = useStoryViewer((state) => state.reelIndex);
  const storyId = useStoryViewer((state) => state.storyId);
  if (!reels) return null;
  return <StoryViewer reels={reels} startReel={reelIndex} startStoryId={storyId} />;
}

type Sheet = "viewers" | "delete" | "report" | "highlight" | "editHighlight" | "deleteHighlight";

/**
 * Stories full screen: a tap on the right goes on, a tap on the left goes
 * back, holding pauses, a swipe sideways moves between people and a swipe
 * down closes. Friends react with an emoji; the owner sees who watched.
 */
function StoryViewer({ reels, startReel, startStoryId }: { reels: ViewerReel[]; startReel: number; startStoryId?: string }) {
  const { t } = useTranslation("stories");
  const me = useMe();
  const queryClient = useQueryClient();
  const [position, setPosition] = useState(() => ({ reel: startReel, segment: startOf(reels[startReel]!, startStoryId) }));
  const [holding, setHolding] = useState(false);
  const [sheet, setSheet] = useState<Sheet>();
  const [pickerOpen, setPickerOpen] = useState(false);
  const [ready, setReady] = useState(false);
  const [muted, setMuted] = useState(false);
  const [hidden, setHidden] = useState(document.visibilityState !== "visible");
  /** Stories deleted while the viewer is open (its reels are a snapshot). */
  const [removed, setRemoved] = useState<ReadonlySet<string>>(new Set());
  /** The viewer's reactions made here, by story; `""` = taken back. */
  const [reacted, setReacted] = useState<Record<string, string>>({});
  const marked = useRef(new Set<string>());
  const bar = useRef<HTMLSpanElement>(null);
  const video = useRef<HTMLVideoElement>(null);
  const press = useRef<{ x: number; y: number; timer: ReturnType<typeof setTimeout> | undefined; held: boolean; swiped: boolean } | null>(null);
  const deleteHighlight = useDeleteHighlight();
  const { fire, burst } = useBurst(["💜"]);

  const removedUsers = useStoryViewer((state) => state.removedUserIds);
  const popup = useRef<HTMLDivElement>(null);
  const gone = (index: number) => removedUsers.has(reels[index]?.user?.id ?? "");
  const reel = reels[position.reel]!;
  const segments = segmentsOf(reel).filter((segment) => !removed.has(segment.story.id));
  const segment = segments[Math.min(position.segment, segments.length - 1)];
  const mine = Boolean(me?.user && reel.user?.id === me.user.id);
  const paused = holding || sheet !== undefined || pickerOpen || hidden || !ready;

  const close = () => closeStoryViewer();
  const goReel = (delta: number) => {
    let next = position.reel + delta;
    // Reels of people who are no longer friends are passed over.
    while (next >= 0 && next < reels.length && gone(next)) next += delta;
    if (next < 0) return;
    if (next >= reels.length) {
      close();
      return;
    }
    setReady(false);
    setPosition({ reel: next, segment: startOf(reels[next]!) });
  };
  const next = () => {
    if (position.segment < segments.length - 1) {
      setReady(false);
      setPosition({ ...position, segment: position.segment + 1 });
    } else goReel(1);
  };
  const previous = () => {
    if (position.segment > 0) {
      setReady(false);
      setPosition({ ...position, segment: position.segment - 1 });
    } else if (position.reel > 0) goReel(-1);
    else if (video.current) video.current.currentTime = 0;
  };

  // The latest state for the animation frame loop and the key handler, which outlive a render.
  const live = useRef({ paused, next, previous, goReel });
  useEffect(() => {
    live.current = { paused, next, previous, goReel };
  });

  // The keyboard: arrows move, Space holds. It works wherever the focus is inside the viewer (or
  // nowhere at all), and stays out of the way of a sheet, a menu, the emoji picker and text fields.
  const keys = useRef({ blocked: false, holding });
  useEffect(() => {
    keys.current = { blocked: sheet !== undefined || pickerOpen, holding };
  });
  useEffect(() => {
    popup.current?.focus();
    const onKey = (event: KeyboardEvent) => {
      const target = event.target instanceof HTMLElement ? event.target : document.body;
      const inside = target === document.body || Boolean(popup.current?.contains(target));
      // A menu that is open owns the arrows, also in the moment before the focus has moved into it.
      if (keys.current.blocked || !inside || document.querySelector('[role="menu"]') || target.isContentEditable || ["INPUT", "TEXTAREA", "SELECT"].includes(target.tagName)) return;
      if (event.key === "ArrowRight") live.current.next();
      else if (event.key === "ArrowLeft") live.current.previous();
      // On a button or a link, Space presses it.
      else if (event.key === " " && !target.closest("button, a")) {
        event.preventDefault();
        setHolding(!keys.current.holding);
      }
    };
    // Heard on the way down: the dialog keeps some keys from travelling back up.
    window.addEventListener("keydown", onKey, true);
    return () => window.removeEventListener("keydown", onKey, true);
  }, []);

  useEffect(() => {
    const onVisibility = () => setHidden(document.visibilityState !== "visible");
    document.addEventListener("visibilitychange", onVisibility);
    return () => document.removeEventListener("visibilitychange", onVisibility);
  }, []);

  // A reel that lost its last story (deleted from here) has nothing left to show.
  const empty = segments.length === 0;
  useEffect(() => {
    if (empty) live.current.goReel(1);
  }, [empty]);

  // The person on screen stopped being a friend (or blocked the viewer, or was blocked): the viewer closes.
  const reelGone = gone(position.reel);
  useEffect(() => {
    if (reelGone) closeStoryViewer();
  }, [reelGone]);

  // Watching a friend's story counts once, when it is really shown: its picture is up and the tab is in front.
  const storyId = segment?.story.id;
  const unseen = Boolean(segment && !segment.story.viewed && !mine);
  const shown = ready && !hidden;
  useEffect(() => {
    if (!storyId || !unseen || !shown || marked.current.has(storyId)) return;
    marked.current.add(storyId);
    void storyClient()
      .markStoryViewed({ storyId })
      .then(() => patchStory(queryClient, storyId, (story) => ({ ...story, viewed: true })))
      .catch(() => {});
  }, [storyId, unseen, shown, queryClient]);

  // The progress of the piece on screen: a photo runs on a clock, a video on its own time.
  const segmentKey = segment?.key;
  const segmentIsVideo = segment ? isVideo(segment.item.media!) : false;
  useEffect(() => {
    if (!segmentKey) return;
    const still = motionOff();
    let elapsed = 0;
    let last = performance.now();
    let frame = requestAnimationFrame(function tick(now) {
      const delta = now - last;
      last = now;
      let progress = 0;
      const player = video.current;
      if (segmentIsVideo) {
        if (player?.duration) progress = player.currentTime / player.duration;
        if (player?.ended) {
          live.current.next();
          return;
        }
      } else if (still) {
        // With motion off nothing moves on by itself: a tap does.
        progress = 1;
      } else {
        if (!live.current.paused) elapsed += delta;
        progress = elapsed / PHOTO_MS;
        if (progress >= 1) {
          live.current.next();
          return;
        }
      }
      if (bar.current) bar.current.style.transform = `scaleX(${Math.min(1, progress)})`;
      frame = requestAnimationFrame(tick);
    });
    return () => cancelAnimationFrame(frame);
  }, [segmentKey, segmentIsVideo]);

  // A video stops while the viewer is paused and goes on when it is not.
  useEffect(() => {
    const player = video.current;
    if (!player || !segmentIsVideo) return;
    if (paused) player.pause();
    else
      void player.play().catch(() => {
        // Sound needs a tap on some browsers: go on silently rather than not at all.
        setMuted(true);
      });
  }, [paused, segmentIsVideo]);

  if (!segment) return null;
  const story = segment.story;
  const myReaction = reacted[story.id] ?? story.myReaction;

  const react = async (emoji: string) => {
    const taking = myReaction === emoji;
    setReacted({ ...reacted, [story.id]: taking ? "" : emoji });
    if (!taking) fire([emoji]);
    try {
      if (taking) await storyClient().removeStoryReaction({ storyId: story.id });
      else await storyClient().reactToStory({ storyId: story.id, emoji });
      patchStory(queryClient, story.id, (current) => ({ ...current, myReaction: taking ? "" : emoji, viewed: true }));
    } catch (error) {
      setReacted({ ...reacted, [story.id]: myReaction });
      toast.error(errorMessage(error));
    }
  };

  const onPointerDown = (event: ReactPointerEvent<HTMLDivElement>) => {
    if (event.pointerType === "mouse" && event.button !== 0) return;
    const state = { x: event.clientX, y: event.clientY, held: false, swiped: false, timer: undefined as ReturnType<typeof setTimeout> | undefined };
    state.timer = setTimeout(() => {
      state.held = true;
      setHolding(true);
    }, HOLD_MS);
    press.current = state;
    event.currentTarget.setPointerCapture(event.pointerId);
  };
  const onPointerMove = (event: ReactPointerEvent<HTMLDivElement>) => {
    const state = press.current;
    if (!state || state.swiped) return;
    if (Math.hypot(event.clientX - state.x, event.clientY - state.y) > SWIPE_SLOP) {
      state.swiped = true;
      clearTimeout(state.timer);
    }
  };
  const onPointerUp = (event: ReactPointerEvent<HTMLDivElement>) => {
    const state = press.current;
    press.current = null;
    if (!state) return;
    clearTimeout(state.timer);
    if (state.held) {
      setHolding(false);
      return;
    }
    const dx = event.clientX - state.x;
    const dy = event.clientY - state.y;
    if (state.swiped) {
      if (Math.abs(dx) > Math.abs(dy)) {
        if (dx <= -SWIPE_REEL) goReel(1);
        else if (dx >= SWIPE_REEL) goReel(-1);
      } else if (dy >= SWIPE_CLOSE) close();
      return;
    }
    const box = event.currentTarget.getBoundingClientRect();
    if (event.clientX - box.left < box.width * 0.3) previous();
    else next();
  };
  const onPointerCancel = () => {
    clearTimeout(press.current?.timer);
    press.current = null;
    setHolding(false);
  };

  const actions: MenuAction[] = mine
    ? [
        ...(reel.highlight
          ? [
              { label: t("viewer.editHighlight"), icon: <Pencil />, onSelect: () => setSheet("editHighlight") },
              { label: t("viewer.deleteHighlight"), icon: <Trash2 />, danger: true, onSelect: () => setSheet("deleteHighlight") },
            ]
          : [{ label: t("viewer.addToHighlight"), icon: <Sparkles />, onSelect: () => setSheet("highlight") }]),
        { label: t("viewer.delete"), icon: <Trash2 />, danger: true, onSelect: () => setSheet("delete") },
      ]
    : [{ label: t("viewer.report"), icon: <Flag />, danger: true, onSelect: () => setSheet("report") }];

  return (
    <BaseDialog.Root open onOpenChange={(open) => !open && close()}>
      <BaseDialog.Portal>
        <BaseDialog.Popup
          aria-label={t("viewer.label", { name: reel.user?.displayName ?? "" })}
          data-testid="story-viewer"
          data-paused={holding}
          className="fixed inset-0 z-50 flex items-center justify-center bg-black text-white outline-none transition-opacity duration-150 data-[ending-style]:opacity-0 data-[starting-style]:opacity-0 sm:bg-black/90"
          ref={popup}
          initialFocus={popup}
        >
          <BaseDialog.Title className="sr-only">{t("viewer.label", { name: reel.user?.displayName ?? "" })}</BaseDialog.Title>
          <button type="button" aria-label={t("viewer.previous")} onClick={previous} className="mr-3 hidden size-11 shrink-0 place-items-center rounded-full bg-white/15 hover:bg-white/25 sm:grid">
            <ChevronLeft className="size-6" />
          </button>

          <div className="relative flex size-full flex-col overflow-hidden bg-black sm:aspect-[9/16] sm:h-[min(92dvh,56rem)] sm:w-auto sm:shrink-0 sm:rounded-[1.75rem] sm:border-2 sm:border-white/25">
            {/* The picture or video, and the surface that takes taps, holds and swipes. */}
            <div className="absolute inset-0 touch-none select-none" onPointerDown={onPointerDown} onPointerMove={onPointerMove} onPointerUp={onPointerUp} onPointerCancel={onPointerCancel} data-testid="story-stage">
              <SegmentMedia key={segment.key} segment={segment} video={video} muted={muted} onReady={() => setReady(true)} />
              {!ready && (
                <span className="absolute inset-0 grid place-items-center">
                  <Spinner className="size-8 text-white/80" />
                </span>
              )}
              {holding && (
                <span className="absolute inset-0 grid place-items-center bg-black/20">
                  <span className="grid size-14 place-items-center rounded-full bg-black/55">
                    <Pause className="size-6 fill-current" />
                  </span>
                </span>
              )}
            </div>

            <div className={cn("pointer-events-none relative z-10 bg-gradient-to-b from-black/65 to-transparent px-3 pt-safe pb-8 transition-opacity", holding && "opacity-0")}>
              <div className="flex gap-1" role="progressbar" aria-valuemin={1} aria-valuemax={segments.length} aria-valuenow={position.segment + 1}>
                {segments.map((item, index) => (
                  <span key={item.key} className="h-1 flex-1 overflow-hidden rounded-full bg-white/35">
                    <span
                      ref={index === position.segment ? bar : undefined}
                      className="block h-full origin-left bg-white"
                      style={{ transform: `scaleX(${index < position.segment ? 1 : 0})` }}
                    />
                  </span>
                ))}
              </div>
              <div className="pointer-events-auto mt-2.5 flex items-center gap-2.5">
                {reel.user && (
                  <Link to="/u/$handle" params={{ handle: reel.user.handle }} onClick={close} className="flex min-w-0 flex-1 items-center gap-2.5">
                    <Avatar user={reel.user} size="sm" />
                    <span className="min-w-0">
                      <span className="block truncate text-sm font-black">{reel.highlight ? reel.highlight.title : reel.user.displayName}</span>
                      <span className="block truncate text-xs text-white/75">
                        {reel.highlight ? reel.user.displayName : `@${reel.user.handle}`} · {compactAge(toMillis(story.createdAt))}
                      </span>
                    </span>
                  </Link>
                )}
                {segmentIsVideo && (
                  <button type="button" aria-label={muted ? t("viewer.unmute") : t("viewer.mute")} onClick={() => setMuted(!muted)} className="grid size-9 place-items-center rounded-full hover:bg-white/15">
                    {muted ? <VolumeX className="size-5" /> : <Volume2 className="size-5" />}
                  </button>
                )}
                <Menu
                  trigger={
                    <button type="button" aria-label={t("viewer.menu")} className="grid size-9 place-items-center rounded-full hover:bg-white/15">
                      <Ellipsis className="size-5" />
                    </button>
                  }
                  actions={actions}
                />
                <BaseDialog.Close aria-label={t("viewer.close")} className="grid size-9 place-items-center rounded-full hover:bg-white/15">
                  <X className="size-5" />
                </BaseDialog.Close>
              </div>
            </div>

            <div className="flex-1" />

            <div className={cn("pointer-events-none relative z-10 bg-gradient-to-t from-black/75 to-transparent px-4 pt-10 pb-safe transition-opacity", holding && "opacity-0")}>
              {story.title && <p className="text-lg font-black leading-snug break-words">{story.title}</p>}
              {segment.item.caption && <p className="mt-1 text-[15px] leading-relaxed break-words whitespace-pre-wrap text-white/90">{segment.item.caption}</p>}
              <div className="pointer-events-auto mt-3 mb-1">
                {mine ? (
                  <OwnerBar story={story} onOpen={() => setSheet("viewers")} />
                ) : (
                  // biome-ignore lint/a11y/useSemanticElements: a row of emoji buttons, not a form group.
                  <div className="relative flex items-center justify-between gap-1" role="group" aria-label={t("viewer.react")}>
                    {QUICK_REACTIONS.map((emoji) => (
                      <button
                        key={emoji}
                        type="button"
                        aria-label={emoji}
                        aria-pressed={myReaction === emoji}
                        onClick={() => void react(emoji)}
                        className={cn("grid size-11 place-items-center rounded-full text-2xl transition-transform active:scale-125", myReaction === emoji ? "bg-white/30 ring-2 ring-white" : "bg-white/10 hover:bg-white/20")}
                      >
                        <span aria-hidden>{emoji}</span>
                      </button>
                    ))}
                    <EmojiPickerButton
                      current={myReaction && !QUICK_REACTIONS.includes(myReaction) ? myReaction : undefined}
                      onOpenChange={setPickerOpen}
                      onPick={(emoji) => void react(emoji)}
                      label={t("viewer.moreReactions")}
                    />
                    <span className="pointer-events-none absolute top-0 left-1/2 text-2xl">{burst}</span>
                  </div>
                )}
              </div>
            </div>
          </div>

          <button type="button" aria-label={t("viewer.next")} onClick={next} className="ml-3 hidden size-11 shrink-0 place-items-center rounded-full bg-white/15 hover:bg-white/25 sm:grid">
            <ChevronRight className="size-6" />
          </button>

          {sheet === "viewers" && <ViewersDialog storyId={story.id} onClose={() => setSheet(undefined)} />}
          {sheet === "highlight" && <HighlightPicker storyId={story.id} onClose={() => setSheet(undefined)} />}
          {sheet === "editHighlight" && reel.highlight && <HighlightEditor highlight={reel.highlight} onClose={() => setSheet(undefined)} onSaved={close} />}
          <ConfirmDialog
            open={sheet === "deleteHighlight"}
            onOpenChange={(open) => !open && setSheet(undefined)}
            title={t("highlights.deleteTitle", { title: reel.highlight?.title ?? "" })}
            description={t("highlights.deleteBody")}
            confirmLabel={t("highlights.delete")}
            danger
            onConfirm={async () => {
              if (reel.highlight && (await deleteHighlight(reel.highlight))) close();
              else setSheet(undefined);
            }}
          />
          <ConfirmDialog
            open={sheet === "delete"}
            onOpenChange={(open) => !open && setSheet(undefined)}
            title={t("viewer.deleteTitle")}
            description={t("viewer.deleteBody")}
            confirmLabel={t("viewer.delete")}
            danger
            onConfirm={async () => {
              try {
                await storyClient().deleteStory({ storyId: story.id });
                removeStory(queryClient, story.id);
                setRemoved(new Set(removed).add(story.id));
                toast(t("viewer.deleted"));
              } catch (error) {
                toast.error(errorMessage(error));
              }
              setSheet(undefined);
            }}
          />
          <ReportDialog
            subject={{ type: common.SubjectType.STORY, id: story.id }}
            open={sheet === "report"}
            onOpenChange={(open) => !open && setSheet(undefined)}
            label={t("viewer.reportLabel", { handle: reel.user?.handle ?? "" })}
          />
        </BaseDialog.Popup>
      </BaseDialog.Portal>
    </BaseDialog.Root>
  );
}

/** One photo or video of a story, filling the stage. It says when it can be shown, which is when its time starts. */
function SegmentMedia({ segment, video, muted, onReady }: { segment: Segment; video: RefObject<HTMLVideoElement | null>; muted: boolean; onReady(): void }) {
  const attachment = segment.item.media!;
  const info = attachment.video;
  useVideoSource(video, info, isVideo(attachment));
  if (isVideo(attachment)) {
    return (
      <video ref={video} poster={imageUrl(info?.poster, 720) || undefined} muted={muted} playsInline preload="auto" onCanPlay={onReady} onError={onReady} className="size-full object-contain" />
    );
  }
  return (
    <img
      src={imageUrl(attachment.image, 720)}
      alt={segment.item.caption}
      draggable={false}
      onLoad={onReady}
      onError={onReady}
      className="size-full object-contain"
      style={{ background: attachment.image?.dominantColor || undefined }}
    />
  );
}

/** For the owner: how many people watched and reacted, live; a tap lists them. */
function OwnerBar({ story, onOpen }: { story: notes.Story; onOpen(): void }) {
  const { t } = useTranslation("stories");
  // The snapshot the viewer opened with is kept current by realtime events through this cache entry.
  const { data } = useQuery({
    queryKey: storyKeys.story(story.id),
    queryFn: async () => (await storyClient().getStory({ storyId: story.id })).story,
    initialData: story,
    staleTime: 30_000,
  });
  const current = data ?? story;
  return (
    <button type="button" onClick={onOpen} className="flex h-11 w-full items-center gap-3 rounded-full bg-white/15 px-4 text-sm font-black hover:bg-white/25" data-testid="story-views">
      <Eye className="size-5" />
      <span>{t("viewer.views", { count: current.viewCount })}</span>
      {current.reactionCount > 0 && <span className="text-white/80">· {t("viewer.reactions", { count: current.reactionCount })}</span>}
      <span className="flex-1" />
      <ChevronRight className="size-4 opacity-70" />
    </button>
  );
}

/** The "any emoji" button at the end of the quick reactions. It shows the viewer's reaction when that is not a quick one. */
function EmojiPickerButton({ current, label, onPick, onOpenChange }: { current?: string; label: string; onPick(emoji: string): void; onOpenChange(open: boolean): void }) {
  return (
    <EmojiPicker
      onPick={onPick}
      onOpenChange={onOpenChange}
      trigger={
        <button type="button" aria-label={label} aria-pressed={Boolean(current)} className={cn("grid size-11 place-items-center rounded-full text-2xl", current ? "bg-white/30 ring-2 ring-white" : "bg-white/10 hover:bg-white/20")}>
          {current ? <span aria-hidden>{current}</span> : <SmilePlus className="size-5" />}
        </button>
      }
    />
  );
}
