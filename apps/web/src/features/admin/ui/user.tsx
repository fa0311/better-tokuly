import { Link } from "@tanstack/react-router";
import type { common } from "@tokuly/api";
import { BadgeCheck } from "lucide-react";
import { useTranslation } from "react-i18next";
import { Avatar } from "#/components/ui/avatar";
import { cn } from "#/lib/cn";

/**
 * A person in a table cell or fact sheet: avatar, name and handle, linking to
 * their page in the console. `fallback` is shown when there is nobody
 * (automatic actions, erased accounts).
 */
export function UserCell({
  user,
  fallback,
  link = true,
  className,
}: {
  user?: common.UserSummary;
  fallback?: string;
  link?: boolean;
  className?: string;
}) {
  const { t } = useTranslation("admin");
  if (!user) return <span className="whitespace-nowrap text-ink-faint">{fallback ?? "—"}</span>;
  const body = (
    <>
      <Avatar user={user} size="xs" />
      <span className="min-w-0">
        <span className="flex items-center gap-1">
          <span className="truncate font-bold">
            {user.deleted ? t("common.deletedUser") : user.displayName || user.handle}
          </span>
          {user.verified && (
            <BadgeCheck className="size-4 shrink-0 text-brand" aria-label={t("users.verified")} />
          )}
        </span>
        {!user.deleted && (
          <span className="block truncate text-xs text-ink-soft">@{user.handle}</span>
        )}
      </span>
    </>
  );
  const classes = cn("inline-flex max-w-full min-w-0 items-center gap-2 align-middle", className);
  return link && !user.deleted ? (
    <Link
      to="/admin/users/$userId"
      params={{ userId: user.id }}
      className={cn(classes, "rounded-lg hover:underline")}
    >
      {body}
    </Link>
  ) : (
    <span className={classes}>{body}</span>
  );
}
