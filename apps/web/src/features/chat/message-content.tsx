import { Link } from "@tanstack/react-router";
import { type chat, media } from "@tokuly/api";
import { CornerUpRight, MapPin, Play } from "lucide-react";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { EmojiText } from "#/features/emoji";
import { resolveTargetPath, useOpenTargetPath } from "#/features/notifications";
import { cn } from "#/lib/cn";
import { useConversationContext } from "./conversation-context";
import { FileRow, isVisual, VisualTile, VoicePlayer } from "./media-parts";
import { displayName } from "./people";
import { replyText } from "./preview";
import { tokenize } from "./text";

/** What goes inside a message: its text, pictures, quote, preview… one component per part. */

type Tone = "own" | "other";

/** Message text with links and mentions made tappable. */
export function MessageText({ message, tone }: { message: chat.Message; tone: Tone }) {
  const open = useOpenTargetPath();
  const tokens = tokenize(
    message.text,
    message.mentions.map((mention) => mention.handle),
    message.mentionsEveryone,
  );
  const mark =
    tone === "own"
      ? "rounded bg-on-brand/20 px-0.5 font-bold"
      : "rounded bg-brand-soft px-0.5 font-bold text-brand-strong";
  return (
    <p className="whitespace-pre-wrap break-words [overflow-wrap:anywhere]">
      {tokens.map((token, index) => {
        switch (token.type) {
          case "link":
            return (
              <a
                key={index}
                href={token.href}
                target="_blank"
                rel="noreferrer"
                className="underline decoration-1 underline-offset-2"
                onClick={(event) => {
                  // Links into this app stay in this tab.
                  if (resolveTargetPath(token.href)?.type !== "internal") return;
                  event.preventDefault();
                  open(token.href);
                }}
              >
                {token.text}
              </a>
            );
          case "mention":
            return (
              <Link key={index} to="/u/$handle" params={{ handle: token.handle }} className={mark}>
                {token.text}
              </Link>
            );
          case "everyone":
            return (
              <span key={index} className={mark}>
                {token.text}
              </span>
            );
          default:
            return (
              <span key={index}>
                <EmojiText text={token.text} />
              </span>
            );
        }
      })}
    </p>
  );
}

/** The quoted message above a reply; tapping it goes there. */
export function ReplyQuote({ reply, tone }: { reply: chat.ReplyReference; tone: Tone }) {
  const { me, jumpTo } = useConversationContext();
  const { t } = useTranslation("chat");
  return (
    <button
      type="button"
      disabled={reply.unavailable}
      onClick={() => jumpTo(reply.messageId)}
      className={cn(
        "mb-1.5 flex w-full min-w-0 items-center gap-2 rounded-xl border-l-4 py-1 pr-2 pl-2 text-left text-xs",
        tone === "own" ? "border-on-brand/60 bg-on-brand/15" : "border-brand bg-surface-2",
      )}
    >
      <span className="min-w-0 flex-1">
        <span className="block truncate font-black">
          {reply.unavailable ? t("reply.unavailableTitle") : displayName(reply.sender)}
        </span>
        <span
          className={cn("block truncate", tone === "own" ? "text-on-brand/80" : "text-ink-soft")}
        >
          {replyText(reply, me.id)}
        </span>
      </span>
      {reply.thumbnailUrl && !reply.unavailable && (
        <img
          src={reply.thumbnailUrl}
          alt=""
          className="size-9 shrink-0 rounded-lg object-cover"
          loading="lazy"
        />
      )}
    </button>
  );
}

export function ForwardedLabel({ tone }: { tone: Tone }) {
  const { t } = useTranslation("chat");
  return (
    <p
      className={cn(
        "mb-1 flex items-center gap-1 text-[11px] font-bold",
        tone === "own" ? "text-on-brand/75" : "text-ink-faint",
      )}
    >
      <CornerUpRight className="size-3" />
      {t("message.forwarded")}
    </p>
  );
}

