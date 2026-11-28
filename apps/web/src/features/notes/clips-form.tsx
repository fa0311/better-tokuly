import { useMutation, useQueryClient } from "@tanstack/react-query";
import type { notes } from "@tokuly/api";
import { useState } from "react";
import { toast } from "sonner";
import { Button } from "#/components/ui/button";
import { TextArea, TextField } from "#/components/ui/field";
import { SwitchRow } from "#/components/ui/switch";
import { errorMessage } from "#/lib/errors";
import { useTranslation } from "#/lib/use-translation";
import { CLIP_DESCRIPTION_MAX, CLIP_NAME_MAX, clipClient, storeClip } from "./clips-api";

/** Name, description and audience of a clip: creates one, or edits `clip`. */
export function ClipForm({
  clip,
  onSaved,
  onCancel,
}: {
  clip?: notes.Clip;
  onSaved(clip: notes.Clip): void;
  onCancel?(): void;
}) {
  const { t } = useTranslation("clips");
  const queryClient = useQueryClient();
  const [name, setName] = useState(clip?.name ?? "");
  const [description, setDescription] = useState(clip?.description ?? "");
  const [isPublic, setPublic] = useState(clip?.public ?? false);
  const save = useMutation({
    mutationFn: async () => {
      const fields = { name: name.trim(), description: description.trim(), public: isPublic };
      const response = clip
        ? await clipClient().updateClip({ clipId: clip.id, ...fields })
        : await clipClient().createClip(fields);
      return response.clip!;
    },
    onSuccess: (saved) => {
      storeClip(queryClient, saved);
      toast.success(t(clip ? "form.saved" : "form.created"));
      onSaved(saved);
    },
    onError: (error) => toast.error(errorMessage(error, { namespace: "clips" })),
  });
  const trimmed = name.trim();
  return (
    <form
      className="flex flex-col gap-4"
      onSubmit={(event) => {
        event.preventDefault();
        if (trimmed) save.mutate();
      }}
    >
      <TextField
        label={t("form.name")}
        placeholder={t("form.namePlaceholder")}
        value={name}
        maxLength={CLIP_NAME_MAX}
        required
        onChange={(event) => setName(event.target.value)}
      />
      <TextArea
        label={t("form.description")}
        placeholder={t("form.descriptionPlaceholder")}
        value={description}
        maxLength={CLIP_DESCRIPTION_MAX}
        onChange={(event) => setDescription(event.target.value)}
      />
      <SwitchRow
        label={t("publicLabel")}
        hint={t("publicHint")}
        checked={isPublic}
        onChange={setPublic}
      />
      <div className="flex justify-end gap-2">
        {onCancel && (
          <Button type="button" variant="ghost" onClick={onCancel} disabled={save.isPending}>
            {t("form.cancel")}
          </Button>
        )}
        <Button type="submit" loading={save.isPending} disabled={!trimmed}>
          {t(clip ? "form.save" : "form.create")}
        </Button>
      </div>
    </form>
  );
}
