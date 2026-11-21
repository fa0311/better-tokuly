import type { notify } from "@tokuly/api";
import { Avatar } from "#/components/ui/avatar";
import { CategoryIcon } from "./category";

/**
 * The picture at the start of a notification: the face of whoever caused it
 * with the category's icon in the corner, or just the icon for system notices.
 */
export function NotificationFace({ entry }: { entry: Pick<notify.Notification, "actor" | "category"> }) {
  if (!entry.actor) return <CategoryIcon category={entry.category} size="md" />;
  return (
    <span className="relative shrink-0">
      <Avatar user={entry.actor} />
      <CategoryIcon category={entry.category} size="xs" className="absolute -right-1.5 -bottom-1.5" />
    </span>
  );
}
