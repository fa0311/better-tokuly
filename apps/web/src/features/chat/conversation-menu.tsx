import { chat } from "@tokuly/api";
import { Archive, ArchiveRestore, Bell, BellOff, Check, CheckCheck, EyeOff, LogOut, Pin, PinOff, X } from "lucide-react";
import { useTranslation } from "react-i18next";
import type { MenuAction } from "#/components/ui/menu";
import { useConversationActions } from "./actions";
import type { ConversationDialog } from "./conversation-dialogs";
import { isGroup, isMuted } from "./model";

/** The actions on a whole conversation, as menu entries (list rows and the conversation's own menu share them). */
export function useConversationMenu(conversation: chat.Conversation | undefined, onDialog: (dialog: ConversationDialog) => void): MenuAction[] {
  const { t } = useTranslation("chat");
  const actions = useConversationActions();
  const viewer = conversation?.viewer;
  if (!conversation || !viewer) return [];
  if (viewer.requestState === chat.RequestState.PENDING) {
    return [
      { label: t("request.accept"), icon: <Check />, onSelect: () => void actions.respond(conversation, true) },
      { label: t("request.decline"), icon: <X />, danger: true, onSelect: () => void actions.respond(conversation, false) },
    ];
  }
  const muted = isMuted(viewer);
  const entries: MenuAction[] = [];
  if (viewer.unreadCount > 0) entries.push({ label: t("menu.markRead"), icon: <CheckCheck />, onSelect: () => void actions.markRead(conversation) });
  entries.push(
    viewer.pinned
      ? { label: t("menu.unpin"), icon: <PinOff />, onSelect: () => void actions.setPinned(conversation, false) }
      : { label: t("menu.pin"), icon: <Pin />, onSelect: () => void actions.setPinned(conversation, true) },
    muted
      ? { label: t("menu.unmute"), icon: <Bell />, onSelect: () => void actions.unmute(conversation) }
      : { label: t("menu.mute"), icon: <BellOff />, onSelect: () => onDialog({ type: "mute", conversation }) },
    viewer.archived
      ? { label: t("menu.unarchive"), icon: <ArchiveRestore />, onSelect: () => void actions.setArchived(conversation, false) }
      : { label: t("menu.archive"), icon: <Archive />, onSelect: () => void actions.setArchived(conversation, true) },
    { label: t("menu.hide"), icon: <EyeOff />, onSelect: () => onDialog({ type: "hide", conversation }) },
  );
  if (isGroup(conversation)) entries.push({ label: t("menu.leave"), icon: <LogOut />, danger: true, onSelect: () => onDialog({ type: "leave", conversation }) });
  return entries;
}
