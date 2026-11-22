import { Flag, Link2, MessageCircle, MoreHorizontal } from "lucide-react";
import { type ReactNode, useState } from "react";
import { useTranslation } from "react-i18next";
import { Menu, type MenuAction } from "#/components/ui/menu";
import { cn } from "#/lib/cn";
import type { Subject } from "./model";
import { ReportDialog } from "./report-dialog";
import { useCopySubjectLink, useShareToChat } from "./share-button";

export interface SubjectMenuOptions {
  /** What is being shared, for the "send in chat" sheet ("動画: 文化祭ダイジェスト"). */
  title?: string;
  /** What is being reported, shown in the report dialog (a title, "@handle", the start of a text). */
  label?: string;
  /** Leave the share entries out (something that has no page of its own, or is the viewer's eyes only). */
  share?: boolean;
  /** Leave the report entry out (the viewer's own content). */
  report?: boolean;
}

/**
 * The share and report entries every content menu ends with, for a `<Menu>`
 * (or any list of actions) a feature builds itself:
 *
 *   const more = useSubjectMenu({ type: common.SubjectType.VIDEO, id: video.id }, { title: video.title, label: video.title, report: !mine });
 *   <Menu trigger={…} actions={[...own, ...more.actions]} />
 *   {more.dialogs}
 *
 * `dialogs` must be rendered once next to the menu: it holds the "send in
 * chat" sheet and the report dialog the actions open.
 */
export function useSubjectMenu(
  subject: Subject,
  { title, label, share = true, report = true }: SubjectMenuOptions = {},
): { actions: MenuAction[]; dialogs: ReactNode } {
  const { t } = useTranslation("engagement");
  const copyLink = useCopySubjectLink(subject);
  const toChat = useShareToChat(subject, title);
  const [reporting, setReporting] = useState(false);
  const actions: MenuAction[] = [
    ...(share
      ? [
          { label: t("share.chat"), icon: <MessageCircle />, onSelect: toChat.open },
          { label: t("share.copy"), icon: <Link2 />, onSelect: () => void copyLink() },
        ]
      : []),
    ...(report
      ? [{ label: t("report.action"), icon: <Flag />, onSelect: () => setReporting(true) }]
      : []),
  ];
  return {
    actions,
    dialogs: (
      <>
        {toChat.sheet}
        {report && (
          <ReportDialog
            subject={subject}
            open={reporting}
            onOpenChange={setReporting}
            label={label}
          />
        )}
      </>
    ),
  };
}

/**
 * A "more" button with the share and report entries, for a header or a row
 * of actions that has no menu of its own:
 *
 *   <SubjectMoreButton subject={subject} title={playlist.name} label={playlist.name} report={!playlist.isOwner} />
 */
export function SubjectMoreButton({
  subject,
  className,
  ...options
}: SubjectMenuOptions & { subject: Subject; className?: string }) {
  const { t } = useTranslation("engagement");
  const more = useSubjectMenu(subject, options);
  if (more.actions.length === 0) return null;
  return (
    <>
      <Menu
        trigger={
          <button
            type="button"
            aria-label={t("more")}
            className={cn(
              "pressable sticker-sm grid size-11 shrink-0 place-items-center rounded-full bg-surface",
              className,
            )}
          >
            <MoreHorizontal className="size-5" />
          </button>
        }
        actions={more.actions}
      />
      {more.dialogs}
    </>
  );
}
