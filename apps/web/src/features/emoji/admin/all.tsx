import { useInfiniteQuery, useMutation } from "@tanstack/react-query";
import { Link } from "@tanstack/react-router";
import { emoji as emojiApi } from "@tokuly/api";
import { emojiKeys, emojiPageParam, shortcode, toCatalogEmoji } from "@tokuly/core/emoji";
import { MoreHorizontal, Plus } from "lucide-react";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { Button } from "#/components/ui/button";
import { ConfirmDialog } from "#/components/ui/confirm";
import { Dialog } from "#/components/ui/dialog";
import { TextArea, TextField } from "#/components/ui/field";
import { LoadMore } from "#/components/ui/load-more";
import { Menu } from "#/components/ui/menu";
import { EmptyState, Skeleton } from "#/components/ui/page";
import { SelectField } from "#/components/ui/select";
import { errorMessage } from "#/lib/errors";
import { EmojiImage } from "../emoji";
import { LICENSE_MAX } from "../emoji-form";
import { SearchBox, StateChip, useDebounced } from "../parts";
import { EmojiEditDialog, ReasonDialog } from "./dialogs";
import { History } from "./queue";
import { emojiAdminClient, UserLine, useAdminRefresh, useCategories } from "./shared";

const STATES = ["any", "approved", "pending", "rejected", "disabled"] as const;
type StateFilter = (typeof STATES)[number];
const STATE_VALUES: Record<StateFilter, emojiApi.EmojiState> = {
  any: emojiApi.EmojiState.UNSPECIFIED,
  approved: emojiApi.EmojiState.APPROVED,
  pending: emojiApi.EmojiState.PENDING,
  rejected: emojiApi.EmojiState.REJECTED,
  disabled: emojiApi.EmojiState.DISABLED,
};
/** Select values for "any category" and "without a category" (category names never start with a control character). */
const ANY = "\u0001any";
const NONE = "\u0001none";
/** The most emoji one bulk change takes. */
const BULK_MAX = 200;

type BulkChange = "category" | "aliases" | "license";

