import { admin, type common, type Timestamp, toDate } from "@tokuly/api";
import { CalendarDays, Megaphone, PartyPopper, Pin, Sparkles, Wrench } from "lucide-react";
import type { ComponentType, ReactNode } from "react";
import { useTranslation } from "react-i18next";
import { cn } from "#/lib/cn";
import { categoryKey } from "./api";
import { Markdown } from "./markdown-view";

/** What each category looks like: its sticker colour and icon. */
const CATEGORY_STYLE: Record<number, { tint: string; icon: ComponentType<{ className?: string }> }> = {
  [admin.AnnouncementCategory.UPDATE]: { tint: "bg-lime text-[#17122b]", icon: Sparkles },
  [admin.AnnouncementCategory.MAINTENANCE]: { tint: "bg-sun text-[#17122b]", icon: Wrench },
  [admin.AnnouncementCategory.EVENT]: { tint: "bg-pink text-white", icon: PartyPopper },
  [admin.AnnouncementCategory.IMPORTANT]: { tint: "bg-coral text-[#17122b]", icon: Megaphone },
};

const styleOf = (category: admin.AnnouncementCategory) => CATEGORY_STYLE[category] ?? (CATEGORY_STYLE[admin.AnnouncementCategory.UPDATE] as (typeof CATEGORY_STYLE)[number]);

/** The category of an announcement as a little sticker. */
export function CategoryBadge({ category, className }: { category: admin.AnnouncementCategory; className?: string }) {
  const { t } = useTranslation("announcements");
  const { tint, icon: Icon } = styleOf(category);
  return (
    <span className={cn("inline-flex h-7 shrink-0 items-center gap-1 rounded-full border-2 border-ink px-2.5 text-xs font-black whitespace-nowrap", tint, className)}>
      <Icon className="size-3.5" />
      {t(categoryKey(category))}
    </span>
  );
}

function PinnedBadge() {
  const { t } = useTranslation("announcements");
  return (
    <span className="inline-flex h-7 shrink-0 items-center gap-1 rounded-full border-2 border-ink bg-brand px-2.5 text-xs font-black whitespace-nowrap text-on-brand">
      <Pin className="size-3.5" />
      {t("pinned")}
    </span>
  );
}

function PublishedDate({ at }: { at: Timestamp | undefined }) {
  const { i18n } = useTranslation("announcements");
  const date = toDate(at);
  if (!date) return null;
  return (
    <time dateTime={date.toISOString()} className="inline-flex items-center gap-1 text-xs font-bold text-ink-soft">
      <CalendarDays className="size-3.5" />
      {new Intl.DateTimeFormat(i18n.language, { year: "numeric", month: "short", day: "numeric" }).format(date)}
    </time>
  );
}

/** Smallest stored rendition that still covers `pixels` on a 2x screen. */
function imageUrl(image: common.Image | undefined, pixels: number): string | undefined {
  if (!image?.url) return undefined;
  const fitting = image.variants.filter((variant) => variant.size >= pixels * 2).sort((a, b) => a.size - b.size)[0];
  return fitting?.url ?? image.url;
}

/** The parts of an announcement its card and article draw; both the published and the editor's form have them. */
export interface AnnouncementContent {
  category: admin.AnnouncementCategory;
  title: string;
  summary: string;
  body: string;
  image?: common.Image;
  pinned: boolean;
  publishedAt?: Timestamp;
}

/** An announcement as a card of the update log's list (the list wraps it in a link). */
export function AnnouncementCard({ announcement, className }: { announcement: AnnouncementContent; className?: string }) {
  const image = imageUrl(announcement.image, 640);
  return (
    <article className={cn("sticker flex h-full flex-col overflow-hidden rounded-card bg-surface", announcement.pinned && "bg-brand-soft", className)}>
      {image && (
        <div className="aspect-[16/7] w-full border-b-[2.5px] border-ink bg-surface-3" style={{ background: announcement.image?.dominantColor || undefined }}>
          <img src={image} alt="" className="size-full object-cover" loading="lazy" decoding="async" />
        </div>
      )}
      <div className="flex flex-1 flex-col gap-2 p-4">
        <div className="flex flex-wrap items-center gap-2">
          {announcement.pinned && <PinnedBadge />}
          <CategoryBadge category={announcement.category} />
          <PublishedDate at={announcement.publishedAt} />
        </div>
        <h2 className="text-lg leading-snug font-black">{announcement.title}</h2>
        {announcement.summary && <p className="line-clamp-3 text-sm leading-relaxed text-ink-soft">{announcement.summary}</p>}
      </div>
    </article>
  );
}

/** An announcement in full: header image, labels, title and the Markdown body. */
export function AnnouncementArticle({ announcement, footer }: { announcement: AnnouncementContent; footer?: ReactNode }) {
  const image = imageUrl(announcement.image, 720);
  return (
    <article className="sticker overflow-hidden rounded-card bg-surface">
      {image && (
        <div className="aspect-[16/7] w-full border-b-[2.5px] border-ink bg-surface-3" style={{ background: announcement.image?.dominantColor || undefined }}>
          <img src={image} alt="" className="size-full object-cover" decoding="async" />
        </div>
      )}
      <div className="p-5 sm:p-7">
        <div className="flex flex-wrap items-center gap-2">
          {announcement.pinned && <PinnedBadge />}
          <CategoryBadge category={announcement.category} />
          <PublishedDate at={announcement.publishedAt} />
        </div>
        <h1 className="mt-3 text-2xl leading-snug font-black sm:text-3xl">{announcement.title}</h1>
        <Markdown source={announcement.body} className="mt-5" />
        {footer}
      </div>
    </article>
  );
}
