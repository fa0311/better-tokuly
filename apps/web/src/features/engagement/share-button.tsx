import { useQueryClient } from "@tanstack/react-query";
import { engagement } from "@tokuly/api";
import { Link2, MessageCircle, Share2, Upload } from "lucide-react";
import { lazy, Suspense, useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { Menu } from "#/components/ui/menu";
import { cn } from "#/lib/cn";
import { errorMessage } from "#/lib/errors";
import { recordShare, useEngagement } from "./api";
import { compactCount, type Subject } from "./model";

export interface ShareButtonProps {
  subject: Subject;
  /** Title handed to the share sheet (a video's title, "@handle のノート"). */
  title?: string;
  /** Show how often the subject was shared next to the icon. */
  showCount?: boolean;
  className?: string;
}

// Loaded when someone first sends to a chat: chat itself uses this feature (reports, links).
const ShareToChat = lazy(() =>
  import("#/features/chat").then((module) => ({ default: module.ShareToChat })),
);

/**
 * "Send to a friend" for any subject, for menus that have their own share
 * entry: render `sheet` once and call `open()` from the menu item.
 *
 *   const toChat = useShareToChat(subject, video.title);
 *   …{ label: t("engagement:share.chat"), onSelect: toChat.open }…
 *   {toChat.sheet}
 */
export function useShareToChat(subject: Subject, title?: string) {
  const [open, setOpen] = useState(false);
  // Kept mounted after the first opening, so the sheet can animate out.
  const [used, setUsed] = useState(false);
  return {
    open: () => {
      setUsed(true);
      setOpen(true);
    },
    sheet: used ? (
      <Suspense fallback={null}>
        <ShareToChat open={open} onOpenChange={setOpen} share={{ subject }} title={title} />
      </Suspense>
    ) : null,
  };
}

/** Puts text on the clipboard, also where the async clipboard API is unavailable (older browsers, a lost focus). */
export async function copyText(text: string): Promise<boolean> {
  try {
    await navigator.clipboard.writeText(text);
    return true;
  } catch {
    const field = document.createElement("textarea");
    field.value = text;
    field.setAttribute("readonly", "");
    field.style.position = "fixed";
    field.style.opacity = "0";
    document.body.append(field);
    field.select();
    const copied = document.execCommand("copy");
    field.remove();
    return copied;
  }
}

/** Records a share of `subject` and copies its link. For menus that offer "copy link" themselves. */
export function useCopySubjectLink(subject: Subject): () => Promise<void> {
  const { t } = useTranslation("engagement");
  const queryClient = useQueryClient();
  return async () => {
    try {
      const url = await recordShare(queryClient, subject, engagement.ShareTarget.LINK_COPY);
      if (await copyText(url)) toast.success(t("share.copied"));
      else toast.error(t("share.copyFailed"));
    } catch (error) {
      toast.error(errorMessage(error));
    }
  };
}

/**
 * Shares a subject: sent to a friend in chat, copied as a link, or handed to
 * the device's share sheet where there is one. Every way is recorded, which
 * is what the share count shows.
 */
export function ShareButton({ subject, title, showCount, className }: ShareButtonProps) {
  const { t, i18n } = useTranslation("engagement");
  const queryClient = useQueryClient();
  const { data } = useEngagement(subject);
  const copyLink = useCopySubjectLink(subject);
  const toChat = useShareToChat(subject, title);
  const canShare = typeof navigator !== "undefined" && typeof navigator.share === "function";

  const shareOut = async () => {
    try {
      const url = await recordShare(queryClient, subject, engagement.ShareTarget.EXTERNAL);
      await navigator.share({ url, title });
    } catch (error) {
      // Closing the share sheet is not an error worth a toast.
      if (!(error instanceof DOMException && error.name === "AbortError"))
        toast.error(errorMessage(error));
    }
  };

  return (
    <>
      <Menu
        trigger={
          <button
            type="button"
            aria-label={t("share.label")}
            className={cn(
              "inline-flex h-9 min-w-9 items-center justify-center gap-1.5 rounded-full px-2 text-sm font-black text-ink-soft transition-colors hover:bg-surface-2 hover:text-ink",
              className,
            )}
          >
            <Share2 className="size-[18px]" />
            {showCount && data && data.shareCount > 0 && (
              <span className="tabular-nums">{compactCount(data.shareCount, i18n.language)}</span>
            )}
          </button>
        }
        actions={[
          { label: t("share.chat"), icon: <MessageCircle />, onSelect: toChat.open },
          { label: t("share.copy"), icon: <Link2 />, onSelect: () => void copyLink() },
          ...(canShare
            ? [{ label: t("share.other"), icon: <Upload />, onSelect: () => void shareOut() }]
            : []),
        ]}
      />
      {toChat.sheet}
    </>
  );
}