/** Every emoji in any state: find, edit, take out of the catalogue, put back, delete, and change many at once. */
export function AllEmoji({ initialQuery }: { initialQuery: string }) {
  const { t } = useTranslation("emoji");
  const refresh = useAdminRefresh();
  const categories = useCategories();
  const [state, setState] = useState<StateFilter>("any");
  const [text, setText] = useState(initialQuery);
  const [category, setCategory] = useState(ANY);
  const [selected, setSelected] = useState<string[]>([]);
  const [adding, setAdding] = useState(false);
  const [bulk, setBulk] = useState<BulkChange | null>(null);
  const [deleting, setDeleting] = useState(false);
  const query = useDebounced(text.trim(), 250);
  const filters = {
    state: STATE_VALUES[state],
    query,
    category: category === ANY ? undefined : category === NONE ? "" : category,
  };
  const list = useInfiniteQuery({
    queryKey: emojiKeys.adminList(filters),
    queryFn: ({ pageParam }) =>
      emojiAdminClient().listEmoji({ ...filters, page: { limit: 40, cursor: pageParam } }),
    initialPageParam: "",
    getNextPageParam: (last) => last.page?.nextCursor || undefined,
  });
  const items = list.data?.pages.flatMap((page) => page.emojis) ?? [];
  const names = categories.data?.categories.map((entry) => entry.name) ?? [];

  const toggle = (id: string) =>
    setSelected((current) =>
      current.includes(id)
        ? current.filter((item) => item !== id)
        : current.length < BULK_MAX
          ? [...current, id]
          : current,
    );
  const onError = (error: unknown) => toast.error(errorMessage(error, { namespace: "emoji" }));
  const bulkUpdate = useMutation({
    mutationFn: (change: emojiApi.BulkUpdateEmojiRequest["change"]) =>
      emojiAdminClient().bulkUpdateEmoji({ emojiIds: selected, change }),
    onSuccess: async ({ updatedCount }) => {
      toast.success(t("admin.bulkDone", { count: updatedCount }));
      setBulk(null);
      setSelected([]);
      await refresh();
    },
    onError,
  });
  const bulkDelete = useMutation({
    mutationFn: () => emojiAdminClient().bulkDeleteEmoji({ emojiIds: selected }),
    onSuccess: async ({ deletedCount }) => {
      toast.success(t("admin.bulkDeleted", { count: deletedCount }));
      setDeleting(false);
      setSelected([]);
      await refresh();
    },
    onError,
  });

  return (
    <>
      <div className="flex flex-wrap items-end gap-2">
        <SearchBox
          value={text}
          onChange={setText}
          placeholder={t("admin.search")}
          className="h-12 min-w-56 flex-1"
        />
        <SelectField<StateFilter>
          className="w-40"
          value={state}
          onChange={setState}
          options={STATES.map((value) => ({
            value,
            label: t(value === "any" ? "admin.anyState" : `state.${value}`),
          }))}
        />
        <SelectField
          className="w-44"
          value={category}
          onChange={setCategory}
          options={[
            { value: ANY, label: t("browse.allCategories") },
            ...names.map((name) => ({ value: name, label: name })),
            { value: NONE, label: t("browse.uncategorized") },
          ]}
        />
        <Button icon={<Plus className="size-4" />} onClick={() => setAdding(true)}>
          {t("admin.add")}
        </Button>
      </div>

      {selected.length > 0 && (
        <div
          className="sticker sticky top-16 z-10 mt-3 flex flex-wrap items-center gap-2 rounded-2xl bg-brand-soft px-3 py-2"
          role="toolbar"
          aria-label={t("admin.bulk")}
        >
          <p className="mr-auto text-sm font-black">
            {t("admin.selected", { count: selected.length })}
          </p>
          <Menu
            trigger={
              <Button size="sm" variant="secondary">
                {t("admin.bulk")}
              </Button>
            }
            actions={[
              { label: t("admin.bulkCategory"), onSelect: () => setBulk("category") },
              { label: t("admin.bulkAliases"), onSelect: () => setBulk("aliases") },
              { label: t("admin.bulkLicense"), onSelect: () => setBulk("license") },
              {
                label: t("admin.bulkSensitiveOn"),
                onSelect: () => bulkUpdate.mutate({ case: "setSensitive", value: true }),
              },
              {
                label: t("admin.bulkSensitiveOff"),
                onSelect: () => bulkUpdate.mutate({ case: "setSensitive", value: false }),
              },
              {
                label: t("admin.bulkLocalOn"),
                onSelect: () => bulkUpdate.mutate({ case: "setLocalOnly", value: true }),
              },
              {
                label: t("admin.bulkLocalOff"),
                onSelect: () => bulkUpdate.mutate({ case: "setLocalOnly", value: false }),
              },
              { label: t("admin.bulkDelete"), danger: true, onSelect: () => setDeleting(true) },
            ]}
          />
          <Button size="sm" variant="ghost" onClick={() => setSelected([])}>
            {t("admin.clearSelection")}
          </Button>
        </div>
      )}

      <div className="mt-3 overflow-hidden rounded-2xl border-2 border-line bg-surface">
        {list.isPending ? (
          <div className="flex flex-col gap-2 p-3">
            <Skeleton className="h-14" />
            <Skeleton className="h-14" />
            <Skeleton className="h-14" />
          </div>
        ) : list.isError ? (
          <EmptyState emoji="😵‍💫" title={t("browse.error")} body={errorMessage(list.error)} />
        ) : items.length === 0 ? (
          <EmptyState emoji="🫥" title={t("admin.listEmpty")} />
        ) : (
          <ul className="divide-y-2 divide-line">
            {items.map(
              (item) =>
                item.emoji && (
                  <Row
                    key={item.emoji.id}
                    detail={item}
                    selected={selected.includes(item.emoji.id)}
                    onToggle={toggle}
                  />
                ),
            )}
          </ul>
        )}
      </div>
      <LoadMore query={list} />

      {adding && <EmojiEditDialog open onOpenChange={setAdding} />}
      {bulk && (
        <BulkDialog
          change={bulk}
          count={selected.length}
          categories={names}
          loading={bulkUpdate.isPending}
          onClose={() => setBulk(null)}
          onApply={(change) => bulkUpdate.mutate(change)}
        />
      )}
      <ConfirmDialog
        open={deleting}
        onOpenChange={setDeleting}
        title={t("admin.bulkDeleteTitle", { count: selected.length })}
        description={t("admin.deleteBody")}
        confirmLabel={t("admin.delete")}
        danger
        loading={bulkDelete.isPending}
        onConfirm={() => bulkDelete.mutate()}
      />
    </>
  );
}

