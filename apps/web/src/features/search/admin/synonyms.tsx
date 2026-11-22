import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import type { search } from "@tokuly/api";
import { Pencil, Plus, Trash2 } from "lucide-react";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { Button } from "#/components/ui/button";
import { ConfirmDialog } from "#/components/ui/confirm";
import { Dialog } from "#/components/ui/dialog";
import { TextArea } from "#/components/ui/field";
import { EmptyState, Skeleton } from "#/components/ui/page";
import { searchAdminClient, searchKeys } from "../api";
import { formatDateTime, Pane, staffErrorMessage } from "./parts";
import { MAX_WORDS, MIN_WORDS, parseWords } from "./words";

/** Groups of words that find each other's documents ("スタンプ", "ステッカー", "sticker"). */
export function Synonyms() {
  const { t } = useTranslation("search");
  const queryClient = useQueryClient();
  /** The group being edited; `{}` for a new one. */
  const [editing, setEditing] = useState<{ group?: search.SynonymGroup }>();
  const [deleting, setDeleting] = useState<search.SynonymGroup>();
  const groups = useQuery({
    queryKey: searchKeys.admin.synonyms,
    queryFn: () => searchAdminClient().listSynonymGroups({}),
  });
  const remove = useMutation({
    mutationFn: (id: string) => searchAdminClient().deleteSynonymGroup({ id }),
    onSuccess: () => {
      toast.success(t("staff.synonyms.deleted"));
      return queryClient.invalidateQueries({ queryKey: searchKeys.admin.synonyms });
    },
    onError: (error) => toast.error(staffErrorMessage(error)),
    onSettled: () => setDeleting(undefined),
  });
  const list = groups.data?.groups ?? [];

  return (
    <Pane
      title={t("staff.synonyms.title")}
      aside={
        <Button size="sm" icon={<Plus className="size-4" />} onClick={() => setEditing({})}>
          {t("staff.synonyms.add")}
        </Button>
      }
    >
      <p className="border-b-2 border-line px-4 py-3 text-sm leading-relaxed text-ink-soft">
        {t("staff.synonyms.lead")}
      </p>
      {groups.isPending ? (
        <div className="flex flex-col gap-2 p-4" aria-busy>
          <Skeleton className="h-12" />
          <Skeleton className="h-12" />
        </div>
      ) : groups.isError ? (
        <EmptyState emoji="🌀" title={staffErrorMessage(groups.error)} />
      ) : list.length === 0 ? (
        <EmptyState
          emoji="🔗"
          title={t("staff.synonyms.empty")}
          body={t("staff.synonyms.emptyBody")}
        />
      ) : (
        <ul aria-label={t("staff.synonyms.title")} className="divide-y-2 divide-line">
          {list.map((group) => (
            <li
              key={group.id}
              data-testid="synonym-group"
              className="flex items-center gap-3 px-4 py-3"
            >
              <div className="min-w-0 flex-1">
                <ul className="flex flex-wrap gap-1.5">
                  {group.words.map((word) => (
                    <li
                      key={word}
                      className="max-w-full truncate rounded-full bg-brand-soft px-2.5 py-1 text-sm font-bold text-brand-strong"
                    >
                      {word}
                    </li>
                  ))}
                </ul>
                <p className="mt-1 text-xs text-ink-faint">
                  {t("staff.synonyms.updated", { when: formatDateTime(group.updatedAt) })}
                </p>
              </div>
              <Button
                size="icon-sm"
                variant="ghost"
                aria-label={t("staff.synonyms.edit", { words: group.words.join(", ") })}
                onClick={() => setEditing({ group })}
              >
                <Pencil className="size-4" />
              </Button>
              <Button
                size="icon-sm"
                variant="ghost"
                aria-label={t("staff.synonyms.delete", { words: group.words.join(", ") })}
                onClick={() => setDeleting(group)}
              >
                <Trash2 className="size-4" />
              </Button>
            </li>
          ))}
        </ul>
      )}
      {editing && (
        <GroupDialog
          key={editing.group?.id ?? "new"}
          group={editing.group}
          onClose={() => setEditing(undefined)}
        />
      )}
      <ConfirmDialog
        open={deleting !== undefined}
        onOpenChange={(open) => !open && setDeleting(undefined)}
        title={t("staff.synonyms.deleteTitle")}
        description={deleting?.words.join("、")}
        confirmLabel={t("staff.synonyms.deleteConfirm")}
        danger
        loading={remove.isPending}
        onConfirm={() => deleting && remove.mutate(deleting.id)}
      />
    </Pane>
  );
}

function GroupDialog({ group, onClose }: { group?: search.SynonymGroup; onClose(): void }) {
  const { t } = useTranslation(["search", "common"]);
  const queryClient = useQueryClient();
  const [text, setText] = useState(group?.words.join("\n") ?? "");
  const words = parseWords(text);
  const valid = words.length >= MIN_WORDS && words.length <= MAX_WORDS;
  const save = useMutation({
    mutationFn: () => searchAdminClient().saveSynonymGroup({ id: group?.id ?? "", words }),
    onSuccess: () => {
      toast.success(t("search:staff.synonyms.saved"));
      onClose();
      return queryClient.invalidateQueries({ queryKey: searchKeys.admin.synonyms });
    },
    onError: (error) => toast.error(staffErrorMessage(error)),
  });
  return (
    <Dialog
      open
      onOpenChange={(open) => !open && onClose()}
      title={t(group ? "search:staff.synonyms.editTitle" : "search:staff.synonyms.addTitle")}
      description={t("search:staff.synonyms.dialogLead")}
      footer={
        <>
          <Button variant="ghost" onClick={onClose} disabled={save.isPending}>
            {t("common:actions.cancel")}
          </Button>
          <Button onClick={() => save.mutate()} disabled={!valid} loading={save.isPending}>
            {t("common:actions.save")}
          </Button>
        </>
      }
    >
      <TextArea
        label={t("search:staff.synonyms.words")}
        hint={t("search:staff.synonyms.wordsHint", { count: words.length })}
        error={words.length > MAX_WORDS ? t("search:staff.errors.SYNONYMS_INVALID") : undefined}
        value={text}
        onChange={(event) => setText(event.target.value)}
        rows={6}
        autoCapitalize="none"
        spellCheck={false}
      />
    </Dialog>
  );
}
