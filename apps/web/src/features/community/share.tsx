import type { common } from "@tokuly/api";
import { Flag, Link2, Send } from "lucide-react";
import { lazy, Suspense, useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import type { MenuAction } from "#/components/ui/menu";
import { copyText, ReportDialog } from "#/features/engagement";
import { NS } from "./api";

// Loaded when someone first sends to a chat.
const ShareToChat = lazy(() =>
  import("#/features/chat").then((module) => ({ default: module.ShareToChat })),
);

/**
 * "Send in chat", "copy link" and "report" for a community page, for a menu
 * of the caller's own; render `sheets` once next to it.
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
}): { actions: MenuAction[]; sheets: React.ReactNode } {
  const { t } = useTranslation(NS);
  const [sharing, setSharing] = useState(false);
  const [shared, setShared] = useState(false);
  const [reporting, setReporting] = useState(false);
  const url = () => `${window.location.origin}${path}`;

  const actions: MenuAction[] = [];
  if (share) {
    actions.push(
      {
        label: t("share.chat"),
        icon: <Send />,
        onSelect: () => {
          setShared(true);
          setSharing(true);
        },
      },
      {
        label: t("share.copy"),
        icon: <Link2 />,
        onSelect: async () => {
          if (await copyText(url())) toast.success(t("share.copied"));
          else toast.error(t("share.copyFailed"));
        },
      },
    );
  }
  if (report)
    actions.push({ label: t("share.report"), icon: <Flag />, onSelect: () => setReporting(true) });

  return {
    actions,
    sheets: (
      <>
        {shared && (
          <Suspense fallback={null}>
            <ShareToChat
              open={sharing}
              onOpenChange={setSharing}
              share={{ url: url() }}
              title={title}
            />
          </Suspense>
        )}
        <ReportDialog
          subject={subject}
          open={reporting}
          onOpenChange={setReporting}
          label={title}
        />
      </>
    ),
  };
}
