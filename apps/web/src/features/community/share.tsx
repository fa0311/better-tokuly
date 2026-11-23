import type { common } from "@tokuly/api";
import type { ReactNode } from "react";
import type { MenuAction } from "#/components/ui/menu";
import { useSubjectMenu } from "#/features/engagement";

/**
 * "Send in chat", "copy link" and "report" for a community page, for a menu
 * of the caller's own; render `sheets` once next to it. Every share is
 * counted for the subject.
 *
 * The link is this app's address of the page (`/g/…`), not the path the
 * community domain publishes, which names a video channel here.
 */
export function useShareMenu({
  path,
  title,
  subject,
  report = true,
  share = true,
}: {
  /** Path of the page in this app. */
  path: string;
  title: string;
  subject: Pick<common.SubjectRef, "type" | "id">;
  /** Offer "report" (not for the viewer's own content). */
  report?: boolean;
  /** Offer the share entries (not for what only members can open… which still works for members). */
  share?: boolean;
}): { actions: MenuAction[]; sheets: ReactNode } {
  const menu = useSubjectMenu(subject, { title, label: title, share, report, path });
  return { actions: menu.actions, sheets: menu.dialogs };
}
