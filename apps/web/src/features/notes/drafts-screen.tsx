import { useInfiniteQuery, useMutation, useQueryClient } from "@tanstack/react-query";
import { useNavigate } from "@tanstack/react-router";
import { type notes, toMillis } from "@tokuly/api";
import { notesKeys } from "@tokuly/core/notes";
import { CalendarClock, Send, Trash2 } from "lucide-react";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { Button } from "#/components/ui/button";
import { LoadMore } from "#/components/ui/load-more";
import { EmptyState } from "#/components/ui/page";
import { Tabs } from "#/components/ui/tabs";
import { errorMessage, reasonText } from "#/lib/errors";
import { draftClient } from "./api";
import { NotesPage } from "./shell";
import { noteDateTime } from "./time";

/** The person's drafts kept on the server, and the notes they scheduled. */
export function DraftsScreen() {
  const { t, i18n } = useTranslation("notes");
  const [scheduledOnly, setScheduledOnly] = useState(false);
  const queryClient = useQueryClient();
  const navigate = useNavigate();
  const query = useInfiniteQuery({
    queryKey: [...notesKeys.drafts, scheduledOnly],
    queryFn: ({ pageParam }) =>
      draftClient().listDrafts({ scheduledOnly, page: { cursor: pageParam, limit: 30 } }),
    initialPageParam: "",
    getNextPageParam: (last) => last.page?.nextCursor || undefined,
  });
  const drafts = query.data?.pages.flatMap((page) => page.drafts) ?? [];
  const refresh = () => queryClient.invalidateQueries({ queryKey: notesKeys.drafts });
  const publish = useMutation({
    mutationFn: (draft: notes.Draft) => draftClient().publishDraft({ draftId: draft.id }),
    onSuccess: (response) => {
      void refresh();
      void queryClient.invalidateQueries({ queryKey: notesKeys.timelines });
      toast.success(t("toast.posted"), {
        action: {
          label: t("toast.view"),
          onClick: () =>
            void navigate({ to: "/notes/$noteId", params: { noteId: response.note!.id } }),
        },
      });
    },
    onError: (error) => toast.error(errorMessage(error)),
  });
  const remove = useMutation({
    mutationFn: (draft: notes.Draft) => draftClient().deleteDraft({ draftId: draft.id }),
    onSuccess: () => {
      void refresh();
      toast.success(t("drafts.deleted"));
    },
    onError: (error) => toast.error(errorMessage(error)),
  });

  return (
    <NotesPage title={t("drafts.title")} back="/notes" scrollId="notes-drafts">
      <Tabs
        items={[
          { value: "all", label: t("drafts.all") },
          { value: "scheduled", label: t("drafts.scheduled") },
        ]}
        value={scheduledOnly ? "scheduled" : "all"}
        onChange={(value) => setScheduledOnly(value === "scheduled")}
        className="mb-3"
      />
      {!query.isPending && drafts.length === 0 && (
        <EmptyState emoji="📝" title={t("drafts.empty")} body={t("drafts.emptyBody")} />
      )}
      <ul className="flex flex-col gap-2.5" data-testid="drafts">
        {drafts.map((draft) => (
          <li
            key={draft.id}
            data-testid="draft"
            className="rounded-[1.25rem] border-2 border-line bg-surface px-3.5 py-3"
          >
            <p className="line-clamp-3 break-words whitespace-pre-wrap">
              {draft.content?.text || t("drafts.noText")}
            </p>
            {draft.attachments.length > 0 && (
              <p className="mt-1 text-xs font-bold text-ink-faint">
                {t("drafts.attachments", { count: draft.attachments.length })}
              </p>
            )}
            {draft.scheduledAt && (
              <p className="mt-1.5 flex items-center gap-1.5 text-sm font-black text-brand">
                <CalendarClock className="size-4" />
                {t("drafts.scheduledAt", {
                  time: noteDateTime(toMillis(draft.scheduledAt), i18n.language),
                })}
              </p>
            )}
            {draft.failureReason && (
              <p className="mt-1.5 text-sm font-bold text-danger">
                {t("drafts.failed", {
                  reason:
                    reasonText(draft.failureReason, { namespace: "notes" }) ?? draft.failureReason,
                })}
              </p>
            )}
            <div className="mt-2 flex items-center gap-2">
              <span className="text-xs text-ink-faint">
                {noteDateTime(toMillis(draft.updatedAt), i18n.language)}
              </span>
              <span className="flex-1" />
              <Button
                variant="ghost"
                size="sm"
                aria-label={t("drafts.delete")}
                onClick={() => remove.mutate(draft)}
              >
                <Trash2 className="size-4" />
              </Button>
              <Button size="sm" loading={publish.isPending} onClick={() => publish.mutate(draft)}>
                <Send className="size-4" />
                {t("drafts.publish")}
              </Button>
            </div>
          </li>
        ))}
      </ul>
      {drafts.length > 0 && <LoadMore query={query} />}
    </NotesPage>
  );
}
