import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import type { search } from "@tokuly/api";
import { isKind, KINDS, type Kind, kindSlug, searchKeys } from "@tokuly/core/search";
import { Search, Trash2 } from "lucide-react";
import { type FormEvent, useId, useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { Button } from "#/components/ui/button";
import { ConfirmDialog } from "#/components/ui/confirm";
import { TextField } from "#/components/ui/field";
import { EmptyState, Skeleton } from "#/components/ui/page";
import { searchAdminClient, searchClient } from "../api";
import { useDebounced } from "../hooks";
import { KindTile } from "../kind-icon";
import { Pane, staffErrorMessage } from "./parts";

/** A document to take out of the index: found by searching, or named by kind and ID. */
interface Target {
  kind: search.SearchKind;
  id: string;
  title?: string;
}

/**
 * Takes one document out of the index. Staff find it the way anyone would —
 * by searching — and remove it from the results; a document that cannot be
 * found by text can be named by kind and ID.
 */
export function Documents() {
  const { t } = useTranslation("search");
  const queryClient = useQueryClient();
  const [text, setText] = useState("");
  const query = useDebounced(text.trim(), 250);
  const [target, setTarget] = useState<Target>();
  const results = useQuery({
    queryKey: [...searchKeys.admin.all, "documents", query],
    // Looking for something to remove is not a search worth remembering.
    queryFn: ({ signal }) =>
      searchClient().search({ query, page: { limit: 30 }, skipHistory: true }, { signal }),
    enabled: query.length > 0,
  });
  const remove = useMutation({
    mutationFn: (document: Target) =>
      searchAdminClient().deleteDocument({ kind: document.kind, id: document.id }),
    onSuccess: (response) => {
      if (response.existed) toast.success(t("staff.documents.removed"));
      else toast.message(t("staff.documents.notHeld"));
      return queryClient.invalidateQueries({ queryKey: searchKeys.all });
    },
    onError: (error) => toast.error(staffErrorMessage(error)),
    onSettled: () => setTarget(undefined),
  });
  const kindName = (kind: search.SearchKind) =>
    isKind(kind) ? t(`kinds.${kindSlug(kind)}`) : String(kind);
  const hits = results.data?.hits ?? [];

  return (
    <div className="grid grid-cols-1 items-start gap-4 xl:grid-cols-[minmax(0,1fr)_22rem]">
      <Pane title={t("staff.documents.findTitle")}>
        <div className="p-4">
          <label className="flex h-11 items-center gap-2 rounded-control border-2 border-line bg-surface px-3 focus-within:border-brand">
            <Search className="size-4 shrink-0 text-ink-faint" />
            <input
              type="search"
              aria-label={t("staff.documents.findLabel")}
              value={text}
              onChange={(event) => setText(event.target.value)}
              placeholder={t("staff.documents.findPlaceholder")}
              className="min-w-0 flex-1 bg-transparent text-sm outline-none placeholder:text-ink-faint [&::-webkit-search-cancel-button]:hidden"
              autoCapitalize="none"
              autoComplete="off"
              spellCheck={false}
            />
          </label>
          <p className="mt-2 px-1 text-xs leading-relaxed text-ink-faint">
            {t("staff.documents.hint")}
          </p>
        </div>
        {query.length === 0 ? null : results.isPending ? (
          <div className="flex flex-col gap-2 px-4 pb-4" aria-busy>
            <Skeleton className="h-14" />
            <Skeleton className="h-14" />
          </div>
        ) : results.isError ? (
          <EmptyState emoji="🌀" title={staffErrorMessage(results.error)} />
        ) : hits.length === 0 ? (
          <p className="px-5 pb-5 text-sm text-ink-soft">{t("staff.documents.none", { query })}</p>
        ) : (
          <ul
            aria-label={t("staff.documents.results")}
            className="divide-y-2 divide-line border-t-2 border-line"
          >
            {hits.map((hit) => (
              <li
                key={`${hit.kind}:${hit.id}`}
                data-testid="document-row"
                className="flex items-center gap-3 px-4 py-2.5"
              >
                <KindTile
                  kind={hit.kind}
                  className="size-9 rounded-xl"
                  iconClassName="size-[18px]"
                />
                <div className="min-w-0 flex-1">
                  <p className="truncate text-sm font-bold">{hit.title}</p>
                  <p className="truncate text-xs text-ink-soft">
                    {kindName(hit.kind)} · <span className="font-mono">{hit.id}</span>
                  </p>
                </div>
                <Button
                  size="sm"
                  variant="secondary"
                  icon={<Trash2 className="size-4" />}
                  onClick={() => setTarget({ kind: hit.kind, id: hit.id, title: hit.title })}
                  aria-label={t("staff.documents.removeNamed", { title: hit.title })}
                >
                  {t("staff.documents.remove")}
                </Button>
              </li>
            ))}
          </ul>
        )}
      </Pane>

      <ById onPick={setTarget} />

      <ConfirmDialog
        open={target !== undefined}
        onOpenChange={(open) => !open && setTarget(undefined)}
        title={t("staff.documents.confirmTitle")}
        description={t("staff.documents.confirmBody")}
        confirmLabel={t("staff.documents.remove")}
        danger
        loading={remove.isPending}
        onConfirm={() => target && remove.mutate(target)}
      >
        {target && (
          <p className="rounded-2xl bg-surface-2 px-3 py-2.5 text-sm">
            <span className="block truncate font-bold">{target.title ?? target.id}</span>
            <span className="block truncate text-xs text-ink-soft">
              {kindName(target.kind)} · <span className="font-mono">{target.id}</span>
            </span>
          </p>
        )}
      </ConfirmDialog>
    </div>
  );
}

/** Names a document by kind and ID, for content that cannot be found by text. */
function ById({ onPick }: { onPick(target: Target): void }) {
  const { t } = useTranslation("search");
  const selectId = useId();
  const [kind, setKind] = useState<Kind>(KINDS[0]);
  const [id, setId] = useState("");
  const submit = (event: FormEvent) => {
    event.preventDefault();
    if (id.trim()) onPick({ kind, id: id.trim() });
  };
  return (
    <Pane title={t("staff.documents.byIdTitle")}>
      <form onSubmit={submit} className="flex flex-col gap-3 p-4">
        <div className="flex flex-col gap-1.5">
          <label htmlFor={selectId} className="px-1 text-sm font-bold text-ink-soft">
            {t("staff.documents.kind")}
          </label>
          <select
            id={selectId}
            value={kind}
            onChange={(event) => setKind(Number(event.target.value) as Kind)}
            className="h-12 w-full rounded-control border-2 border-line bg-surface px-3 text-[15px] font-bold text-ink focus:border-brand focus:outline-none"
          >
            {KINDS.map((option) => (
              <option key={option} value={option}>
                {t(`kinds.${kindSlug(option)}`)}
              </option>
            ))}
          </select>
        </div>
        <TextField
          label={t("staff.documents.id")}
          value={id}
          onChange={(event) => setId(event.target.value)}
          placeholder="01a200b9-ba8c-7882-b900-6820e398de5d"
          autoCapitalize="none"
          autoComplete="off"
          spellCheck={false}
        />
        <Button
          type="submit"
          variant="secondary"
          disabled={!id.trim()}
          icon={<Trash2 className="size-4" />}
        >
          {t("staff.documents.remove")}
        </Button>
      </form>
    </Pane>
  );
}