/** The pictures and videos of a message as one block: a single one large, several as a grid. */
export function VisualGrid({
  message,
  attachments,
}: {
  message: chat.Message;
  attachments: media.Attachment[];
}) {
  const { openMedia } = useConversationContext();
  const { t } = useTranslation("chat");
  const count = attachments.length;
  const label = (attachment: media.Attachment) =>
    attachment.kind === media.AssetKind.VIDEO ? t("media.openVideo") : t("media.openPhoto");
  const indexOf = (attachment: media.Attachment) =>
    message.attachments.filter(isVisual).indexOf(attachment);

  if (count === 1) {
    const only = attachments[0]!;
    const size =
      only.kind === media.AssetKind.VIDEO
        ? only.video?.poster?.width
          ? only.video.poster
          : only.video
        : only.image;
    // The box is known before the picture loads, so the list does not jump. Very tall or wide pictures are cropped.
    const ratio =
      size?.width && size.height ? Math.min(1.91, Math.max(0.6, size.width / size.height)) : 1;
    const width = ratio >= 1 ? 260 : Math.round(300 * ratio);
    return (
      <div style={{ width, aspectRatio: String(ratio) }} className="max-w-full">
        <VisualTile
          attachment={only}
          pixels={300}
          label={label(only)}
          onOpen={() => openMedia({ message, index: 0 })}
          className="size-full rounded-2xl"
        />
      </div>
    );
  }
  return (
    <div
      className={cn(
        "grid w-[17rem] max-w-full gap-1 overflow-hidden rounded-2xl",
        count === 2 || count === 4 ? "grid-cols-2" : "grid-cols-3",
      )}
    >
      {attachments.map((attachment) => (
        <VisualTile
          key={attachment.assetId || attachment.url}
          attachment={attachment}
          pixels={140}
          label={label(attachment)}
          onOpen={() => openMedia({ message, index: indexOf(attachment) })}
          className="aspect-square"
        />
      ))}
    </div>
  );
}

/** Everything a media message carries: pictures and videos, voice messages, files. */
export function MediaContent({ message, tone }: { message: chat.Message; tone: Tone }) {
  const visuals = message.attachments.filter(isVisual);
  const voices = message.attachments.filter(
    (attachment) => attachment.kind === media.AssetKind.AUDIO,
  );
  const files = message.attachments.filter(
    (attachment) => attachment.kind === media.AssetKind.FILE,
  );
  return (
    <>
      {visuals.length > 0 && <VisualGrid message={message} attachments={visuals} />}
      {voices.map((attachment) => (
        <VoicePlayer
          key={attachment.assetId || attachment.url}
          attachment={attachment}
          tone={tone}
        />
      ))}
      {files.map((attachment) => (
        <FileRow key={attachment.assetId || attachment.url} attachment={attachment} tone={tone} />
      ))}
    </>
  );
}

/** A shared place: a little map-coloured card that opens the map. */
export function LocationCard({
  location,
  onOpen,
}: {
  location: chat.LocationShare;
  onOpen(): void;
}) {
  const { t } = useTranslation("chat");
  return (
    <button
      type="button"
      onClick={onOpen}
      className="block w-60 max-w-full overflow-hidden rounded-2xl border-2 border-line bg-surface text-left text-ink"
      data-testid="location-card"
    >
      <span className="relative block h-24 bg-[linear-gradient(135deg,var(--mint)_0%,var(--sky)_100%)]">
        <span className="absolute inset-0 opacity-30 [background-image:linear-gradient(var(--surface)_2px,transparent_2px),linear-gradient(90deg,var(--surface)_2px,transparent_2px)] [background-size:28px_28px]" />
        <span className="sticker-sm absolute top-1/2 left-1/2 grid size-11 -translate-x-1/2 -translate-y-1/2 place-items-center rounded-full bg-pink text-white">
          <MapPin className="size-6" />
        </span>
      </span>
      <span className="block px-3 py-2">
        <span className="block truncate text-sm font-black">
          {location.name || t("location.shared")}
        </span>
        <span className="block truncate text-xs text-ink-soft">
          {location.address || t("location.open")}
        </span>
      </span>
    </button>
  );
}

const SPOTIFY_HEIGHT: Record<string, number> = { track: 152, episode: 152 };

/**
 * The preview of the first link in a message. YouTube and Spotify links play
 * in place once tapped (nothing is loaded from them before that); links into
 * this app open inside it.
 */
