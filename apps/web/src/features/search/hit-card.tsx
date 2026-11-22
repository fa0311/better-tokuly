import { search } from "@tokuly/api";
import { BadgeCheck, EyeOff, ListVideo, Reply } from "lucide-react";
import type { ReactNode } from "react";
import { useTranslation } from "react-i18next";
import { Avatar } from "#/components/ui/avatar";
import { Coins } from "#/components/ui/coins";
import { cn } from "#/lib/cn";
import { AppLink } from "./app-link";
import { ageLabel, formatCount, formatDuration } from "./format";
import { Segments } from "./highlight";
import { KindTile } from "./kind-icon";
import { hitHref } from "./hit-href";
import { extraFlag, extraNumber, extraText, type HitExtra, hitExtra } from "./kinds";

const K = search.SearchKind;

/** One search result as a compact card that links to its subject. Every kind has its own face. */
export function HitCard({ hit, className }: { hit: search.SearchHit; className?: string }) {
  const extra = hitExtra(hit);
  return (
    <AppLink
      href={hitHref(hit)}
      data-testid="search-hit"
      data-kind={search.SearchKind[hit.kind]}
      className={cn("flex min-w-0 gap-3 rounded-2xl border-2 border-line bg-surface p-3 transition-colors hover:border-ink-faint", className)}
    >
      <Face hit={hit} extra={extra} />
    </AppLink>
  );
}

function Face({ hit, extra }: { hit: search.SearchHit; extra: HitExtra }) {
  switch (hit.kind) {
    case K.USER:
      return <UserFace hit={hit} extra={extra} />;
    case K.CHANNEL:
      return <ChannelFace hit={hit} extra={extra} />;
    case K.NOTE:
      return <NoteFace hit={hit} extra={extra} />;
    case K.VIDEO:
    case K.SHORT:
    case K.LIVE:
    case K.CLIP:
    case K.PLAYLIST:
      return <MediaFace hit={hit} extra={extra} />;
    case K.STAMP_PACK:
      return <PackFace hit={hit} extra={extra} />;
    case K.HASHTAG:
      return <HashtagFace hit={hit} extra={extra} />;
    case K.PLACE:
      return <PlaceFace hit={hit} extra={extra} />;
    default:
      return <ArticleFace hit={hit} extra={extra} />;
  }
}

interface FaceProps {
  hit: search.SearchHit;
  extra: HitExtra;
}

function Title({ hit, className }: { hit: search.SearchHit; className?: string }) {
  return <Segments segments={hit.titleSegments} fallback={hit.title} className={cn("font-black break-words", className)} />;
}

function Snippet({ hit, lines = 2 }: { hit: search.SearchHit; lines?: 1 | 2 | 3 }) {
  if (hit.snippet.length === 0) return null;
  return <Segments segments={hit.snippet} className={cn("text-sm leading-snug break-words text-ink-soft", lines === 1 ? "line-clamp-1" : lines === 2 ? "line-clamp-2" : "line-clamp-3")} />;
}

function Meta({ children }: { children: ReactNode }) {
  return <span className="flex min-w-0 flex-wrap items-center gap-x-1.5 text-xs text-ink-soft">{children}</span>;
}

const Dot = () => <span aria-hidden>·</span>;

function Verified() {
  const { t } = useTranslation("search");
  return <BadgeCheck className="size-4 shrink-0 text-brand" aria-label={t("hit.verified")} />;
}

function UserFace({ hit, extra }: FaceProps) {
  const handle = extraText(extra, "handle") || hit.owner?.handle;
  return (
    <>
      {hit.owner ? <Avatar user={hit.owner} outlined /> : <KindTile kind={hit.kind} />}
      <span className="flex min-w-0 flex-1 flex-col">
        <span className="flex min-w-0 items-center gap-1">
          <Title hit={hit} className="truncate" />
          {(extraFlag(extra, "verified") || hit.owner?.verified) && <Verified />}
        </span>
        {handle && <span className="truncate text-sm text-ink-soft">@{handle}</span>}
        <Snippet hit={hit} />
      </span>
    </>
  );
}

