import { useQueryClient } from "@tanstack/react-query";
import type { chat } from "@tokuly/api";
import { CornerUpRight } from "lucide-react";
import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { Button } from "#/components/ui/button";
import { Dialog } from "#/components/ui/dialog";
import { errorMessage } from "./errors";
import { messageClient } from "./api";
import { storeCreated } from "./messages";
import { type Target, TargetPicker, useResolveTargets } from "./target-picker";

/** A forward reaches at most this many conversations. */
const MAX_TARGETS = 10;

/** "Forward": copies a message into the conversations picked here. */
export function ForwardDialog({ message, onClose }: { message: chat.Message | null; onClose(): void }) {
  const { t } = useTranslation("chat");
  const queryClient = useQueryClient();
  const resolve = useResolveTargets();
  const [targets, setTargets] = useState<Target[]>([]);
  const [sending, setSending] = useState(false);
  const open = message !== null;
  useEffect(() => {
    if (!open) setTargets([]);
  }, [open]);

  const forward = async () => {
    if (!message) return;
    setSending(true);
    try {
      const conversationIds = await resolve(targets);
      const response = await messageClient().forwardMessages({ messageIds: [message.id], conversationIds, clientId: crypto.randomUUID() });
      for (const copy of response.messages) storeCreated(queryClient, copy);
      toast.success(t("forward.done", { count: conversationIds.length }));
      onClose();
    } catch (error) {
      toast.error(errorMessage(error));
    } finally {
      setSending(false);
    }
  };

  return (
    <Dialog
      open={open}
      onOpenChange={(next) => !next && onClose()}
      title={t("forward.title")}
      description={t("forward.lead", { count: MAX_TARGETS })}
      className="sm:h-[min(40rem,calc(100dvh-4rem))]"
      footer={
        <Button onClick={forward} loading={sending} disabled={targets.length === 0} icon={<CornerUpRight className="size-4" />}>
          {t("forward.send", { count: targets.length })}
        </Button>
      }
    >
      <TargetPicker selected={targets} onChange={setTargets} limit={MAX_TARGETS} />
    </Dialog>
  );
}