export function LinkPreviewCard({ preview, tone }: { preview: media.LinkPreview; tone: Tone }) {
  const { t } = useTranslation("chat");
  const open = useOpenTargetPath();
  const [playing, setPlaying] = useState(false);
  const youtube = preview.embedProvider === "youtube" && preview.embedId;
  const spotify = preview.embedProvider === "spotify" && preview.embedId;
  const frame = "mt-2 block w-72 max-w-full overflow-hidden rounded-2xl border-2 text-left";
  const border = tone === "own" ? "border-on-brand/30 bg-on-brand/10" : "border-line bg-surface-2";

  if (playing && youtube) {
    return (
      <div className={cn(frame, border, "aspect-video")}>
        <iframe
          src={`https://www.youtube-nocookie.com/embed/${encodeURIComponent(preview.embedId)}?autoplay=1&playsinline=1`}
          title={preview.title || "YouTube"}
          allow="autoplay; encrypted-media; picture-in-picture; fullscreen"
          allowFullScreen
          // The player needs its own scripts and origin, and opens YouTube when its logo is tapped.
          sandbox="allow-scripts allow-same-origin allow-presentation allow-popups"
          referrerPolicy="strict-origin-when-cross-origin"
          className="size-full"
        />
      </div>
    );
  }
  if (playing && spotify) {
    const type = preview.embedId.split("/")[0] ?? "";
    return (
      <div className={cn(frame, border)}>
        <iframe
          src={`https://open.spotify.com/embed/${preview.embedId.split("/").map(encodeURIComponent).join("/")}`}
          title={preview.title || "Spotify"}
          allow="autoplay; clipboard-write; encrypted-media; fullscreen; picture-in-picture"
          // The player needs its own scripts and origin, and opens Spotify from its links.
          sandbox="allow-scripts allow-same-origin allow-popups"
          referrerPolicy="strict-origin-when-cross-origin"
          className="block w-full"
          style={{ height: SPOTIFY_HEIGHT[type] ?? 352 }}
        />
      </div>
    );
  }

  const body = (
    <>
      {preview.imageUrl && (
        <span
          className={cn(
            "relative block overflow-hidden bg-surface-3",
            spotify ? "float-left mr-3 size-20" : "aspect-[1.91/1] w-full",
          )}
        >
          <img src={preview.imageUrl} alt="" className="size-full object-cover" loading="lazy" />
          {(youtube || spotify) && (
            <span className="absolute inset-0 grid place-items-center bg-black/20">
              <span
                className={cn(
                  "grid place-items-center rounded-full bg-black/60 text-white",
                  spotify ? "size-9" : "size-14",
                )}
              >
                <Play
                  className={cn("translate-x-0.5 fill-current", spotify ? "size-4" : "size-7")}
                />
              </span>
            </span>
          )}
        </span>
      )}
      <span className="block px-3 py-2">
        <span
          className={cn(
            "block truncate text-[11px] font-bold",
            tone === "own" ? "text-on-brand/70" : "text-ink-faint",
          )}
        >
          {preview.siteName || hostOf(preview.url)}
          {(youtube || spotify) && ` · ${t("link.playHere")}`}
        </span>
        {preview.title && <span className="line-clamp-2 text-sm font-black">{preview.title}</span>}
        {preview.description && !spotify && (
          <span
            className={cn(
              "mt-0.5 line-clamp-2 text-xs",
              tone === "own" ? "text-on-brand/80" : "text-ink-soft",
            )}
          >
            {preview.description}
          </span>
        )}
      </span>
    </>
  );

  if (youtube || spotify) {
    return (
      <button
        type="button"
        className={cn(frame, border)}
        onClick={() => setPlaying(true)}
        aria-label={t("link.play", { title: preview.title || preview.siteName })}
        data-testid="link-embed"
      >
        {body}
      </button>
    );
  }
  return (
    <a
      href={preview.url}
      target="_blank"
      rel="noreferrer"
      className={cn(frame, border)}
      data-testid="link-preview"
      onClick={(event) => {
        if (resolveTargetPath(preview.url)?.type !== "internal") return;
        event.preventDefault();
        open(preview.url);
      }}
    >
      {body}
    </a>
  );
}

function hostOf(url: string): string {
  try {
    return new URL(url).host.replace(/^www\./, "");
  } catch {
    return url;
  }
}
