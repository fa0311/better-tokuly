import { useMutation, useQueryClient } from "@tanstack/react-query";
import type { emoji as emojiApi } from "@tokuly/api";
import { GripVertical, Pencil, Plus, Trash2 } from "lucide-react";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { Button } from "#/components/ui/button";
import { Dialog } from "#/components/ui/dialog";
import { TextField } from "#/components/ui/field";
import { EmptyState, Skeleton } from "#/components/ui/page";
import { SelectField } from "#/components/ui/select";
import { Sortable } from "#/components/ui/sortable";
import { errorMessage } from "#/lib/errors";
import { categoriesKey, emojiAdminClient, useAdminRefresh, useCategories } from "./shared";

/** The categories of the picker: add, rename, drag into order, delete (moving their emoji elsewhere). */
export function Categories() {
  const { t } = useTranslation("emoji");
  const { t: tCommon } = useTranslation("common");
  const queryClient = useQueryClient();
  const refresh = useAdminRefresh();
  const categories = useCategories();
  const [name, setName] = useState("");
  const [renaming, setRenaming] = useState<{ name: string; next: string } | null>(null);
  const [deleting, setDeleting] = useState<{ name: string; moveTo: string } | null>(null);

  const list = categories.data?.categories ?? [];
  // Every change answers with the whole list in its new order.
  const applied = async (next: emojiApi.Category[]) => {
    queryClient.setQueryData(
      categoriesKey,
      (current: emojiApi.ListCategoriesResponse | undefined) =>
        current ? { ...current, categories: next } : current,
    );
    await refresh();
  };
  const onError = (error: unknown) => {
    toast.error(errorMessage(error, { namespace: "emoji" }));
    void queryClient.invalidateQueries({ queryKey: categoriesKey });
  };
  const create = useMutation({
    mutationFn: () => emojiAdminClient().createCategory({ name: name.trim() }),
    onSuccess: async (response) => {
      setName("");
      toast.success(t("admin.categoryCreated"));
      await applied(response.categories);
    },
    onError,
  });
  const rename = useMutation({
    mutationFn: (input: { name: string; next: string }) =>
      emojiAdminClient().renameCategory({ name: input.name, newName: input.next.trim() }),
    onSuccess: async (response) => {
      setRenaming(null);
      toast.success(t("admin.categoryRenamed"));
      await applied(response.categories);
    },
    onError,
  });
  const reorder = useMutation({
    mutationFn: (names: string[]) => emojiAdminClient().reorderCategories({ names }),
    onMutate: (names) => {
      // The list keeps the dropped order while the server is asked.
      queryClient.setQueryData(
        categoriesKey,
        (current: emojiApi.ListCategoriesResponse | undefined) =>
          current
            ? {
                ...current,
                categories: names.flatMap((item) =>
                  current.categories.filter((entry) => entry.name === item),
                ),
              }
            : current,
      );
    },
    onSuccess: (response) => applied(response.categories),
    onError,
  });
  const remove = useMutation({
    mutationFn: (input: { name: string; moveTo: string }) =>
      emojiAdminClient().deleteCategory(input),
    onSuccess: async (response) => {
      setDeleting(null);
      toast.success(t("admin.categoryDeleted"));
      await applied(response.categories);
    },
    onError,
  });

  return (
    <div className="max-w-2xl">
      <form
        className="flex items-end gap-2"
        onSubmit={(event) => {
          event.preventDefault();
          if (name.trim()) create.mutate();
        }}
      >
        <TextField
          className="min-w-0 flex-1"
          label={t("admin.newCategory")}
          value={name}
          maxLength={32}
          onChange={(event) => setName(event.target.value)}
        />
        <Button
          type="submit"
          className="mb-0.5"
          icon={<Plus className="size-4" />}
          loading={create.isPending}
          disabled={!name.trim()}
        >
          {t("admin.createCategory")}
        </Button>
      </form>
      <p className="mt-1.5 px-1 text-xs text-ink-faint">{t("admin.categoryHint")}</p>

      <div className="mt-4 rounded-2xl border-2 border-line bg-surface p-2">
        {categories.isPending ? (
          <Skeleton className="h-32" />
        ) : list.length === 0 ? (
          <EmptyState
            emoji="🗂️"
            title={t("admin.categoriesEmpty")}
            body={t("admin.categoriesEmptyBody")}
          />
        ) : (
          <Sortable
            items={list}
            getKey={(entry) => entry.name}
            onReorder={(next) => reorder.mutate(next.map((entry) => entry.name))}
            className="flex flex-col gap-1"
          >
            {(entry, state) => (
              <div
                data-testid="category-row"
                className="flex items-center gap-2 rounded-xl bg-surface px-1 py-1.5"
              >
                <button
                  type="button"
                  {...state.grip}
                  aria-label={t("admin.moveCategory", { name: entry.name })}
                  className="grid size-9 shrink-0 place-items-center rounded-lg text-ink-faint hover:bg-surface-2"
                >
                  <GripVertical className="size-5" />
                </button>
                <span className="min-w-0 flex-1 truncate font-bold">{entry.name}</span>
                <span className="text-sm text-ink-faint">
                  {t("admin.categoryCount", { count: entry.emojiCount })}
                </span>
                <Button
                  variant="ghost"
                  size="icon-sm"
                  aria-label={t("admin.renameCategory", { name: entry.name })}
                  onClick={() => setRenaming({ name: entry.name, next: entry.name })}
                >
                  <Pencil className="size-4" />
                </Button>
                <Button
                  variant="ghost"
                  size="icon-sm"
                  className="text-danger"
                  aria-label={t("admin.deleteCategory", { name: entry.name })}
                  onClick={() => setDeleting({ name: entry.name, moveTo: "" })}
                >
                  <Trash2 className="size-4" />
                </Button>
              </div>
            )}
          </Sortable>
        )}
      </div>
      {categories.data && categories.data.uncategorizedCount > 0 && (
        <p className="mt-2 px-1 text-sm text-ink-soft">
          {t("admin.uncategorizedCount", { count: categories.data.uncategorizedCount })}
        </p>
      )}

      {renaming && (
        <Dialog
          open
          onOpenChange={() => setRenaming(null)}
          title={t("admin.renameCategory", { name: renaming.name })}
          footer={
            <>
              <Button variant="ghost" onClick={() => setRenaming(null)}>
                {tCommon("actions.cancel")}
              </Button>
              <Button
                loading={rename.isPending}
                disabled={!renaming.next.trim() || renaming.next.trim() === renaming.name}
                onClick={() => rename.mutate(renaming)}
              >
                {t("admin.save")}
              </Button>
            </>
          }
        >
          <TextField
            label={t("admin.categoryName")}
            value={renaming.next}
            maxLength={32}
            onChange={(event) => setRenaming({ ...renaming, next: event.target.value })}
          />
        </Dialog>
      )}
      {deleting && (
        <Dialog
          open
          onOpenChange={() => setDeleting(null)}
          title={t("admin.deleteCategory", { name: deleting.name })}
          description={t("admin.deleteCategoryBody")}
          footer={
            <>
              <Button variant="ghost" onClick={() => setDeleting(null)}>
                {tCommon("actions.cancel")}
              </Button>
              <Button
                variant="danger"
                loading={remove.isPending}
                onClick={() => remove.mutate(deleting)}
              >
                {t("admin.delete")}
              </Button>
            </>
          }
        >
          <SelectField
            label={t("admin.moveTo")}
            value={deleting.moveTo}
            onChange={(moveTo) => setDeleting({ ...deleting, moveTo })}
            options={[
              { value: "", label: t("form.noCategory") },
              ...list
                .filter((entry) => entry.name !== deleting.name)
                .map((entry) => ({ value: entry.name, label: entry.name })),
            ]}
          />
        </Dialog>
      )}
    </div>
  );
}
