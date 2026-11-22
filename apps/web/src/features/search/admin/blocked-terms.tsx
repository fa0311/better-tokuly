import { useInfiniteQuery, useMutation, useQueryClient } from "@tanstack/react-query";
import { Ban, X } from "lucide-react";
import { type FormEvent, useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { Avatar } from "#/components/ui/avatar";
import { Button } from "#/components/ui/button";
import { TextField } from "#/components/ui/field";
import { LoadMore } from "#/components/ui/load-more";
import { EmptyState, Skeleton } from "#/components/ui/page";
import { searchAdminClient, searchKeys } from "../api";
import { formatDateTime, Pane, staffErrorMessage } from "./parts";

const TERM_MAX = 64;
const NOTE_MAX = 200;

/**
 * Terms that keep a query out of trending and out of the typeahead's popular
 * completions. They do not hide search results: content is removed in its own
 * domain (or, for the moment, under "documents").
 */
export function BlockedTerms() {
  const { t } = useTranslation("search");
  const queryClient = useQueryClient();
  const [term, setTerm] = useState("");
  const [note, setNote] = useState("");
  const list = useInfiniteQuery({
    queryKey: searchKeys.admin.blockedTerms,
    queryFn: ({ pageParam }) =>
      searchAdminClient().listBlockedTerms({ page: { cursor: pageParam, limit: 50 } }),
    initialPageParam: "",
    getNextPageParam: (last) => last.page?.nextCursor || undefined,
  });
  // What trends and completions show changes with the list.
  const refresh = () =>
    Promise.all([
      queryClient.invalidateQueries({ queryKey: searchKeys.admin.blockedTerms }),
      queryClient.invalidateQueries({ queryKey: searchKeys.trending }),
      queryClient.invalidateQueries({ queryKey: searchKeys.suggestions }),
    ]);
  const add = useMutation({
    mutationFn: () => searchAdminClient().addBlockedTerm({ term: term.trim(), note: note.trim() }),
    onSuccess: (response) => {
      toast.success(t("staff.blocked.added", { term: response.term?.term ?? term.trim() }));
      setTerm("");
      setNote("");
      return refresh();
    },
    onError: (error) => toast.error(staffErrorMessage(error)),
  });
  const remove = useMutation({
    mutationFn: (value: string) => searchAdminClient().removeBlockedTerm({ term: value }),
    onSuccess: (_response, value) => {
      toast.success(t("staff.blocked.removed", { term: value }));
      return refresh();
    },
    onError: (error) => toast.error(staffErrorMessage(error)),
  });
  const terms = list.data?.pages.flatMap((page) => page.terms) ?? [];
  const submit = (event: FormEvent) => {
    event.preventDefault();
    if (term.trim()) add.mutate();
  };

  return (
    <div className="grid grid-cols-1 items-start gap-4 xl:grid-cols-[22rem_minmax(0,1fr)]">
      <Pane title={t("staff.blocked.addTitle")}>
        <form onSubmit={submit} className="flex flex-col gap-3 p-4">
          <p className="text-sm leading-relaxed text-ink-soft">{t("staff.blocked.lead")}</p>
          <TextField
            label={t("staff.blocked.term")}
            value={term}
            onChange={(event) => setTerm(event.target.value)}
            maxLength={TERM_MAX}
            autoCapitalize="none"
            autoComplete="off"
            spellCheck={false}
          />
          <TextField
            label={t("staff.blocked.note")}
            hint={t("staff.blocked.noteHint")}
            value={note}
            onChange={(event) => setNote(event.target.value)}
            maxLength={NOTE_MAX}
            autoComplete="off"
          />
          <Button
            type="submit"
            disabled={!term.trim()}
            loading={add.isPending}
            icon={<Ban className="size-4" />}
          >
            {t("staff.blocked.add")}
          </Button>
        </form>
      </Pane>
      <Pane title={t("staff.blocked.listTitle")}>
        {list.isPending ? (
          <div className="flex flex-col gap-2 p-4" aria-busy>
            <Skeleton className="h-12" />
            <Skeleton className="h-12" />
          </div>
        ) : list.isError ? (
          <EmptyState emoji="🌀" title={staffErrorMessage(list.error)} />
        ) : terms.length === 0 ? (
          <EmptyState
            emoji="🕊️"
            title={t("staff.blocked.empty")}
            body={t("staff.blocked.emptyBody")}
          />
        ) : (
          <>
            <ul aria-label={t("staff.blocked.listTitle")} className="divide-y-2 divide-line">
              {terms.map((entry) => (
                <li
                  key={entry.term}
                  data-testid="blocked-term"
                  className="flex items-center gap-3 px-4 py-2.5"
                >
                  <div className="min-w-0 flex-1">
                    <p className="truncate font-black">{entry.term}</p>
                    {entry.note && <p className="truncate text-sm text-ink-soft">{entry.note}</p>}
                    <p className="flex min-w-0 items-center gap-1.5 text-xs text-ink-faint">
                      {entry.addedBy && (
                        <>
                          <Avatar user={entry.addedBy} size="xs" />
                          <span className="truncate">
                            {entry.addedBy.displayName || entry.addedBy.handle}
                          </span>
                          <span aria-hidden>·</span>
                        </>
                      )}
                      <span className="shrink-0">{formatDateTime(entry.createdAt)}</span>
                    </p>
                  </div>
                  <Button
                    size="icon-sm"
                    variant="ghost"
                    aria-label={t("staff.blocked.remove", { term: entry.term })}
                    onClick={() => remove.mutate(entry.term)}
                    disabled={remove.isPending && remove.variables === entry.term}
                  >
                    <X className="size-4" />
                  </Button>
                </li>
              ))}
            </ul>
            <LoadMore query={list} />
          </>
        )}
      </Pane>
    </div>
  );
}
