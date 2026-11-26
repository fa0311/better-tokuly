import { useMutation, useQueryClient } from "@tanstack/react-query";
import { useNavigate } from "@tanstack/react-router";
import type { notes } from "@tokuly/api";
import { charCount, listKeys } from "@tokuly/core/notes";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { Button } from "#/components/ui/button";
import { Dialog } from "#/components/ui/dialog";
import { TextArea, TextField } from "#/components/ui/field";
import { SwitchRow } from "#/components/ui/switch";
import { errorMessage } from "#/lib/errors";
import { LIST_LIMITS, listClient, resetListTimeline, storeList } from "./lists-api";

export const LISTS_NS = { namespace: "notelists" };

/**
 * Creating a list (`list` absent) or changing one: name, description and
 * whether it is public. A new list opens right away.
 */
export function ListEditor({
  open,
  onOpenChange,
  list,
}: {
  open: boolean;
  onOpenChange(open: boolean): void;
  list?: notes.NoteList;
}) {
  const { t } = useTranslation("notelists");
  return (
    <Dialog
      open={open}
      onOpenChange={onOpenChange}
      title={list ? t("editor.editTitle") : t("editor.createTitle")}
    >
      {open && <ListForm list={list} onDone={() => onOpenChange(false)} />}
    </Dialog>
  );
}

function ListForm({ list, onDone }: { list?: notes.NoteList; onDone(): void }) {
  const { t } = useTranslation("notelists");
  const queryClient = useQueryClient();
  const navigate = useNavigate();
  const [name, setName] = useState(list?.name ?? "");
  const [description, setDescription] = useState(list?.description ?? "");
  const [isPublic, setPublic] = useState(list?.public ?? false);

  const save = useMutation({
    mutationFn: async () => {
      const input = { name, description, public: isPublic };
      return list
        ? (await listClient().updateList({ listId: list.id, ...input })).list
        : (await listClient().createList(input)).list;
    },
    onSuccess: async (saved) => {
      storeList(queryClient, saved);
      await queryClient.invalidateQueries({ queryKey: listKeys.all });
      if (list) {
        // Going public can drop members; the timeline follows.
        if (list.public !== saved?.public) await resetListTimeline(queryClient, list.id);
        toast.success(t("toast.listSaved"));
        onDone();
        return;
      }
      toast.success(t("toast.listCreated"));
      onDone();
      if (saved) void navigate({ to: "/notes/lists/$listId", params: { listId: saved.id } });
    },
    onError: (error) => toast.error(errorMessage(error, LISTS_NS)),
  });

  const nameLength = charCount(name.trim());
  const valid =
    nameLength > 0 &&
    nameLength <= LIST_LIMITS.name &&
    charCount(description.trim()) <= LIST_LIMITS.description;
  return (
    <form
      className="flex flex-col gap-4"
      onSubmit={(event) => {
        event.preventDefault();
        if (valid) save.mutate();
      }}
    >
      <TextField
        label={t("editor.name")}
        placeholder={t("editor.namePlaceholder")}
        value={name}
        maxLength={LIST_LIMITS.name * 2}
        hint={`${nameLength}/${LIST_LIMITS.name}`}
        onChange={(event) => setName(event.target.value)}
        autoFocus
      />
      <TextArea
        label={t("editor.description")}
        placeholder={t("editor.descriptionPlaceholder")}
        value={description}
        rows={3}
        hint={`${charCount(description.trim())}/${LIST_LIMITS.description}`}
        onChange={(event) => setDescription(event.target.value)}
      />
      <SwitchRow
        label={t("editor.public")}
        hint={t("editor.publicHint")}
        checked={isPublic}
        onChange={setPublic}
      />
      <Button type="submit" block disabled={!valid} loading={save.isPending}>
        {list ? t("editor.save") : t("editor.create")}
      </Button>
    </form>
  );
}