function ChannelFace({ hit, extra }: FaceProps) {
  const { t } = useTranslation("search");
  const handle = extraText(extra, "handle");
  const subscribers = extraNumber(extra, "subscriber_count");
  return (
    <>
      {hit.imageUrl ? <img src={hit.imageUrl} alt="" className="size-11 shrink-0 rounded-full border-2 border-ink bg-surface-3 object-cover" loading="lazy" decoding="async" draggable={false} /> : <KindTile kind={hit.kind} className="rounded-full" />}
      <span className="flex min-w-0 flex-1 flex-col">
        <span className="flex min-w-0 items-center gap-1">
          <Title hit={hit} className="truncate" />
          {extraFlag(extra, "verified") && <Verified />}
        </span>
        <Meta>
          {handle && <span className="truncate">@{handle}</span>}
          {handle && subscribers !== undefined && <Dot />}
          {subscribers !== undefined && <span>{t("hit.subscribers", { count: subscribers, formatted: formatCount(subscribers) })}</span>}
        </Meta>
        <Snippet hit={hit} />
      </span>
    </>
  );
}

function NoteFace({ hit, extra }: FaceProps) {
  const { t } = useTranslation("search");
  const author = hit.owner;
  const hidden = extraFlag(extra, "content_warning");
  return (
    <>
      {author ? <Avatar user={author} size="sm" /> : <KindTile kind={hit.kind} className="size-8 rounded-[0.6rem]" iconClassName="size-4" />}
      <span className="flex min-w-0 flex-1 flex-col gap-0.5">
        <Meta>
          <span className="truncate font-bold text-ink">{author?.displayName || extraText(extra, "author_name") || author?.handle}</span>
          <Dot />
          <span>{ageLabel(hit.createdAt)}</span>
          {typeof extra.reply_to_id === "string" && extra.reply_to_id && (
            <span className="inline-flex items-center gap-0.5">
              <Reply className="size-3.5" />
              {t("hit.reply")}
            </span>
          )}
        </Meta>
        {hit.snippet.length > 0 ? <Segments segments={hit.snippet} className="line-clamp-3 text-[15px] leading-snug break-words" /> : <Title hit={hit} className="line-clamp-3 text-[15px] leading-snug font-medium" />}
        {hidden && (
          <span className="mt-0.5 inline-flex w-fit items-center gap-1 rounded-full bg-surface-2 px-2 py-0.5 text-xs font-bold text-ink-soft">
            <EyeOff className="size-3.5" />
            {t("hit.contentWarning")}
          </span>
        )}
      </span>
      {hit.imageUrl && <img src={hit.imageUrl} alt="" className="size-16 shrink-0 rounded-xl bg-surface-3 object-cover" loading="lazy" decoding="async" draggable={false} />}
    </>
  );
}

function MediaFace({ hit, extra }: FaceProps) {
  const { t } = useTranslation("search");
  const tall = hit.kind === K.SHORT;
  const duration = extraNumber(extra, "duration_seconds");
  const items = extraNumber(extra, "item_count");
  const maker = extraText(extra, "channel_name") || hit.owner?.displayName || hit.owner?.handle;
  return (
    <>
      <span className={cn("relative shrink-0 overflow-hidden rounded-xl bg-surface-3", tall ? "aspect-[9/16] w-20" : "aspect-video w-32 sm:w-40")}>
        {hit.imageUrl ? (
          <img src={hit.imageUrl} alt="" className="size-full object-cover" loading="lazy" decoding="async" draggable={false} />
        ) : (
          <span className="grid size-full place-items-center">
            <KindTile kind={hit.kind} className="size-9 rounded-xl" iconClassName="size-[18px]" />
          </span>
        )}
        {hit.kind === K.LIVE && <span className="absolute top-1.5 left-1.5 rounded-md bg-live px-1.5 py-0.5 text-[10px] font-black tracking-wide text-white">LIVE</span>}
        {hit.kind === K.PLAYLIST && items !== undefined && (
          <span className="absolute inset-y-0 right-0 flex w-2/5 flex-col items-center justify-center gap-0.5 bg-black/70 text-xs font-black text-white">
            <ListVideo className="size-4" />
            {items}
          </span>
        )}
        {hit.kind !== K.PLAYLIST && hit.kind !== K.LIVE && duration !== undefined && duration > 0 && (
          <span className="absolute right-1.5 bottom-1.5 rounded-md bg-black/75 px-1.5 py-0.5 text-[11px] font-bold text-white tabular-nums">{formatDuration(duration)}</span>
        )}
      </span>
      <span className="flex min-w-0 flex-1 flex-col gap-0.5">
        <Title hit={hit} className="line-clamp-2 leading-snug" />
        <Meta>
          {maker && <span className="truncate">{maker}</span>}
          {maker && hit.createdAt && <Dot />}
          <span>{ageLabel(hit.createdAt)}</span>
          {extraFlag(extra, "age_restricted") && <span className="rounded bg-surface-2 px-1 font-bold">{t("hit.ageRestricted")}</span>}
        </Meta>
        <Snippet hit={hit} lines={tall ? 3 : 1} />
      </span>
    </>
  );
}

