import { Link } from "@tanstack/react-router";
import { type common, community, media } from "@tokuly/api";
import {
  categoryOf,
  imageUrl,
  pseudonymName,
  ROLE_KEYS,
  safeColor,
  splitMentions,
} from "@tokuly/core/community";
import { EyeOff, ImagePlus, Lock, Users, X } from "lucide-react";
import { type ReactNode, useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { Avatar } from "#/components/ui/avatar";
import { Chip } from "#/components/ui/card";
import { Dialog } from "#/components/ui/dialog";
import { Spinner } from "#/components/ui/spinner";
import { EmojiText } from "#/features/emoji";
import { useAccountTimeZone } from "#/lib/account-time";
import { cn } from "#/lib/cn";
import { errorMessage } from "#/lib/errors";
import { compactAge } from "#/lib/format";
import { uploadAndProcess } from "#/lib/upload";
import { NS } from "./api";

// ───────────── Words for enums ─────────────

// ───────────── Time ─────────────

/** "12月1日(火) 19:00" on the account's clock. */
export function useWhen(): (ms: number, options?: { date?: boolean; time?: boolean }) => string {
  const zone = useAccountTimeZone();
  const { i18n } = useTranslation();
  return (ms, { date = true, time = true } = {}) =>
    new Intl.DateTimeFormat(i18n.language, {
      timeZone: zone,
      ...(date ? { month: "short", day: "numeric", weekday: "short" } : {}),
      ...(time ? { hour: "2-digit", minute: "2-digit", hourCycle: "h23" } : {}),
    }).format(new Date(ms));
}

/** A duration in words: "30秒", "10分", "7日", "ずっと" (0). */
export function useDurationLabel(): (seconds: number) => string {
  const { t } = useTranslation(NS);
  return (seconds) =>
    seconds === 0
      ? t("duration.forever")
      : seconds < 60
        ? t("duration.seconds", { count: seconds })
        : seconds < 3600
          ? t("duration.minutes", { count: Math.round(seconds / 60) })
          : seconds < 86_400
            ? t("duration.hours", { count: Math.round(seconds / 3600) })
            : t("duration.days", { count: Math.round(seconds / 86_400) });
}

/** A compact age that keeps itself current ("3分前"). */
export function Age({ ms }: { ms: number }) {
  const [, tick] = useState(0);
  useEffect(() => {
    const timer = window.setInterval(() => tick((n) => n + 1), 60_000);
    return () => window.clearInterval(timer);
  }, []);
  return <time dateTime={new Date(ms).toISOString()}>{compactAge(ms)}</time>;
}

// ───────────── Communities ─────────────

const ICON_SIZES = {
  sm: { box: "size-9 rounded-xl text-sm", pixels: 36 },
  md: { box: "size-12 rounded-2xl text-lg", pixels: 48 },
  lg: { box: "size-20 rounded-[1.4rem] text-3xl", pixels: 80 },
} as const;

/** A community's icon: its picture, or the emoji of its category. */
export function CommunityIcon({
  community: c,
  size = "md",
  className,
}: {
  community: Pick<community.Community, "icon" | "category" | "name">;
  size?: keyof typeof ICON_SIZES;
  className?: string;
}) {
  const { box, pixels } = ICON_SIZES[size];
  const url = imageUrl(c.icon, pixels);
  return (
    <span
      className={cn(
        "sticker-sm grid shrink-0 place-items-center overflow-hidden bg-surface-2",
        box,
        className,
      )}
      style={c.icon?.dominantColor ? { background: c.icon.dominantColor } : undefined}
    >
      {url ? (
        <img src={url} alt="" className="size-full object-cover" loading="lazy" draggable={false} />
      ) : (
        <span aria-hidden>{categoryOf(c.category).emoji}</span>
      )}
    </span>
  );
}

/** "🔒 承認制" / "🙈 招待のみ"; nothing for a public community. */
export function VisibilityChip({ visibility }: { visibility: community.CommunityVisibility }) {
  const { t } = useTranslation(NS);
  if (visibility === community.CommunityVisibility.RESTRICTED)
    return (
      <Chip>
        <Lock className="size-3" />
        {t("visibility.restricted")}
      </Chip>
    );
  if (visibility === community.CommunityVisibility.HIDDEN)
    return (
      <Chip>
        <EyeOff className="size-3" />
        {t("visibility.hidden")}
      </Chip>
    );
  return null;
}

export function MemberCount({ count }: { count: number }) {
  const { t } = useTranslation(NS);
  return (
    <span className="inline-flex items-center gap-1 text-xs font-bold text-ink-soft">
      <Users className="size-3.5" />
      {t("memberCount", { count, formatted: count.toLocaleString() })}
    </span>
  );
}

/** A community in a grid of the directory: banner, icon, what it is, how many are in. */
export function CommunityCard({
  community: c,
  footer,
}: {
  community: community.Community;
  /** A line under the description (the friends who are in it). */
  footer?: ReactNode;
}) {
  const { t } = useTranslation(NS);
  const banner = imageUrl(c.banner, 320);
  const category = categoryOf(c.category);
  return (
    <Link
      to="/g/$handle"
      params={{ handle: c.handle }}
      className="sticker pressable flex flex-col overflow-hidden rounded-card bg-surface text-left"
    >
      <div
        className="h-16 bg-brand-soft"
        style={
          banner
            ? {
                backgroundImage: `url("${banner}")`,
                backgroundSize: "cover",
                backgroundPosition: "center",
              }
            : undefined
        }
      />
      <div className="flex flex-1 flex-col gap-1.5 px-4 pb-4">
        <div className="-mt-6 flex items-end gap-2">
          <CommunityIcon community={c} />
          {c.viewer?.status === community.MembershipStatus.MEMBER && (
            <Chip className="mb-0.5 bg-lime text-[#17122b]">{t("joined")}</Chip>
          )}
        </div>
        <p className="line-clamp-1 font-black leading-snug">
          <EmojiText text={c.name} />
        </p>
        {c.description && (
          <p className="line-clamp-2 text-sm leading-snug text-ink-soft">
            <EmojiText text={c.description} />
          </p>
        )}
        <div className="mt-auto flex flex-wrap items-center gap-x-2 gap-y-1 pt-1">
          <Chip>
            <span aria-hidden>{category.emoji}</span>
            {t(`category.${category.key}`)}
          </Chip>
          <VisibilityChip visibility={c.visibility} />
          <MemberCount count={c.memberCount} />
        </div>
        {footer}
      </div>
    </Link>
  );
}

/** A community as a row of a list (my communities, a profile). */
export function CommunityRow({
  community: c,
  trailing,
  subtitle,
}: {
  community: community.Community;
  trailing?: ReactNode;
  subtitle?: ReactNode;
}) {
  const { t } = useTranslation(NS);
  return (
    <Link
      to="/g/$handle"
      params={{ handle: c.handle }}
      className="flex items-center gap-3 rounded-2xl px-3 py-2.5 transition-colors hover:bg-surface-2"
    >
      <span className="relative">
        <CommunityIcon community={c} />
        {c.viewer?.hasUnread && (
          <span
            role="img"
            aria-label={t("unread")}
            className="absolute -top-1 -right-1 size-3.5 rounded-full border-2 border-surface bg-pink"
          />
        )}
      </span>
      <span className="min-w-0 flex-1">
        <span className="block truncate font-bold">
          <EmojiText text={c.name} />
        </span>
        <span className="block truncate text-sm text-ink-soft">
          {subtitle ?? <MemberCount count={c.memberCount} />}
        </span>
      </span>
      {trailing}
    </Link>
  );
}

export function RoleChip({ role }: { role: community.MemberRole }) {
  const { t } = useTranslation(NS);
  if (role <= community.MemberRole.MEMBER) return null;
  const tint =
    role === community.MemberRole.OWNER
      ? "bg-sun text-[#17122b]"
      : role === community.MemberRole.ADMIN
        ? "bg-sky text-[#17122b]"
        : "bg-mint text-[#17122b]";
  return <Chip className={tint}>{t(`role.${ROLE_KEYS[role]}`)}</Chip>;
}

// ───────────── Posts ─────────────

/**
 * Who wrote a post, as the server names them: a member, or — in an
 * anonymous channel — the pseudonym of that thread. A pseudonym is never
 * tied to an account here; "you" is shown only where the server says the
 * post is the viewer's own.
 */
export function PostAuthor({
  author,
  pseudonym,
  mine,
  createdAtMs,
  edited,
  badges,
  size = "md",
}: {
  author?: common.UserSummary;
  pseudonym?: community.Pseudonym;
  /** `viewer.mine` of the post. */
  mine?: boolean;
  createdAtMs: number;
  edited?: boolean;
  /** Chips after the name (thread author, role). */
  badges?: ReactNode;
  size?: "sm" | "md";
}) {
  const { t, i18n } = useTranslation(NS);
  const meta = (
    <span className="flex flex-wrap items-center gap-x-1.5 text-xs text-ink-faint">
      <Age ms={createdAtMs} />
      {edited && <span>· {t("post.edited")}</span>}
    </span>
  );
  if (pseudonym) {
    const color = safeColor(pseudonym.color);
    return (
      <div className="flex min-w-0 items-center gap-2.5">
        <span
          aria-hidden
          className={cn(
            "grid shrink-0 place-items-center rounded-[0.85rem] border-2 border-ink bg-surface-3",
            size === "sm" ? "size-8 text-base" : "size-11 text-2xl",
          )}
          style={color ? { background: color } : undefined}
        >
          {pseudonym.emoji || "🫥"}
        </span>
        <div className="min-w-0">
          <div className="flex flex-wrap items-center gap-1.5">
            <span className="truncate font-bold">{pseudonymName(pseudonym, i18n.language)}</span>
            <Chip>{t("anonymous.chip")}</Chip>
            {mine && <Chip className="bg-brand-soft text-brand-strong">{t("post.mine")}</Chip>}
            {badges}
          </div>
          {meta}
        </div>
      </div>
    );
  }
  const name = author?.deleted
    ? t("post.deletedUser")
    : author?.displayName || author?.handle || t("post.deletedUser");
  const body = (
    <>
      <Avatar user={author} size={size === "sm" ? "sm" : "md"} />
      <div className="min-w-0">
        <div className="flex flex-wrap items-center gap-1.5">
          <span className="truncate font-bold">
            <EmojiText text={name} />
          </span>
          {badges}
        </div>
        {meta}
      </div>
    </>
  );
  return author && !author.deleted && author.handle ? (
    <Link
      to="/u/$handle"
      params={{ handle: author.handle }}
      className="flex min-w-0 items-center gap-2.5"
    >
      {body}
    </Link>
  ) : (
    <div className="flex min-w-0 items-center gap-2.5">{body}</div>
  );
}

/** A post's text: line breaks kept, custom emoji drawn, members' `@handle`s linked. */
export function PostBody({
  body,
  mentions,
  className,
}: {
  body: string;
  mentions: community.Mention[];
  className?: string;
}) {
  if (!body) return null;
  return (
    <p className={cn("whitespace-pre-wrap break-words leading-relaxed", className)}>
      {splitMentions(body, mentions).map((part, index) =>
        part.handle ? (
          <Link
            key={index}
            to="/u/$handle"
            params={{ handle: part.handle }}
            className="font-bold text-brand"
          >
            {part.text}
          </Link>
        ) : (
          <EmojiText key={index} text={part.text} />
        ),
      )}
    </p>
  );
}

/** The pictures of a post; a tap shows one large. */
export function PostImages({ images }: { images: common.Image[] }) {
  const { t } = useTranslation(NS);
  const [open, setOpen] = useState<common.Image>();
  if (images.length === 0) return null;
  return (
    <>
      <div className={cn("grid gap-1.5", images.length > 1 ? "grid-cols-2" : "grid-cols-1")}>
        {images.map((image, index) => (
          <button
            key={image.assetId || index}
            type="button"
            onClick={() => setOpen(image)}
            aria-label={t("post.openImage", { index: index + 1 })}
            className="overflow-hidden rounded-2xl border-2 border-line bg-surface-2"
            style={image.dominantColor ? { background: image.dominantColor } : undefined}
          >
            <img
              src={imageUrl(image, 320)}
              alt=""
              loading="lazy"
              className={cn(
                "w-full object-cover",
                images.length > 1 ? "aspect-square" : "max-h-96",
              )}
            />
          </button>
        ))}
      </div>
      <Dialog
        open={Boolean(open)}
        onOpenChange={(next) => !next && setOpen(undefined)}
        title={t("post.image")}
        className="sm:w-[min(56rem,calc(100vw-2rem))]"
      >
        {open && <img src={open.url} alt="" className="mx-auto max-h-[70dvh] rounded-2xl" />}
      </Dialog>
    </>
  );
}

export function TagList({
  tags,
  onPick,
  className,
}: {
  tags: string[];
  onPick?(tag: string): void;
  className?: string;
}) {
  if (tags.length === 0) return null;
  return (
    <div className={cn("flex flex-wrap gap-1.5", className)}>
      {tags.map((tag) =>
        onPick ? (
          <button
            key={tag}
            type="button"
            onClick={() => onPick(tag)}
            className="inline-flex h-6 items-center rounded-full bg-surface-2 px-2.5 text-xs font-bold text-ink-soft hover:bg-surface-3"
          >
            #{tag}
          </button>
        ) : (
          <Chip key={tag}>#{tag}</Chip>
        ),
      )}
    </div>
  );
}

// ───────────── Uploads ─────────────

export interface PendingImage {
  /** Local key of the picture. */
  key: string;
  preview: string;
  /** Set once the upload finished. */
  assetId?: string;
}

/**
 * The pictures being attached to a post. Every picture is uploaded afresh
 * for the post it goes into and never offered again afterwards — an upload
 * that also appears somewhere else under a name would tie an anonymous post
 * to its author.
 */
export function useImageAttachments(max: number) {
  const { t } = useTranslation(NS);
  const [images, setImages] = useState<PendingImage[]>([]);
  const aborts = useRef(new Map<string, AbortController>());

  useEffect(() => {
    const controllers = aborts.current;
    return () => {
      for (const controller of controllers.values()) controller.abort();
    };
  }, []);

  const add = (files: FileList | File[]) => {
    const room = max - images.length;
    const chosen = [...files].filter((file) => file.type.startsWith("image/"));
    if (chosen.length > room) toast.error(t("compose.tooManyImages", { count: max }));
    for (const file of chosen.slice(0, Math.max(0, room))) {
      const key = crypto.randomUUID();
      const preview = URL.createObjectURL(file);
      const abort = new AbortController();
      aborts.current.set(key, abort);
      setImages((current) => [...current, { key, preview }]);
      uploadAndProcess(file, { purpose: media.AssetPurpose.NOTE_IMAGE, signal: abort.signal })
        .then((asset) =>
          setImages((current) =>
            current.map((image) => (image.key === key ? { ...image, assetId: asset.id } : image)),
          ),
        )
        .catch((error) => {
          if (abort.signal.aborted) return;
          toast.error(errorMessage(error, { namespace: NS }));
          setImages((current) => current.filter((image) => image.key !== key));
        })
        .finally(() => aborts.current.delete(key));
    }
  };
  const remove = (key: string) => {
    aborts.current.get(key)?.abort();
    setImages((current) => current.filter((image) => image.key !== key));
  };
  return {
    images,
    add,
    remove,
    reset: () => setImages([]),
    uploading: images.some((image) => !image.assetId),
    assetIds: images.flatMap((image) => (image.assetId ? [image.assetId] : [])),
    full: images.length >= max,
  };
}

export function ImageAttachmentRow({
  images,
  onRemove,
}: {
  images: PendingImage[];
  onRemove(key: string): void;
}) {
  const { t } = useTranslation(NS);
  if (images.length === 0) return null;
  return (
    <div className="flex flex-wrap gap-2">
      {images.map((image) => (
        <div key={image.key} className="relative">
          <img
            src={image.preview}
            alt=""
            className={cn(
              "size-20 rounded-2xl border-2 border-line object-cover",
              !image.assetId && "opacity-50",
            )}
          />
          {!image.assetId && (
            <span className="absolute inset-0 grid place-items-center">
              <Spinner className="size-6" />
            </span>
          )}
          <button
            type="button"
            onClick={() => onRemove(image.key)}
            aria-label={t("compose.removeImage")}
            className="absolute -top-2 -right-2 grid size-7 place-items-center rounded-full border-2 border-ink bg-surface"
          >
            <X className="size-4" />
          </button>
        </div>
      ))}
    </div>
  );
}

/** An icon button that opens the file chooser for pictures. */
export function ImagePickButton({
  onPick,
  disabled,
  label,
}: {
  onPick(files: FileList): void;
  disabled?: boolean;
  label: string;
}) {
  const input = useRef<HTMLInputElement>(null);
  return (
    <>
      <input
        ref={input}
        type="file"
        accept="image/*"
        multiple
        hidden
        onChange={(event) => {
          if (event.target.files?.length) onPick(event.target.files);
          event.target.value = "";
        }}
      />
      <button
        type="button"
        disabled={disabled}
        aria-label={label}
        onClick={() => input.current?.click()}
        className="grid size-10 place-items-center rounded-full text-ink-soft hover:bg-surface-2 disabled:opacity-40"
      >
        <ImagePlus className="size-5" />
      </button>
    </>
  );
}

/** A line of small print that explains something; `tone` picks the colour. */
export function Notice({
  icon,
  children,
  tone = "soft",
  className,
}: {
  icon?: ReactNode;
  children: ReactNode;
  tone?: "soft" | "warn";
  className?: string;
}) {
  return (
    <div
      className={cn(
        "flex items-start gap-2.5 rounded-2xl px-3.5 py-3 text-sm leading-relaxed",
        tone === "warn" ? "bg-sun text-[#17122b]" : "bg-surface-2 text-ink-soft",
        className,
      )}
    >
      {icon && <span className="mt-0.5 shrink-0 [&>svg]:size-4">{icon}</span>}
      <div className="min-w-0 flex-1">{children}</div>
    </div>
  );
}

/** A heading of a block inside a page. */
export function SectionTitle({ children, action }: { children: ReactNode; action?: ReactNode }) {
  return (
    <div className="mt-6 mb-2 flex items-center gap-2 px-1">
      <h2 className="min-w-0 flex-1 text-base font-black">{children}</h2>
      {action}
    </div>
  );
}
