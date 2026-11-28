import { common, type engagement } from "@tokuly/api";
import { ChevronRight } from "lucide-react";
import { type ComponentType, Suspense } from "react";
import { useTranslation } from "react-i18next";
import { create } from "zustand";
import { useOpenPath } from "./open-path";

export interface SubjectCardProps {
  /** What engagement knows about the subject: its reference, a title, a path, maybe an image. It is available. */
  preview: engagement.SubjectPreview;
}

/** The cards registered so far; a list drawn before a feature registered draws again when it does. */
const useRegistry = create<Partial<Record<common.SubjectType, ComponentType<SubjectCardProps>>>>(
  () => ({}),
);

/**
 * Tells the lists of saved and reacted-to subjects how to draw one type of
 * content. Each content feature registers its own card once, when its module
 * loads (its `background.tsx` is a good place), as an `onDemand` component so
 * the card's code loads with the first list that draws one:
 *
 *   const VideoSubjectCard = onDemand(
 *     () => import("./subject-card").then((module) => module.VideoSubjectCard),
 *     "notice",
 *   );
 *   registerSubjectCard(common.SubjectType.VIDEO, VideoSubjectCard);
 *
 * Types without a card of their own get a generic row with the title, the
 * thumbnail and a link.
 */
export function registerSubjectCard(
  type: common.SubjectType,
  component: ComponentType<SubjectCardProps>,
): void {
  useRegistry.setState({ [type]: component });
}

/** Name of a subject type in the `engagement` strings (`subjects.<key>`), with a catch-all. */
const TYPE_KEYS: Partial<Record<common.SubjectType, string>> = {
  [common.SubjectType.USER]: "user",
  [common.SubjectType.CHANNEL]: "channel",
  [common.SubjectType.NOTE]: "note",
  [common.SubjectType.STORY]: "story",
  [common.SubjectType.VIDEO]: "video",
  [common.SubjectType.SHORT]: "short",
  [common.SubjectType.LIVE_STREAM]: "live",
  [common.SubjectType.CLIP]: "clip",
  [common.SubjectType.PLAYLIST]: "playlist",
  [common.SubjectType.COMMENT]: "comment",
  [common.SubjectType.STAMP_PACK]: "stampPack",
  [common.SubjectType.STAMP]: "stamp",
  [common.SubjectType.PLACE]: "place",
  [common.SubjectType.NEWS_ARTICLE]: "news",
};

export function subjectTypeKey(type: common.SubjectType | undefined): string {
  return (type !== undefined && TYPE_KEYS[type]) || "other";
}

const TYPE_EMOJI: Record<string, string> = {
  user: "🧑",
  channel: "📺",
  note: "📝",
  story: "📸",
  video: "🎬",
  short: "⚡",
  live: "🔴",
  clip: "✂️",
  playlist: "🎶",
  comment: "💬",
  stampPack: "🧸",
  stamp: "🧸",
  place: "📍",
  news: "📰",
  other: "✨",
};

/**
 * One saved (or reacted-to) subject, drawn by the card its feature
 * registered, or generically. A subject that is gone says so.
 */
export function SubjectCard({ preview }: SubjectCardProps) {
  const { t } = useTranslation("engagement");
  const registered = useRegistry((cards) =>
    preview.subject ? cards[preview.subject.type] : undefined,
  );
  if (!preview.available) {
    return (
      <div className="flex items-center gap-3 rounded-card border-2 border-dashed border-line px-4 py-5 text-ink-soft">
        <span className="text-2xl" aria-hidden>
          🫥
        </span>
        <div className="min-w-0">
          <p className="font-bold">{t("subjects.unavailable")}</p>
          <p className="text-sm text-ink-faint">
            {t(`subjects.${subjectTypeKey(preview.subject?.type)}`)}
          </p>
        </div>
      </div>
    );
  }
  const Card = registered ?? GenericSubjectCard;
  return (
    <Suspense fallback={null}>
      <Card preview={preview} />
    </Suspense>
  );
}

/** The fallback card: thumbnail, type, title, and a tap that opens the subject's page. */
export function GenericSubjectCard({ preview }: SubjectCardProps) {
  const { t } = useTranslation("engagement");
  const open = useOpenPath();
  const key = subjectTypeKey(preview.subject?.type);
  const body = (
    <>
      {preview.imageUrl ? (
        <img
          src={preview.imageUrl}
          alt=""
          loading="lazy"
          className="size-16 shrink-0 rounded-2xl border-2 border-ink object-cover"
        />
      ) : (
        <span
          aria-hidden
          className="grid size-16 shrink-0 place-items-center rounded-2xl border-2 border-ink bg-surface-2 text-3xl"
        >
          {TYPE_EMOJI[key]}
        </span>
      )}
      <span className="min-w-0 flex-1">
        <span className="block text-xs font-black text-brand">{t(`subjects.${key}`)}</span>
        <span className="line-clamp-2 font-bold leading-snug">
          {preview.title || t("subjects.untitled")}
        </span>
      </span>
    </>
  );
  const classes = "sticker flex w-full items-center gap-3 rounded-card bg-surface p-3 text-left";
  if (!preview.path) return <div className={classes}>{body}</div>;
  return (
    <button type="button" onClick={() => open(preview.path)} className={`${classes} pressable`}>
      {body}
      <ChevronRight className="size-5 shrink-0 text-ink-faint" />
    </button>
  );
}
