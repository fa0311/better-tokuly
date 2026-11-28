import { type common, toMillis, type video } from "@tokuly/api";
import {
  audienceKey,
  categoryKey,
  chapterAt,
  descriptionParts,
  formatClock,
} from "@tokuly/core/video";
import { ChevronDown, ChevronUp, Gamepad2 } from "lucide-react";
import { type ReactNode, useState } from "react";
import { cn } from "#/lib/cn";
import { useTranslation } from "#/lib/use-translation";

/** A creator's free text with its web addresses and clock times (`1:23`) made tappable. */
function linked(text: string, onSeek?: (seconds: number) => void): ReactNode[] {
  return descriptionParts(text).map((part, index) => {
    const key = `${index}-${part.text}`;
    if (part.kind === "link")
      return (
        <a
          key={key}
          href={part.text}
          target="_blank"
          rel="noopener noreferrer nofollow ugc"
          className="font-bold text-brand underline-offset-2 hover:underline"
        >
          {part.text}
        </a>
      );
    if (part.kind === "time" && onSeek)
      return (
        <button
          key={key}
          type="button"
          className="font-bold text-brand hover:underline"
          onClick={() => onSeek(part.seconds)}
        >
          {part.text}
        </button>
      );
    return part.text;
  });
}

/**
 * The box under a video's title: when it went up, what the creator wrote
 * (folded until asked for), tags, genre, game, and the chapters as a list
 * that jumps to each.
 */
export function Description({
  video: item,
  currentTime,
  onSeek,
}: {
  video: video.Video /** Position of the player, to mark the chapter playing. */;
  currentTime: number;
  onSeek(seconds: number): void;
}) {
  const { t, i18n } = useTranslation("video");
  const [open, setOpen] = useState(false);
  const summary = item.summary;
  const published = summary?.publishedAt ? new Date(toMillis(summary.publishedAt)) : undefined;
  const category = summary ? categoryKey(summary.category) : undefined;
  const hasMore =
    item.description.length > 140 ||
    item.description.split("\n").length > 3 ||
    item.chapters.length > 0 ||
    item.tags.length > 0;
  const playing = chapterAt(item.chapters, currentTime);

  return (
    <section
      data-testid="video-description"
      className="rounded-card bg-surface-2 p-4 text-[15px]"
      aria-label={t("watch.description")}
    >
      <p className="flex flex-wrap gap-x-3 gap-y-1 text-sm font-black">
        <span>
          {t("card.views", {
            count: Number(summary?.viewCount ?? 0n),
            formatted: Number(summary?.viewCount ?? 0n).toLocaleString(i18n.language),
          })}
        </span>
        {published && (
          <span>
            {published.toLocaleDateString(i18n.language, {
              year: "numeric",
              month: "long",
              day: "numeric",
            })}
          </span>
        )}
      </p>
      {item.description ? (
        <p
          className={cn(
            "mt-2 leading-relaxed break-words whitespace-pre-wrap",
            !open && "line-clamp-3",
          )}
        >
          {linked(item.description, onSeek)}
        </p>
      ) : (
        <p className="mt-2 text-ink-faint">{t("watch.noDescription")}</p>
      )}

      {open && (
        <div className="mt-4 flex flex-col gap-4">
          {(category || item.game) && (
            <p className="flex flex-wrap gap-2">
              {category && <Tag>{t(category)}</Tag>}
              {item.game && (
                <Tag>
                  <Gamepad2 className="size-3.5" />
                  {item.game}
                </Tag>
              )}
            </p>
          )}
          {item.tags.length > 0 && (
            <ul
              className="flex flex-wrap gap-x-3 gap-y-1 text-sm font-bold text-brand"
              aria-label={t("watch.tags")}
            >
              {item.tags.map((tag) => (
                <li key={tag}>#{tag}</li>
              ))}
            </ul>
          )}
          {item.chapters.length > 0 && (
            <Chapters chapters={item.chapters} playing={playing} onSeek={onSeek} />
          )}
        </div>
      )}

      {hasMore && (
        <button
          type="button"
          aria-expanded={open}
          className="mt-2 inline-flex items-center gap-1 text-sm font-black text-ink-soft hover:text-ink"
          onClick={() => setOpen((value) => !value)}
        >
          {open ? <ChevronUp className="size-4" /> : <ChevronDown className="size-4" />}
          {open ? t("watch.showLess") : t("watch.showMore")}
        </button>
      )}
    </section>
  );
}

function Tag({ children }: { children: ReactNode }) {
  return (
    <span className="inline-flex h-7 items-center gap-1 rounded-full bg-surface px-3 text-xs font-black text-ink-soft">
      {children}
    </span>
  );
}

function Chapters({
  chapters,
  playing,
  onSeek,
}: {
  chapters: video.Chapter[];
  playing?: video.Chapter;
  onSeek(seconds: number): void;
}) {
  const { t } = useTranslation("video");
  return (
    <div>
      <h3 className="text-sm font-black">{t("watch.chapters")}</h3>
      <ol className="mt-1.5 flex flex-col">
        {chapters.map((chapter) => (
          <li key={chapter.startSeconds}>
            <button
              type="button"
              aria-current={chapter === playing ? "true" : undefined}
              className={cn(
                "flex w-full items-center gap-3 rounded-xl px-2 py-1.5 text-left hover:bg-surface",
                chapter === playing && "bg-surface",
              )}
              onClick={() => onSeek(chapter.startSeconds)}
            >
              <span className="w-14 shrink-0 rounded-md bg-brand-soft px-1.5 text-center text-xs leading-6 font-black text-brand-strong tabular-nums">
                {formatClock(chapter.startSeconds)}
              </span>
              <span className="min-w-0 flex-1 truncate font-bold">{chapter.title}</span>
            </button>
          </li>
        ))}
      </ol>
    </div>
  );
}

/**
 * One line under a title saying who can see the video, for anything that is not plainly
 * public (`@tokuly/core/video`'s `audienceKey`).
 */
export function audienceNote(
  visibility: common.Visibility,
  allowLink: boolean,
  t: (key: string) => string,
): string {
  const key = audienceKey(visibility, allowLink);
  return key ? t(key) : "";
}
