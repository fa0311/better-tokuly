import { useMutation, useQueryClient } from "@tanstack/react-query";
import { common } from "@tokuly/api";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { TextField } from "#/components/ui/field";
import { errorMessage } from "#/lib/errors";
import { moderationClient } from "../api";
import { SUBJECT_TYPES } from "../filter-values";
import { subjectTypeKey } from "../labels";
import { ActionDialog, type ActionTexts } from "../ui/action-dialog";
import { FilterSelect } from "../ui/filters";

/** Accounts are suspended, not taken down, so they are not offered here. */
const REMOVABLE_TYPES = SUBJECT_TYPES.filter((type) => type !== common.SubjectType.USER);

/**
 * Takes a piece of content down by reference, without a report: the tool for
 * something staff found themselves or were told about elsewhere.
 */
export function TakedownDialog({
  open,
  onOpenChange,
}: {
  open: boolean;
  onOpenChange(open: boolean): void;
}) {
  const { t } = useTranslation("admin");
  const queryClient = useQueryClient();
  const [type, setType] = useState<common.SubjectType>(common.SubjectType.NOTE);
  const [id, setId] = useState("");
  const [idMissing, setIdMissing] = useState(false);

  const takedown = useMutation({
    mutationFn: (texts: ActionTexts) =>
      moderationClient().takedownContent({
        subject: { type, id: id.trim() },
        reason: texts.reason,
        note: texts.note,
      }),
    onSuccess: (response) => {
      toast.success(
        t(response.resolvedCase ? "moderation.takedown.doneWithCase" : "moderation.takedown.done"),
      );
      setId("");
      onOpenChange(false);
      return queryClient.invalidateQueries({ queryKey: ["admin"] });
    },
    onError: (error) => toast.error(errorMessage(error)),
  });

  return (
    <ActionDialog
      open={open}
      onOpenChange={onOpenChange}
      title={t("moderation.takedown.title")}
      description={t("moderation.takedown.description")}
      confirmLabel={t("moderation.takedown.confirm")}
      danger
      loading={takedown.isPending}
      requireReason
      validate={() => {
        setIdMissing(!id.trim());
        return Boolean(id.trim());
      }}
      onConfirm={(texts) => takedown.mutate(texts)}
    >
      <FilterSelect
        label={t("moderation.takedown.type")}
        value={String(type)}
        onChange={(value) => setType(Number(value) as common.SubjectType)}
        options={REMOVABLE_TYPES.map((option) => ({
          value: String(option),
          label: t(subjectTypeKey(option)),
        }))}
      />
      <TextField
        label={t("moderation.takedown.id")}
        hint={t("moderation.takedown.idHint")}
        error={idMissing ? t("errors:SUBJECT_INVALID") : undefined}
        value={id}
        onChange={(event) => {
          setId(event.target.value);
          setIdMissing(false);
        }}
        autoCapitalize="none"
        spellCheck={false}
      />
    </ActionDialog>
  );
}
