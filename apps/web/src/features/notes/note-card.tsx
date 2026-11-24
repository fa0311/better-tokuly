import { Link, useNavigate } from "@tanstack/react-router";
import { common, type media, type notes, toMillis } from "@tokuly/api";
import { EyeOff, MapPin, Pin, Play, Repeat2 } from "lucide-react";
import { type MouseEvent, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { Avatar } from "#/components/ui/avatar";
import { EmojiText } from "#/features/emoji";
import { ReactionChips, useOpenPath, useSubjectTopic } from "#/features/engagement";
import { StampImage, StampSheet } from "#/features/stamps";
import { cn } from "#/lib/cn";
import { linkPreviewImage } from "#/lib/link-preview-image";
import { useMe } from "#/stores/session";
import { useIsDeleted } from "./api";
import { ChannelChip } from "./channels-chip";
import { attachmentImage, ImageGrid, imageUrl, isVideo, NoteVideo } from "./media";
import { NoteActions, NoteMenu, noteSubject } from "./note-actions";
import { NotePoll } from "./poll";
import { NoteRichText } from "./rich-text";
import { noteAge, noteDateTime, useClock } from "./time";
import { useRecordView } from "./views";
import { VisibilityMark } from "./visibility";

export interface NoteCardProps {
  note: notes.Note;
  /** Set when the row is in a timeline because someone renoted the note. */
  renote?: notes.RenoteInfo;
  /**
   * `list` (default): a row of a timeline or a thread; a tap anywhere opens
   * the note. `focus`: the note its own page is about, drawn larger.
   */
  variant?: "list" | "focus";
  /** Say that the note is pinned (on its author's profile). */
  showPinned?: boolean;
  /** Leave out 「返信先: @handle」: the note it answers is right above (a thread). */
  hideReplyContext?: boolean;
  /** Called after the viewer deleted the note from its menu. */
  onDeleted?(): void;
  className?: string;
}

/**
 * A note as people see it everywhere: who wrote it and when, the text with
 * its links, pictures or a video, a stamp, the place, the note it quotes,
 * live counters and every action. While it is on screen it follows the
 * note's realtime topic and counts as viewed.
 */
export function NoteCard({
  note,
  renote,
  variant = "list",
  showPinned,
  hideReplyContext,
  onDeleted,
  className,
}: NoteCardProps) {
  const { t, i18n } = useTranslation("notes");
  const navigate = useNavigate();
  const me = useMe();
  const card = useRef<HTMLElement>(null);
  const deleted = useIsDeleted(note.id);
  useClock();
  useSubjectTopic(common.SubjectType.NOTE, note.id, !deleted);
  useRecordView(card, note.id, !deleted && variant === "list" && note.author?.id !== me?.user?.id);

  const author = note.author;
  const posted = toMillis(note.createdAt);
  const focus = variant === "focus";
  const subject = noteSubject(note.id);

  if (deleted) {
    return (
      <div
        className={cn(
          "rounded-[1.25rem] border-2 border-dashed border-line px-4 py-5 text-center text-sm font-bold text-ink-faint",
          className,
        )}
        data-testid="note-deleted"
      >
        {t("card.deleted")}
      </div>
    );
  }

  const open = (event: MouseEvent<HTMLElement>) => {
    if (focus) return;
    const target = event.target as HTMLElement;
    // Clicks inside menus and dialogs bubble here through React although they are elsewhere in the page.
    if (!event.currentTarget.contains(target)) return;
    if (target.closest("a, button, video, iframe, input, textarea, [data-stop-open]")) return;
    if (window.getSelection()?.toString()) return;
    void navigate({ to: "/notes/$noteId", params: { noteId: note.id } });
  };

  const names = author && (
    <span className={cn("flex min-w-0 gap-x-1.5", focus ? "flex-col" : "items-baseline")}>
      <Link
        to="/u/$handle"
        params={{ handle: author.handle }}
        className={cn("truncate font-black hover:underline", !focus && "max-w-[62%] shrink-0")}
      >
        <EmojiText text={author.displayName} />
        {author.verified && (
          <span className="ml-1" role="img" aria-label={t("card.verified")}>
            ✅
          </span>
        )}
      </Link>
      {/* When space is short the handle gives way before the name does. */}
      <span className="min-w-0 truncate text-sm text-ink-faint">@{author.handle}</span>
    </span>
  );
  const time = (
    <Link
      to="/notes/$noteId"
      params={{ noteId: note.id }}
      className="shrink-0 text-sm text-ink-faint hover:underline"
      title={noteDateTime(posted, i18n.language)}
    >
      <time dateTime={new Date(posted).toISOString()}>
        {focus ? noteDateTime(posted, i18n.language) : noteAge(posted, i18n.language)}
      </time>
    </Link>
  );

  return (
    <article
      ref={card}
      data-testid="note"
      data-note-id={note.id}
      onClick={open}
      className={cn(
        "relative rounded-[1.25rem] border-2 bg-surface px-3.5 pt-3 pb-1.5 transition-colors",
        focus ? "border-ink shadow-pop" : "cursor-pointer border-line hover:border-ink-faint",
        className,
      )}
    >
      {renote?.user && (
        <p className="mb-2 flex items-center gap-1.5 text-xs font-black text-success">
          <Repeat2 className="size-4 shrink-0" strokeWidth={3} />
          <Link
            to="/u/$handle"
            params={{ handle: renote.user.handle }}
            className="truncate hover:underline"
          >
            {t("card.renotedBy", { name: renote.user.displayName })}
          </Link>
        </p>
      )}
      {showPinned && note.pinned && (
        <p className="mb-2 flex items-center gap-1.5 text-xs font-black text-brand">
          <Pin className="size-3.5 shrink-0 fill-current" />
          {t("card.pinned")}
        </p>
      )}
      <ChannelChip channel={note.channel} />

      <div className="grid grid-cols-[auto_minmax(0,1fr)] gap-x-3">
        {author && (
          <Link
            to="/u/$handle"
            params={{ handle: author.handle }}
            aria-label={author.displayName}
            className={cn("self-start", focus && "row-span-1")}
          >
            <Avatar user={author} size="md" />
          </Link>
        )}
        <div className="min-w-0">
          <header className="flex items-center gap-1.5">
            {names}
            {!focus && (
              <>
                <span aria-hidden className="text-ink-faint">
                  ·
                </span>
                {time}
              </>
            )}
            {!focus && (
              <VisibilityMark
                visibility={note.visibility}
                direct={note.directRecipients.length > 0}
              />
            )}
            {note.editCount > 0 && (
              <span className="shrink-0 text-xs font-bold text-ink-faint">{t("card.edited")}</span>
            )}
            <span className="flex-1" />
            <NoteMenu note={note} renoter={renote?.user} onDeleted={onDeleted} />
          </header>
          {!focus && !hideReplyContext && <ReplyContext note={note} />}
          {!focus && <NoteBody note={note} className="mt-1" />}
        </div>

        {focus && (
          <div className="col-span-2 mt-3">
            <ReplyContext note={note} />
            <NoteBody note={note} large />
            <p className="mt-3 flex items-center gap-2 text-sm text-ink-faint">
              {time}
              <VisibilityMark
                visibility={note.visibility}
                direct={note.directRecipients.length > 0}
              />
            </p>
          </div>
        )}

        <div className={cn("col-span-2 mt-1.5", !focus && "sm:col-span-1 sm:col-start-2")}>
          <ReactionChips subject={subject} className="mb-1" />
          <NoteActions
            note={note}
            className={cn("-mx-1.5", focus && "border-t-2 border-line pt-1")}
          />
        </div>
      </div>
    </article>
  );
}

/** 「返信先: @handle」 above a reply. It keeps rendering after the note it answered was deleted. */
function ReplyContext({ note }: { note: notes.Note }) {
  const { t } = useTranslation("notes");
  const target = note.replyTo;
  if (!target?.author) return null;
  return (
    <p className="truncate text-sm text-ink-faint">
      {t("card.replyingTo")}{" "}
      {target.unavailable ? (
        <span>@{target.author.handle}</span>
      ) : (
        <Link
          to="/notes/$noteId"
          params={{ noteId: target.noteId }}
          className="font-bold text-brand hover:underline"
        >
          @{target.author.handle}
        </Link>
      )}
    </p>
  );
}

/**
 * What a note says: text, media, stamp, link preview, quoted note, place.
 * A note marked sensitive keeps all of it behind a tap.
 */
export function NoteBody({
  note,
  large,
  embedded,
  className,
}: {
  note: notes.Note;
  large?: boolean /** Inside a quote: no nested quote, small media. */;
  embedded?: boolean;
  className?: string;
}) {
  const { t } = useTranslation("notes");
  const [revealed, setRevealed] = useState(false);
  const [stampOpen, setStampOpen] = useState(false);

  if (note.contentWarning && !revealed) {
    return (
      <div
        className={cn(
          "flex items-center gap-3 rounded-2xl border-2 border-dashed border-line bg-surface-2 px-3 py-3",
          className,
        )}
      >
        <EyeOff className="size-5 shrink-0 text-ink-soft" />
        <p className="min-w-0 flex-1 text-sm font-bold text-ink-soft">
          {note.contentWarningText ? (
            <EmojiText text={note.contentWarningText} />
          ) : (
            t("card.sensitive")
          )}
        </p>
        <button
          type="button"
          onClick={() => setRevealed(true)}
          className="sticker-sm pressable h-8 shrink-0 rounded-full bg-surface px-3 text-sm font-black"
        >
          {t("card.reveal")}
        </button>
      </div>
    );
  }

  const video = note.attachments.find(isVideo);
  const images = note.attachments.filter((attachment) => !isVideo(attachment));
  return (
    <div className={cn("flex flex-col gap-2.5", className)}>
      <NoteRichText
        note={note}
        className={cn(
          large ? "text-[1.15rem] leading-relaxed" : "text-[15px] leading-relaxed",
          embedded && "line-clamp-4",
        )}
      />
      {embedded ? (
        <EmbeddedMedia attachments={note.attachments} />
      ) : video ? (
        <NoteVideo attachment={video} />
      ) : (
        <ImageGrid attachments={images} details={note.attachmentDetails} />
      )}
      {note.poll && <NotePoll note={note} poll={note.poll} disabled={embedded} />}
      {note.stamp && (
        <button
          type="button"
          aria-label={note.stamp.name}
          onClick={() => setStampOpen(true)}
          className="w-fit"
          disabled={embedded}
        >
          <StampImage stamp={note.stamp} size={embedded ? 64 : 120} />
        </button>
      )}
      {stampOpen && note.stamp && (
        <StampSheet stampId={note.stamp.id} onClose={() => setStampOpen(false)} />
      )}
      {!embedded && note.linkPreview?.url && !note.quote && note.attachments.length === 0 && (
        <LinkPreviewCard preview={note.linkPreview} />
      )}
      {!embedded && note.quote && <QuoteEmbed quote={note.quote} />}
      {note.place?.name && (
        <p className="flex w-fit max-w-full items-center gap-1 rounded-full bg-surface-2 px-2.5 py-1 text-xs font-black text-ink-soft">
          <MapPin className="size-3.5 shrink-0 text-coral" />
          <span className="truncate">{note.place.name}</span>
        </p>
      )}
    </div>
  );
}

/** Media inside a quote: small squares, a play mark on a video. They do not open; the quote itself does. */
function EmbeddedMedia({ attachments }: { attachments: media.Attachment[] }) {
  if (attachments.length === 0) return null;
  return (
    <div className="flex gap-1.5">
      {attachments.slice(0, 4).map((attachment, index) => (
        <span
          key={attachment.assetId || index}
          className="relative size-16 overflow-hidden rounded-xl border-2 border-ink bg-surface-3"
        >
          <img
            src={imageUrl(attachmentImage(attachment), 64)}
            alt=""
            loading="lazy"
            className="size-full object-cover"
          />
          {isVideo(attachment) && (
            <span className="absolute inset-0 grid place-items-center bg-black/25 text-white">
              <Play className="size-5 fill-current" />
            </span>
          )}
        </span>
      ))}
    </div>
  );
}

/** The note a quote note shows underneath its own text. */
export function QuoteEmbed({ quote, className }: { quote: notes.QuotedNote; className?: string }) {
  const { t, i18n } = useTranslation("notes");
  const navigate = useNavigate();
  const deleted = useIsDeleted(quote.noteId);
  const note = quote.note;
  if (quote.unavailable || deleted || !note) {
    return (
      <div
        className={cn(
          "rounded-2xl border-2 border-dashed border-line px-3 py-3 text-sm font-bold text-ink-faint",
          className,
        )}
      >
        {t("card.quoteUnavailable")}
      </div>
    );
  }
  return (
    // biome-ignore lint/a11y/useSemanticElements: the embed holds links of its own (a profile, hashtags), which an anchor cannot contain.
    <div
      role="link"
      tabIndex={0}
      data-stop-open
      data-testid="quote"
      onClick={(event) => {
        if ((event.target as HTMLElement).closest("a, button")) return;
        void navigate({ to: "/notes/$noteId", params: { noteId: note.id } });
      }}
      onKeyDown={(event) => {
        if (event.key === "Enter" && event.target === event.currentTarget)
          void navigate({ to: "/notes/$noteId", params: { noteId: note.id } });
      }}
      className={cn(
        "cursor-pointer rounded-2xl border-2 border-line bg-surface-2/60 p-3 transition-colors hover:border-ink-faint",
        className,
      )}
    >
      <div className="mb-1 flex items-center gap-1.5">
        <Avatar user={note.author} size="xs" />
        <span className="truncate text-sm font-black">
          <EmojiText text={note.author?.displayName ?? ""} />
        </span>
        <span className="truncate text-xs text-ink-faint">@{note.author?.handle}</span>
        <span className="shrink-0 text-xs text-ink-faint">
          · {noteAge(toMillis(note.createdAt), i18n.language)}
        </span>
      </div>
      <NoteBody note={note} embedded />
    </div>
  );
}

/** The preview of the first link in a note. YouTube and Spotify links play in place once tapped. */
function LinkPreviewCard({ preview }: { preview: media.LinkPreview }) {
  const { t } = useTranslation("notes");
  const open = useOpenPath();
  const [playing, setPlaying] = useState(false);
  // Pictures that do not load leave no empty frame behind.
  const [imageFailed, setImageFailed] = useState(false);
  const [faviconFailed, setFaviconFailed] = useState(false);
  const embed =
    preview.embedProvider === "youtube" && preview.embedId
      ? {
          src: `https://www.youtube-nocookie.com/embed/${encodeURIComponent(preview.embedId)}?autoplay=1`,
          className: "aspect-video",
        }
      : preview.embedProvider === "spotify" && /^[a-z]+\/[A-Za-z0-9]+$/.test(preview.embedId)
        ? { src: `https://open.spotify.com/embed/${preview.embedId}`, className: "h-[152px]" }
        : undefined;

  if (embed && playing) {
    return (
      <iframe
        src={embed.src}
        title={preview.title || preview.siteName}
        allow="autoplay; encrypted-media; picture-in-picture; fullscreen"
        loading="lazy"
        className={cn("w-full rounded-2xl border-2 border-ink bg-black", embed.className)}
      />
    );
  }

  let host = preview.siteName;
  try {
    host ||= new URL(preview.url).hostname.replace(/^www\./, "");
  } catch {
    // An unparsable URL keeps whatever name the preview came with.
  }
  const image = imageFailed ? undefined : linkPreviewImage(preview.imageUrl);
  const favicon = faviconFailed ? undefined : linkPreviewImage(preview.faviconUrl);
  const body = (
    <>
      {image && (
        <span className="relative block w-24 shrink-0 self-stretch bg-surface-3 sm:w-32">
          <img
            src={image}
            alt=""
            loading="lazy"
            className="absolute inset-0 size-full object-cover"
            onError={() => setImageFailed(true)}
          />
          {embed && (
            <span className="absolute inset-0 grid place-items-center bg-black/25 text-white">
              <Play className="size-7 fill-current" />
            </span>
          )}
        </span>
      )}
      <span className="min-w-0 flex-1 px-3 py-2.5">
        <span className="flex items-center gap-1.5 text-xs font-bold text-ink-faint">
          {favicon && (
            <img
              src={favicon}
              alt=""
              className="size-3.5 rounded-sm"
              onError={() => setFaviconFailed(true)}
            />
          )}
          <span className="truncate">{host}</span>
        </span>
        <span className="mt-0.5 line-clamp-2 text-sm font-black leading-snug">
          {preview.title || preview.url}
        </span>
        {preview.description && (
          <span className="mt-0.5 line-clamp-1 text-xs text-ink-soft">{preview.description}</span>
        )}
      </span>
    </>
  );
  const classes =
    "flex min-h-20 overflow-hidden rounded-2xl border-2 border-line bg-surface text-left transition-colors hover:border-ink-faint";
  if (embed) {
    return (
      <button
        type="button"
        aria-label={t("card.playEmbed", { title: preview.title })}
        onClick={() => setPlaying(true)}
        className={classes}
      >
        {body}
      </button>
    );
  }
  // A link to this app's own pages stays in the app.
  if (preview.embedProvider === "tokuly") {
    return (
      <button type="button" onClick={() => open(preview.url)} className={classes}>
        {body}
      </button>
    );
  }
  return (
    <a href={preview.url} target="_blank" rel="noopener noreferrer nofollow" className={classes}>
      {body}
    </a>
  );
}
