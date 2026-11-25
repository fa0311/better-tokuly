import type { notify } from "@tokuly/api";
import { UserRound } from "lucide-react";
import { useTranslation } from "react-i18next";
import { Avatar } from "#/components/ui/avatar";
import { CategoryIcon } from "./category";

/**
 * The picture at the start of a notification: the face of whoever caused it
 * with the category's icon in the corner, a neutral 「だれか」 face when the
 * server does not say who (the actor is unnamed), or just the icon for
 * system notices.
 */
export function NotificationFace({
  entry,
}: {
  entry: Pick<notify.Notification, "actor" | "actorUnnamed" | "category">;
}) {
  const { t } = useTranslation("notifications");
  if (!entry.actor && !entry.actorUnnamed)
    return <CategoryIcon category={entry.category} size="md" />;
  return (
    <span className="relative shrink-0">
      {entry.actor ? (
        <Avatar user={entry.actor} />
      ) : (
        <span
          role="img"
          aria-label={t("someone")}
          title={t("someone")}
          data-testid="someone-face"
          className="grid size-11 place-items-center rounded-[0.85rem] bg-surface-3 text-ink-soft"
        >
          <UserRound className="size-6" aria-hidden />
        </span>
      )}
      <CategoryIcon
        category={entry.category}
        size="xs"
        className="absolute -right-1.5 -bottom-1.5"
      />
    </span>
  );
}
