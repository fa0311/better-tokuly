import { useMutation, useQueryClient } from "@tanstack/react-query";
import { useNavigate } from "@tanstack/react-router";
import { ask } from "@tokuly/api";
import { Archive, ArchiveRestore, ExternalLink, Flag, Inbox, PenLine, ShieldBan, Trash2, Undo2 } from "lucide-react";
import { type ReactNode, useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { ConfirmDialog } from "#/components/ui/confirm";
import type { MenuAction } from "#/components/ui/menu";
import { ReportDialog } from "#/features/engagement";
import { AnswerComposer } from "./answer-composer";
import { askKeys, boxClient, dropFromInbox, refreshAnswers, refreshInbox } from "./api";
import { toastError } from "./parts";
import { questionSubject } from "./qa-card";

const { QuestionFolder } = ask;

export interface QuestionActions {
  /** Opens the answer composer (or the editor, once answered). */
  answer(): void;
  /** Sets the question aside, or brings it back from the archive. */
  toggleArchive(): void;
  /** Everything the owner can do with this question, for a menu. */
  menu: MenuAction[];
  /** The composer and the confirmations; render it once next to the menu. */
  dialogs: ReactNode;
}

/**
 * What the owner of a box can do with one question of theirs — answer,
 * archive, release from the filter, delete, block whoever sent it, report —
 * wherever the question is shown. Moves are applied to the loaded lists at
 * once and confirmed by the server afterwards.
 */
export function useQuestionActions(question: ask.Question, options: { /** Called after the question was deleted (a page showing only it goes elsewhere). */ onDeleted?(): void } = {}): QuestionActions {
  const { t } = useTranslation("ask");
  const queryClient = useQueryClient();
  const navigate = useNavigate();
  const [dialog, setDialog] = useState<"answer" | "delete" | "deleteAnswer" | "block" | "report" | null>(null);
  const close = () => setDialog(null);
  const id = question.id;
  const answered = Boolean(question.answer);

  const move = useMutation({
    mutationFn: (folder: ask.QuestionFolder) => boxClient().moveQuestion({ questionId: id, folder }),
    onMutate: () => dropFromInbox(queryClient, id),
    onSuccess: (_response, folder) => {
      toast.success(question.folder === QuestionFolder.FILTERED ? t("inbox.toast.released") : folder === QuestionFolder.ARCHIVED ? t("inbox.toast.archived") : t("inbox.toast.unarchived"));
    },
    onError: toastError,
    onSettled: () => refreshInbox(queryClient),
  });
  const remove = useMutation({
    mutationFn: () => boxClient().deleteQuestion({ questionId: id }),
    onSuccess: () => {
      dropFromInbox(queryClient, id);
      refreshAnswers(queryClient);
      queryClient.removeQueries({ queryKey: askKeys.question(id) });
      toast.success(t("inbox.toast.deleted"));
      close();
      options.onDeleted?.();
    },
    onError: toastError,
  });
  const removeAnswer = useMutation({
    mutationFn: async () => (await boxClient().deleteAnswer({ questionId: id })).question!,
    onSuccess: (unanswered) => {
      dropFromInbox(queryClient, id);
      refreshAnswers(queryClient, unanswered);
      toast.success(t("inbox.toast.answerDeleted"));
      close();
    },
    onError: toastError,
  });
  const block = useMutation({
    mutationFn: () => boxClient().blockSender({ target: { case: "questionId", value: id } }),
    onSuccess: () => {
      void queryClient.invalidateQueries({ queryKey: askKeys.blocks });
      toast.success(t("inbox.toast.blocked"));
      close();
    },
    onError: toastError,
  });

  const toggleArchive = () => move.mutate(question.folder === QuestionFolder.ARCHIVED ? QuestionFolder.INBOX : QuestionFolder.ARCHIVED);

  const menu: MenuAction[] = [
    ...(answered
      ? [
          { label: t("inbox.actions.open"), icon: <ExternalLink />, onSelect: () => void navigate({ to: "/ask/q/$questionId", params: { questionId: id } }) },
          { label: t("inbox.actions.editAnswer"), icon: <PenLine />, onSelect: () => setDialog("answer") },
          { label: t("inbox.actions.deleteAnswer"), icon: <Undo2 />, onSelect: () => setDialog("deleteAnswer") },
        ]
      : [
          { label: t("inbox.actions.answer"), icon: <PenLine />, onSelect: () => setDialog("answer") },
          question.folder === QuestionFolder.FILTERED
            ? { label: t("inbox.actions.release"), icon: <Inbox />, onSelect: () => move.mutate(QuestionFolder.INBOX) }
            : question.folder === QuestionFolder.ARCHIVED
              ? { label: t("inbox.actions.unarchive"), icon: <ArchiveRestore />, onSelect: toggleArchive }
              : { label: t("inbox.actions.archive"), icon: <Archive />, onSelect: toggleArchive },
        ]),
    { label: t("inbox.actions.block"), icon: <ShieldBan />, onSelect: () => setDialog("block") },
    { label: t("inbox.actions.report"), icon: <Flag />, onSelect: () => setDialog("report") },
    { label: t("inbox.actions.delete"), icon: <Trash2 />, danger: true, onSelect: () => setDialog("delete") },
  ];

  const dialogs = (
    <>
      {dialog === "answer" && <AnswerComposer question={question} onClose={close} />}
      <ConfirmDialog
        open={dialog === "delete"}
        onOpenChange={close}
        title={t("inbox.confirmDelete.title")}
        description={t("inbox.confirmDelete.body")}
        confirmLabel={t("inbox.confirmDelete.confirm")}
        danger
        loading={remove.isPending}
        onConfirm={() => remove.mutate()}
      />
      <ConfirmDialog
        open={dialog === "deleteAnswer"}
        onOpenChange={close}
        title={t("inbox.confirmDeleteAnswer.title")}
        description={t("inbox.confirmDeleteAnswer.body")}
        confirmLabel={t("inbox.confirmDeleteAnswer.confirm")}
        danger
        loading={removeAnswer.isPending}
        onConfirm={() => removeAnswer.mutate()}
      />
      <ConfirmDialog
        open={dialog === "block"}
        onOpenChange={close}
        title={t("inbox.confirmBlock.title")}
        description={t("inbox.confirmBlock.body")}
        confirmLabel={t("inbox.confirmBlock.confirm")}
        danger
        loading={block.isPending}
        onConfirm={() => block.mutate()}
      />
      <ReportDialog subject={questionSubject(id)} open={dialog === "report"} onOpenChange={(open) => !open && close()} label={question.text} />
    </>
  );

  return { answer: () => setDialog("answer"), toggleArchive, menu, dialogs };
}
