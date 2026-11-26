import { useQueryClient } from "@tanstack/react-query";
import { ask } from "@tokuly/api";
import { askKeys, dropFromInbox, refreshAnswers, refreshInbox } from "@tokuly/core/ask";
import { router } from "expo-router";
import { type ReactNode, useState } from "react";
import { useTranslation } from "react-i18next";
import { Alert } from "react-native";
import { ActionSheet, type SheetAction } from "#/components/action-sheet";
import { boxAs, stillActive } from "./data";
import { ReportSheet, showAskError } from "./parts";

const { QuestionFolder } = ask;

export interface QuestionActions {
  /** Opens the answer screen (the editor, once answered). */
  answer(): void;
  /** Opens the sheet of everything the owner can do with the question. */
  openMenu(): void;
  /** The sheet and the report reasons; render once next to the question. */
  sheets: ReactNode;
}

/**
 * What the owner of a box can do with one question of theirs — answer,
 * archive, release from the filter, delete, block whoever sent it, report —
 * wherever the question is shown. The block names nobody: the server copies
 * the sender from the question, and the owner never learns who it was.
 */
export function useQuestionActions(
  question: ask.Question,
  options: { onDeleted?(): void } = {},
): QuestionActions {
  const { t } = useTranslation("ask");
  const { t: tc } = useTranslation("common");
  const { t: tm } = useTranslation("mobile");
  const queryClient = useQueryClient();
  const [menu, setMenu] = useState(false);
  const [reporting, setReporting] = useState(false);
  const id = question.id;
  const answered = Boolean(question.answer);

  /** Runs one change as the account in use now; `done` only while that account still is. */
  const change = async <T,>(
    send: (client: ReturnType<typeof boxAs>["client"]) => Promise<T>,
    done: (result: T) => void,
  ) => {
    const { account, client } = boxAs();
    try {
      const result = await send(client);
      if (stillActive(account)) done(result);
    } catch (error) {
      if (stillActive(account)) showAskError(error);
    }
  };

  const confirm = (title: string, body: string, label: string, onConfirm: () => void) =>
    Alert.alert(title, body, [
      { text: tc("actions.cancel"), style: "cancel" },
      { text: label, style: "destructive", onPress: onConfirm },
    ]);

  const move = (folder: ask.QuestionFolder) => {
    dropFromInbox(queryClient, id);
    void change(
      (client) => client.moveQuestion({ questionId: id, folder }),
      () =>
        Alert.alert(
          question.folder === QuestionFolder.FILTERED
            ? t("inbox.toast.released")
            : folder === QuestionFolder.ARCHIVED
              ? t("inbox.toast.archived")
              : t("inbox.toast.unarchived"),
        ),
    ).finally(() => refreshInbox(queryClient));
  };
  const toggleArchive = () =>
    move(
      question.folder === QuestionFolder.ARCHIVED ? QuestionFolder.INBOX : QuestionFolder.ARCHIVED,
    );

  const remove = () =>
    confirm(
      t("inbox.confirmDelete.title"),
      t("inbox.confirmDelete.body"),
      t("inbox.confirmDelete.confirm"),
      () =>
        void change(
          (client) => client.deleteQuestion({ questionId: id }),
          () => {
            dropFromInbox(queryClient, id);
            refreshAnswers(queryClient);
            queryClient.removeQueries({ queryKey: askKeys.question(id) });
            Alert.alert(t("inbox.toast.deleted"));
            options.onDeleted?.();
          },
        ),
    );
  const removeAnswer = () =>
    confirm(
      t("inbox.confirmDeleteAnswer.title"),
      t("inbox.confirmDeleteAnswer.body"),
      t("inbox.confirmDeleteAnswer.confirm"),
      () =>
        void change(
          async (client) => (await client.deleteAnswer({ questionId: id })).question,
          (unanswered) => {
            dropFromInbox(queryClient, id);
            refreshAnswers(queryClient, unanswered);
            Alert.alert(t("inbox.toast.answerDeleted"));
          },
        ),
    );
  const block = () =>
    confirm(
      t("inbox.confirmBlock.title"),
      t("inbox.confirmBlock.body"),
      t("inbox.confirmBlock.confirm"),
      () =>
        void change(
          (client) => client.blockSender({ target: { case: "questionId", value: id } }),
          () => {
            void queryClient.invalidateQueries({ queryKey: askKeys.blocks });
            Alert.alert(t("inbox.toast.blocked"));
          },
        ),
    );

  const answer = () => router.push(`/ask/answer/${id}`);

  const actions: SheetAction[] = answered
    ? [
        {
          key: "ask-open",
          label: t("inbox.actions.open"),
          onPress: () => router.push(`/ask/q/${id}`),
        },
        { key: "ask-edit", label: t("inbox.actions.editAnswer"), onPress: answer },
        { key: "ask-unanswer", label: t("inbox.actions.deleteAnswer"), onPress: removeAnswer },
      ]
    : [
        { key: "ask-answer", label: t("inbox.actions.answer"), onPress: answer },
        question.folder === QuestionFolder.FILTERED
          ? {
              key: "ask-release",
              label: t("inbox.actions.release"),
              onPress: () => move(QuestionFolder.INBOX),
            }
          : question.folder === QuestionFolder.ARCHIVED
            ? { key: "ask-unarchive", label: t("inbox.actions.unarchive"), onPress: toggleArchive }
            : { key: "ask-archive", label: t("inbox.actions.archive"), onPress: toggleArchive },
      ];
  actions.push(
    { key: "ask-block", label: t("inbox.actions.block"), onPress: block, destructive: true },
    // iOS presents no modal while the sheet before it is still sliding away.
    {
      key: "ask-report",
      label: t("inbox.actions.report"),
      onPress: () => setTimeout(() => setReporting(true), 400),
    },
    { key: "ask-delete", label: t("inbox.actions.delete"), onPress: remove, destructive: true },
  );

  const sheets = (
    <>
      <ActionSheet
        visible={menu}
        title={tm("ask.actions")}
        actions={actions}
        onClose={() => setMenu(false)}
      />
      <ReportSheet questionId={reporting ? id : null} onClose={() => setReporting(false)} />
    </>
  );

  return { answer, openMenu: () => setMenu(true), sheets };
}
