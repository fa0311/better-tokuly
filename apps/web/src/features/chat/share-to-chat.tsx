import { useQueryClient } from "@tanstack/react-query";
import { type common, engagement } from "@tokuly/api";
import { Send } from "lucide-react";
import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { Button } from "#/components/ui/button";
import { Dialog } from "#/components/ui/dialog";
import { api } from "#/lib/api";
import { messageClient } from "./api";
import { errorMessage } from "./errors";
import { storeCreated } from "./messages";
import { type Target, TargetPicker, useResolveTargets } from "./target-picker";

/** One share reaches at most this many conversations. */
const MAX_TARGETS = 10;

export interface ShareToChatProps {
  /** The sheet is open. The caller owns this and closes it from `onOpenChange`. */
  open: boolean;
  onOpenChange(open: boolean): void;
  /**
   * What to send: a link (`{ url }`), or something of the app by reference
   * (`{ subject }`: a note, a video, a place…), which is sent as its link and
   * counted as a share of it. Chat shows either with its preview.
   */
  share: { url: string } | { subject: Pick<common.SubjectRef, "type" | "id"> };
  /** Reminds the person what they are sending ("ノート: 放課後の空"); optional. */
  title?: string;
  /** Called once it was sent, with the conversations it went to. */
  onShared?(conversationIds: string[]): void;
}

/** The link to send: the URL itself, or the subject's page (recording the share). */
async function linkOf(share: ShareToChatProps["share"]): Promise<string> {
  if ("url" in share) return share.url;
  const response = await api()
    .service(engagement.EngagementService)
    .recordShare({ subject: share.subject, target: engagement.ShareTarget.CHAT });
  return response.url;
}

/**
 * "Send to a friend": pick conversations (or friends), add a note if you
 * like, and the link goes into each of them as a message.
 *
 *   <ShareToChat open={sharing} onOpenChange={setSharing} share={{ subject: { type: common.SubjectType.NOTE, id: note.id } }} title={note.text} />
 */
export function ShareToChat({ open, onOpenChange, share, title, onShared }: ShareToChatProps) {
  const { t } = useTranslation("chat");
  const queryClient = useQueryClient();
  const resolve = useResolveTargets();
  const [targets, setTargets] = useState<Target[]>([]);
  const [note, setNote] = useState("");
  const [sending, setSending] = useState(false);
  useEffect(() => {
    if (open) return;
    setTargets([]);
    setNote("");
  }, [open]);

  const submit = async () => {
    setSending(true);
    try {
      const [url, conversationIds] = await Promise.all([linkOf(share), resolve(targets)]);
      const text = note.trim() ? `${note.trim()}\n${url}` : url;
      const results = await Promise.allSettled(
        conversationIds.map(async (conversationId) => {
          const { message } = await messageClient().sendMessage({
            conversationId,
            clientId: crypto.randomUUID(),
            text,
          });
          if (message) storeCreated(queryClient, message);
          return conversationId;
        }),
      );
      const sent = results.flatMap((result) =>
        result.status === "fulfilled" ? [result.value] : [],
      );
      const failed = results.find((result) => result.status === "rejected");
      if (failed) toast.error(errorMessage(failed.reason));
      if (sent.length > 0) {
        toast.success(t("share.done", { count: sent.length }));
        onShared?.(sent);
        onOpenChange(false);
      }
    } catch (error) {
      toast.error(errorMessage(error));
    } finally {
      setSending(false);
    }
  };

  return (
    <Dialog
      open={open}
      onOpenChange={onOpenChange}
      title={t("share.title")}
      description={title}
      className="sm:h-[min(42rem,calc(100dvh-4rem))]"
      footer={
        <Button
          onClick={submit}
          loading={sending}
          disabled={targets.length === 0}
          icon={<Send className="size-4" />}
        >
          {t("share.send", { count: targets.length })}
        </Button>
      }
    >
      <div className="flex flex-col gap-3">
        <input
          value={note}
          onChange={(event) => setNote(event.target.value)}
          placeholder={t("share.note")}
          aria-label={t("share.note")}
          maxLength={500}
          className="h-11 rounded-control border-2 border-line bg-surface px-4 text-[15px] outline-none placeholder:text-ink-faint focus:border-brand"
        />
        <TargetPicker selected={targets} onChange={setTargets} limit={MAX_TARGETS} />
      </div>
    </Dialog>
  );
}