function Row({
  detail,
  selected,
  onToggle,
}: {
  detail: emojiApi.EmojiDetail;
  selected: boolean;
  onToggle(id: string): void;
}) {
  const { t } = useTranslation("emoji");
  const refresh = useAdminRefresh();
  const entry = toCatalogEmoji(detail.emoji as emojiApi.CustomEmoji);
  const code = shortcode(entry.name);
  const [dialog, setDialog] = useState<"edit" | "disable" | "delete" | "history" | null>(null);
  const approved = detail.state === emojiApi.EmojiState.APPROVED;
  const disabled = detail.state === emojiApi.EmojiState.DISABLED;

  const done = (key: string) => async () => {
    setDialog(null);
    toast.success(t(key, { code }));
    await refresh();
  };
  const onError = (error: unknown) => toast.error(errorMessage(error, { namespace: "emoji" }));
  const disable = useMutation({
    mutationFn: (reason: string) => emojiAdminClient().disableEmoji({ emojiId: entry.id, reason }),
    onSuccess: done("admin.disabled"),
    onError,
  });
  const enable = useMutation({
    mutationFn: () => emojiAdminClient().enableEmoji({ emojiId: entry.id }),
    onSuccess: done("admin.enabled"),
    onError,
  });
  const approve = useMutation({
    mutationFn: () => emojiAdminClient().approveEmoji({ emojiId: entry.id }),
    onSuccess: done("admin.approved"),
    onError,
  });
  const remove = useMutation({
    mutationFn: () => emojiAdminClient().deleteEmoji({ emojiId: entry.id }),
    onSuccess: done("admin.deleted"),
    onError,
  });

  return (
    <li
      data-testid="admin-emoji"
      data-name={entry.name}
      className="flex items-center gap-3 px-3 py-2.5"
    >
      <input
        type="checkbox"
        checked={selected}
        onChange={() => onToggle(entry.id)}
        aria-label={t("admin.select", { code })}
        className="size-5 shrink-0 accent-[var(--brand)]"
      />
      <span className="grid size-11 shrink-0 place-items-center rounded-xl bg-surface-2">
        <EmojiImage emoji={entry} size={32} reveal still className="size-8" />
      </span>
      <div className="min-w-0 flex-1">
        <div className="flex flex-wrap items-center gap-x-2 gap-y-1">
          {approved ? (
            <Link
              to="/emoji/$name"
              params={{ name: emojiPageParam(entry.name) }}
              className="truncate font-mono font-bold hover:underline"
            >
              {code}
            </Link>
          ) : (
            <span className="truncate font-mono font-bold">{code}</span>
          )}
          <StateChip state={detail.state} />
        </div>
        <p className="mt-0.5 flex flex-wrap items-center gap-x-2 text-xs text-ink-soft">
          <span>{entry.category || t("browse.uncategorized")}</span>
          <span>{t("detail.useCount", { count: Number(detail.useCount) })}</span>
          {detail.author && <UserLine user={detail.author} />}
          {entry.sensitive && <span className="font-bold text-coral">{t("flags.sensitive")}</span>}
          {entry.localOnly && <span>{t("flags.localOnly")}</span>}
        </p>
        {detail.reviewNote && !approved && (
          <p className="mt-0.5 truncate text-xs text-ink-faint">{detail.reviewNote}</p>
        )}
      </div>
      <Menu
        trigger={
          <Button variant="ghost" size="icon-sm" aria-label={t("admin.actions", { code })}>
            <MoreHorizontal className="size-5" />
          </Button>
        }
        actions={[
          { label: t("admin.edit"), onSelect: () => setDialog("edit") },
          ...(approved
            ? [{ label: t("admin.disable"), onSelect: () => setDialog("disable") }]
            : []),
          ...(disabled ? [{ label: t("admin.enable"), onSelect: () => enable.mutate() }] : []),
          ...(detail.state === emojiApi.EmojiState.PENDING ||
          detail.state === emojiApi.EmojiState.REJECTED
            ? [{ label: t("admin.approve"), onSelect: () => approve.mutate() }]
            : []),
          { label: t("admin.historyTitle"), onSelect: () => setDialog("history") },
          { label: t("admin.delete"), danger: true, onSelect: () => setDialog("delete") },
        ]}
      />
      {dialog === "edit" && (
        <EmojiEditDialog detail={detail} open onOpenChange={() => setDialog(null)} />
      )}
      <ReasonDialog
        open={dialog === "disable"}
        onOpenChange={() => setDialog(null)}
        title={t("admin.disableTitle", { code })}
        confirmLabel={t("admin.disable")}
        loading={disable.isPending}
        onConfirm={(reason) => disable.mutate(reason)}
      />
      <ConfirmDialog
        open={dialog === "delete"}
        onOpenChange={() => setDialog(null)}
        title={t("admin.deleteTitle", { code })}
        description={t("admin.deleteBody")}
        confirmLabel={t("admin.delete")}
        danger
        loading={remove.isPending}
        onConfirm={() => remove.mutate()}
      />
      <Dialog
        open={dialog === "history"}
        onOpenChange={() => setDialog(null)}
        title={t("admin.historyTitle")}
        description={code}
      >
        {detail.history.length > 0 ? (
          <History records={detail.history} />
        ) : (
          <p className="text-sm text-ink-soft">{t("admin.historyEmpty")}</p>
        )}
      </Dialog>
    </li>
  );
}