function PackFace({ hit, extra }: FaceProps) {
  const { t } = useTranslation("search");
  const price = extraNumber(extra, "price") ?? 0;
  const stamps = extraNumber(extra, "stamp_count");
  const official = extraFlag(extra, "official");
  return (
    <>
      {hit.imageUrl ? <img src={hit.imageUrl} alt="" className="size-14 shrink-0 rounded-xl bg-surface-2 object-contain p-1" loading="lazy" decoding="async" draggable={false} /> : <KindTile kind={hit.kind} className="size-14 rounded-xl" />}
      <span className="flex min-w-0 flex-1 flex-col gap-0.5">
        <span className="flex min-w-0 items-center gap-1.5">
          <Title hit={hit} className="truncate" />
          {official && <span className="shrink-0 rounded-full bg-brand px-1.5 py-0.5 text-[10px] font-black text-on-brand">{t("hit.official")}</span>}
        </span>
        <Meta>
          {!official && hit.owner && !hit.owner.deleted && <span className="truncate">{hit.owner.displayName || hit.owner.handle}</span>}
          {!official && hit.owner && !hit.owner.deleted && stamps !== undefined && <Dot />}
          {stamps !== undefined && <span>{t("hit.stampCount", { count: stamps })}</span>}
        </Meta>
        <Snippet hit={hit} lines={1} />
      </span>
      <span className="shrink-0 self-center">{price > 0 ? <Coins amount={price} size="sm" /> : <span className="rounded-full bg-lime px-2.5 py-1 text-xs font-black text-[#17122b]">{t("hit.free")}</span>}</span>
    </>
  );
}

function HashtagFace({ hit, extra }: FaceProps) {
  const { t } = useTranslation("search");
  const notes = extraNumber(extra, "note_count");
  return (
    <>
      <KindTile kind={hit.kind} />
      <span className="flex min-w-0 flex-1 flex-col justify-center">
        <Title hit={hit} className="truncate" />
        {notes !== undefined && <span className="text-sm text-ink-soft">{t("hit.noteCount", { count: notes })}</span>}
      </span>
    </>
  );
}

function PlaceFace({ hit, extra }: FaceProps) {
  const emoji = extraText(extra, "emoji");
  return (
    <>
      {hit.imageUrl ? (
        <img src={hit.imageUrl} alt="" className="size-11 shrink-0 rounded-[0.85rem] bg-surface-3 object-cover" loading="lazy" decoding="async" draggable={false} />
      ) : emoji ? (
        <span aria-hidden className="grid size-11 shrink-0 place-items-center rounded-[0.85rem] bg-surface-2 text-2xl">
          {emoji}
        </span>
      ) : (
        <KindTile kind={hit.kind} />
      )}
      <span className="flex min-w-0 flex-1 flex-col justify-center">
        <Title hit={hit} className="truncate" />
        <Snippet hit={hit} lines={1} />
      </span>
    </>
  );
}

/** News, and any kind this client has no layout of its own for yet. */
function ArticleFace({ hit, extra }: FaceProps) {
  const source = extraText(extra, "source_name") || hit.owner?.displayName;
  return (
    <>
      <span className="flex min-w-0 flex-1 flex-col gap-0.5">
        <Title hit={hit} className="line-clamp-2 leading-snug" />
        <Meta>
          {source && <span className="truncate font-bold">{source}</span>}
          {source && hit.createdAt && <Dot />}
          <span>{ageLabel(hit.createdAt)}</span>
        </Meta>
        <Snippet hit={hit} />
      </span>
      {hit.imageUrl ? <img src={hit.imageUrl} alt="" className="size-16 shrink-0 rounded-xl bg-surface-3 object-cover" loading="lazy" decoding="async" draggable={false} /> : <KindTile kind={hit.kind} />}
    </>
  );
}

/** The outline of a result while a page of results loads. */
export function HitSkeleton() {
  return (
    <div className="flex gap-3 rounded-2xl border-2 border-line bg-surface p-3" aria-hidden>
      <div className="size-11 shrink-0 animate-pulse rounded-[0.85rem] bg-surface-3" />
      <div className="flex min-w-0 flex-1 flex-col gap-2 py-1">
        <div className="h-4 w-2/5 animate-pulse rounded-full bg-surface-3" />
        <div className="h-3 w-4/5 animate-pulse rounded-full bg-surface-3" />
      </div>
    </div>
  );
}
