import { engagement, toMillis } from "@tokuly/api";
import { ImageOff } from "lucide-react";
import type { MouseEvent, ReactNode } from "react";
import { resolveTargetPath, useOpenTargetPath } from "#/features/notifications";
import { StampImage } from "#/features/stamps";
import { cn } from "#/lib/cn";
import { compactAge } from "#/lib/format";
import { useTranslation } from "#/lib/use-translation";
import { AuthorAvatar, AuthorLink, authorName } from "./author";

const { CommentStatus, HoldReason } = engagement;

/** Where a comment lives in the app: its subject's page, opened at the comment. */
export function commentPath(
  subject: engagement.SubjectPreview | undefined,
  commentId: string,
): string | null {
  if (!subject?.available || !subject.path) return null;
  return `${subject.path}${subject.path.includes("?") ? "&" : "?"}comment=${commentId}`;
}

/** A link to a path the server handed out, opened inside the app. */
export function PathLink({
  path,
  className,
  children,
  label,
}: {
  path: string;
  className?: string;
  children: ReactNode;
  label?: string;
}) {
  const open = useOpenTargetPath();
  const target = resolveTargetPath(path);
  if (!target) return <span className={className}>{children}</span>;
  const onClick = (event: MouseEvent) => {
    // Modified clicks (new tab, new window) are the browser's to handle.
    if (
      target.type !== "internal" ||
      event.defaultPrevented ||
      event.button !== 0 ||
      event.metaKey ||
      event.ctrlKey ||
      event.shiftKey ||
      event.altKey
    )
      return;
    event.preventDefault();
    open(path);
  };
  return (
    <a
      href={target.type === "internal" ? target.href : target.url}
      onClick={onClick}
      className={className}
      aria-label={label}
    >
      {children}
    </a>
  );
}

const HOLD_REASON_KEYS: Partial<Record<engagement.HoldReason, string>> = {
  [HoldReason.REVIEW_ALL]: "reason.reviewAll",
  [HoldReason.BLOCKED_WORD]: "reason.blockedWord",
  [HoldReason.LINK]: "reason.link",
  [HoldReason.HIDDEN_BY_OWNER]: "reason.hiddenByOwner",
  [HoldReason.BLOCKED_COMMENTER]: "reason.blockedCommenter",
};

/**
 * A comment outside its thread — in "my comments" and in the owner's comment
 * manager: who wrote it, what it says, its state, and what it is on.
 */
export function ContextRow({
  entry,
  showAuthor,
  actions,
}: {
  entry: engagement.CommentInContext;
  showAuthor: boolean;
  actions?: ReactNode;
}) {
  const { t } = useTranslation("comments");
  const comment = entry.comment;
  if (!comment) return null;
  const subject = entry.subject;
  const path = commentPath(subject, comment.id);
  const reasonKey = HOLD_REASON_KEYS[comment.holdReason];

  return (
    <article
      data-testid="comment-row"
      className="sticker flex flex-col gap-3 rounded-card bg-surface p-4"
    >
      <div className="flex items-start gap-3">
        {showAuthor && (
          <AuthorLink author={comment.author} className="shrink-0">
            <AuthorAvatar author={comment.author} size="sm" />
          </AuthorLink>
        )}
        <div className="min-w-0 flex-1">
          <div className="flex flex-wrap items-center gap-x-2 gap-y-1 text-sm">
            {showAuthor && (
              <AuthorLink author={comment.author} className="max-w-full truncate font-black">
                {authorName(comment.author) || t("item.unknown")}
              </AuthorLink>
            )}
            <span className="text-xs text-ink-faint">
              {compactAge(toMillis(comment.createdAt))}
            </span>
            {comment.parentId && (
              <span className="rounded-full bg-surface-2 px-2 py-0.5 text-[11px] font-black text-ink-soft">
                {t("row.reply")}
              </span>
            )}
            {comment.status === CommentStatus.HELD && (
              <span className="rounded-full bg-sun px-2 py-0.5 text-[11px] font-black text-[#17122b]">
                {t("status.held")}
              </span>
            )}
            {comment.status === CommentStatus.HIDDEN && (
              <span className="rounded-full bg-surface-3 px-2 py-0.5 text-[11px] font-black text-ink-soft">
                {t("status.hidden")}
              </span>
            )}
            {reasonKey && <span className="text-xs font-bold text-ink-soft">{t(reasonKey)}</span>}
          </div>
          {comment.text && (
            <p className="mt-1 line-clamp-4 text-[15px] leading-relaxed break-words whitespace-pre-wrap">
              {comment.text}
            </p>
          )}
          {comment.stamp && (comment.stamp.imageUrl || comment.stamp.animationUrl) && (
            <StampImage stamp={comment.stamp} size={72} className="mt-1.5" />
          )}
          {comment.image?.url && (
            <img
              src={
                comment.image.variants.find((variant) => variant.size >= 240)?.url ??
                comment.image.url
              }
              alt=""
              loading="lazy"
              className="mt-1.5 max-h-32 rounded-xl object-cover"
            />
          )}
        </div>
      </div>

      <SubjectChip subject={subject} path={path} />
      {actions && <div className="flex flex-wrap items-center justify-end gap-2">{actions}</div>}
    </article>
  );
}

/** What a comment is on, as a small card leading there. */
function SubjectChip({
  subject,
  path,
}: {
  subject: engagement.SubjectPreview | undefined;
  path: string | null;
}) {
  const { t } = useTranslation("comments");
  const body = (
    <>
      {subject?.imageUrl ? (
        <img
          src={subject.imageUrl}
          alt=""
          loading="lazy"
          className="h-10 w-16 shrink-0 rounded-lg object-cover"
        />
      ) : (
        <span className="grid h-10 w-16 shrink-0 place-items-center rounded-lg bg-surface-3 text-ink-faint">
          <ImageOff className="size-4" />
        </span>
      )}
      <span className={cn("min-w-0 flex-1 truncate text-sm font-bold", !path && "text-ink-faint")}>
        {path ? subject?.title || t("row.untitled") : t("row.unavailable")}
      </span>
    </>
  );
  const classes = "flex items-center gap-3 rounded-2xl bg-surface-2 p-2";
  return path ? (
    <PathLink
      path={path}
      className={cn(classes, "hover:bg-surface-3")}
      label={t("row.open", { title: subject?.title ?? "" })}
    >
      {body}
    </PathLink>
  ) : (
    <div className={classes}>{body}</div>
  );
}