function BulkDialog({
  change,
  count,
  categories,
  loading,
  onClose,
  onApply,
}: {
  change: BulkChange;
  count: number;
  categories: string[];
  loading: boolean;
  onClose(): void;
  onApply(change: emojiApi.BulkUpdateEmojiRequest["change"]): void;
}) {
  const { t } = useTranslation("emoji");
  const { t: tCommon } = useTranslation("common");
  const [category, setCategory] = useState("");
  const [text, setText] = useState("");
  const apply = () => {
    if (change === "category") onApply({ case: "setCategory", value: category });
    else if (change === "license") onApply({ case: "setLicense", value: text.trim() });
    else
      onApply({
        case: "addAliases",
        value: {
          $typeName: "tokuly.emoji.v1.AliasList",
          aliases: text
            .split(/[\s,、，]+/)
            .map((word) => word.trim().toLowerCase())
            .filter(Boolean),
        },
      });
  };
  return (
    <Dialog
      open
      onOpenChange={onClose}
      title={t(
        change === "category"
          ? "admin.bulkCategory"
          : change === "aliases"
            ? "admin.bulkAliases"
            : "admin.bulkLicense",
      )}
      description={t("admin.selected", { count })}
      footer={
        <>
          <Button variant="ghost" onClick={onClose} disabled={loading}>
            {tCommon("actions.cancel")}
          </Button>
          <Button loading={loading} disabled={change === "aliases" && !text.trim()} onClick={apply}>
            {t("admin.apply")}
          </Button>
        </>
      }
    >
      {change === "category" ? (
        <SelectField
          label={t("fields.category")}
          value={category}
          onChange={setCategory}
          options={[
            { value: "", label: t("form.noCategory") },
            ...categories.map((name) => ({ value: name, label: name })),
          ]}
        />
      ) : change === "aliases" ? (
        <TextField
          label={t("fields.aliases")}
          value={text}
          onChange={(event) => setText(event.target.value)}
          hint={t("admin.bulkAliasesHint")}
        />
      ) : (
        <TextArea
          label={t("fields.license")}
          value={text}
          maxLength={LICENSE_MAX}
          onChange={(event) => setText(event.target.value)}
        />
      )}
    </Dialog>
  );
}
