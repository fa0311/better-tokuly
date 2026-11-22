import type { chat } from "@tokuly/api";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { ConfirmDialog } from "#/components/ui/confirm";
import { Dialog } from "#/components/ui/dialog";
import { ListRow } from "#/components/ui/page";
import { SwitchRow } from "#/components/ui/switch";
import { type MuteFor, useConversationActions } from "./actions";
import { isGroup } from "./model";
import { titleOf } from "./people";

const HOUR = 3_600_000;
const MUTE_CHOICES: { key: string; forMs: MuteFor }[] = [
  { key: "hour", forMs: HOUR },
  { key: "eightHours", forMs: 8 * HOUR },
  { key: "day", forMs: 24 * HOUR },
  { key: "forever", forMs: null },
];

/** Which of the dialogs about a whole conversation is open. */
export type ConversationDialog = { type: "mute" | "hide" | "leave"; conversation: chat.Conversation } | null;

/**
 * The dialogs that go with the actions on a conversation: how long to mute,
 * hide (optionally forgetting the history), leave a group. The caller keeps
 * `dialog` and is told when the conversation is gone from the lists.
 */
export function ConversationDialogs({ dialog, myId, onClose, onGone }: { dialog: ConversationDialog; myId: string; onClose(): void; onGone?(conversation: chat.Conversation): void }) {
  const { t } = useTranslation("chat");
  const actions = useConversationActions();
  const [clearHistory, setClearHistory] = useState(false);
  const [busy, setBusy] = useState(false);
  // The dialog keeps its content while it animates out.
  const [shown, setShown] = useState(dialog);
  if (dialog && dialog !== shown) setShown(dialog);
  if (!shown) return null;
  const { type, conversation } = shown;
  const open = dialog !== null;
  const name = titleOf(conversation, myId);

  const finish = async (work: Promise<boolean>) => {
    setBusy(true);
    const done = await work;
    setBusy(false);
    if (!done) return;
    onClose();
    setClearHistory(false);
    onGone?.(conversation);
  };

  if (type === "mute") {
    return (
      <Dialog open={open} onOpenChange={(next) => !next && onClose()} title={t("mute.title")} description={t("mute.lead", { name })}>
        <div className="flex flex-col">
          {MUTE_CHOICES.map((choice) => (
            <ListRow
              key={choice.key}
              title={t(`mute.${choice.key}`)}
              onClick={() => {
                void actions.mute(conversation, choice.forMs);
                onClose();
              }}
            />
          ))}
        </div>
      </Dialog>
    );
  }

  if (type === "hide") {
    return (
      <ConfirmDialog
        open={open}
        onOpenChange={(next) => !next && onClose()}
        title={t("hide.title", { name })}
        description={t("hide.lead")}
        confirmLabel={clearHistory ? t("hide.confirmClear") : t("hide.confirm")}
        danger={clearHistory}
        loading={busy}
        onConfirm={() => finish(actions.hide(conversation, clearHistory))}
      >
        <SwitchRow label={t("hide.clear")} hint={t("hide.clearHint")} checked={clearHistory} onChange={setClearHistory} />
      </ConfirmDialog>
    );
  }

  return (
    <ConfirmDialog
      open={open}
      onOpenChange={(next) => !next && onClose()}
      title={t("leave.title", { name })}
      description={isGroup(conversation) ? t("leave.lead") : undefined}
      confirmLabel={t("leave.confirm")}
      danger
      loading={busy}
      onConfirm={() => finish(actions.leave(conversation))}
    />
  );
}
