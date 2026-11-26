import { useMutation, useQueryClient } from "@tanstack/react-query";
import { useNavigate } from "@tanstack/react-router";
import { type common, notes } from "@tokuly/api";
import { charCount, listKeys, notesKeys } from "@tokuly/core/notes";
import { Plus, X } from "lucide-react";
import { useEffect, useId, useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { Avatar } from "#/components/ui/avatar";
import { Button } from "#/components/ui/button";
import { Dialog } from "#/components/ui/dialog";
import { TextField } from "#/components/ui/field";
import { ListRow } from "#/components/ui/page";
import { SwitchRow } from "#/components/ui/switch";
import { cn } from "#/lib/cn";
import { errorMessage } from "#/lib/errors";
import { ANTENNA_LIMITS, antennaClient, useLists, useUserSearch } from "./lists-api";
import { LISTS_NS } from "./lists-editor";

/** Words as typed in one field: split on blank space, duplicates once. */
export function splitWords(text: string): string[] {
  return [...new Set(text.split(/\s+/u).filter(Boolean))];
}

export const SOURCES = [
  { value: notes.AntennaSource.EVERYONE, key: "everyone" },
  { value: notes.AntennaSource.FOLLOWING, key: "following" },
  { value: notes.AntennaSource.LIST, key: "list" },
  { value: notes.AntennaSource.USERS, key: "users" },
] as const;

export function sourceKey(source: notes.AntennaSource): (typeof SOURCES)[number]["key"] {
  return SOURCES.find((item) => item.value === source)?.key ?? "everyone";
}

/** Creating an antenna (`antenna` absent) or changing one. */
export function AntennaEditor({
  open,
  onOpenChange,
  antenna,
}: {
  open: boolean;
  onOpenChange(open: boolean): void;
  antenna?: notes.Antenna;
}) {
  const { t } = useTranslation("notelists");
  return (
    <Dialog
      open={open}
      onOpenChange={onOpenChange}
      title={antenna ? t("antennaEditor.editTitle") : t("antennaEditor.createTitle")}
    >
      {open && <AntennaForm antenna={antenna} onDone={() => onOpenChange(false)} />}
    </Dialog>
  );
}

function AntennaForm({ antenna, onDone }: { antenna?: notes.Antenna; onDone(): void }) {
  const { t } = useTranslation("notelists");
  const queryClient = useQueryClient();
  const navigate = useNavigate();
  const radioName = useId();
  const [name, setName] = useState(antenna?.name ?? "");
  const [groups, setGroups] = useState<string[]>(
    antenna?.keywords.length ? antenna.keywords.map((group) => group.words.join(" ")) : [""],
  );
  const [exclude, setExclude] = useState(antenna?.excludeWords.join(" ") ?? "");
  const [source, setSource] = useState<notes.AntennaSource>(
    antenna?.source || notes.AntennaSource.EVERYONE,
  );
  const [listId, setListId] = useState(antenna?.listId ?? "");
  const [users, setUsers] = useState<common.UserSummary[]>(antenna?.users ?? []);
  const [withReplies, setWithReplies] = useState(antenna?.withReplies ?? false);
  const [mediaOnly, setMediaOnly] = useState(antenna?.mediaOnly ?? false);
  const [notify, setNotify] = useState(antenna?.notify ?? false);
  const lists = useLists();

  const keywords = groups.map(splitWords).filter((words) => words.length > 0);
  const needsKeywords =
    source === notes.AntennaSource.EVERYONE || source === notes.AntennaSource.FOLLOWING;
  const problem =
    charCount(name.trim()) === 0 || charCount(name.trim()) > ANTENNA_LIMITS.name
      ? "name"
      : needsKeywords && keywords.length === 0
        ? t("antennaEditor.needKeyword")
        : source === notes.AntennaSource.LIST && !listId
          ? t("antennaEditor.needList")
          : source === notes.AntennaSource.USERS && users.length === 0
            ? t("antennaEditor.needUsers")
            : null;

  const save = useMutation({
    mutationFn: async () => {
      const input = {
        name,
        keywords: keywords.map((words) => ({ words })),
        excludeWords: splitWords(exclude),
        source,
        listId: source === notes.AntennaSource.LIST ? listId : "",
        userIds: source === notes.AntennaSource.USERS ? users.map((user) => user.id) : [],
        withReplies,
        mediaOnly,
        notify,
      };
      return antenna
        ? (await antennaClient().updateAntenna({ antennaId: antenna.id, antenna: input })).antenna
        : (await antennaClient().createAntenna({ antenna: input })).antenna;
    },
    onSuccess: async (saved) => {
      if (saved) queryClient.setQueryData(listKeys.antenna(saved.id), saved);
      await queryClient.invalidateQueries({ queryKey: listKeys.antennas, exact: true });
      toast.success(antenna ? t("toast.antennaSaved") : t("toast.antennaCreated"));
      onDone();
      if (!antenna && saved)
        void navigate({ to: "/notes/antennas/$antennaId", params: { antennaId: saved.id } });
      if (antenna)
        await queryClient.resetQueries({
          queryKey: notesKeys.timeline({ kind: "antenna", antennaId: antenna.id }),
        });
    },
    onError: (error) => toast.error(errorMessage(error, LISTS_NS)),
  });

  return (
    <form
      className="flex flex-col gap-4"
      onSubmit={(event) => {
        event.preventDefault();
        if (!problem) save.mutate();
      }}
    >
      <TextField
        label={t("antennaEditor.name")}
        placeholder={t("antennaEditor.namePlaceholder")}
        value={name}
        hint={`${charCount(name.trim())}/${ANTENNA_LIMITS.name}`}
        onChange={(event) => setName(event.target.value)}
        autoFocus
      />

      <fieldset className="flex flex-col gap-1.5">
        <legend className="mb-1.5 px-1 text-sm font-bold text-ink-soft">
          {t("antennaEditor.source")}
        </legend>
        <div className="grid grid-cols-2 gap-2">
          {SOURCES.map((item) => (
            <label
              key={item.key}
              className={cn(
                "flex cursor-pointer items-center gap-2 rounded-2xl border-2 px-3 py-2.5 text-sm font-bold",
                source === item.value ? "border-brand bg-surface-2" : "border-line",
              )}
            >
              <input
                type="radio"
                name={radioName}
                className="accent-[var(--color-brand)]"
                checked={source === item.value}
                onChange={() => setSource(item.value)}
              />
              {t(`antennas.source.${item.key}`)}
            </label>
          ))}
        </div>
      </fieldset>

      {source === notes.AntennaSource.LIST && (
        <fieldset className="flex flex-col gap-1">
          <legend className="mb-1.5 px-1 text-sm font-bold text-ink-soft">
            {t("antennaEditor.chooseList")}
          </legend>
          {(lists.data ?? []).map((list) => (
            <label
              key={list.id}
              className="flex cursor-pointer items-center gap-2 rounded-2xl px-3 py-2 font-bold hover:bg-surface-2"
            >
              <input
                type="radio"
                name={`${radioName}-list`}
                checked={listId === list.id}
                onChange={() => setListId(list.id)}
              />
              {list.name}
            </label>
          ))}
        </fieldset>
      )}

      {source === notes.AntennaSource.USERS && <UsersPicker users={users} onChange={setUsers} />}

      <div className="flex flex-col gap-2">
        <p className="px-1 text-sm font-bold text-ink-soft">{t("antennaEditor.keywords")}</p>
        <p className="px-1 text-xs text-ink-faint">{t("antennaEditor.keywordsHint")}</p>
        {groups.map((group, index) => (
          <div key={index} className="flex items-end gap-2">
            <TextField
              className="flex-1"
              label={t("antennaEditor.group", { index: index + 1 })}
              placeholder={t("antennaEditor.groupPlaceholder")}
              value={group}
              onChange={(event) =>
                setGroups((current) =>
                  current.map((item, at) => (at === index ? event.target.value : item)),
                )
              }
            />
            {groups.length > 1 && (
              <Button
                type="button"
                size="icon"
                variant="ghost"
                aria-label={t("antennaEditor.removeGroup", { index: index + 1 })}
                onClick={() => setGroups((current) => current.filter((_, at) => at !== index))}
              >
                <X className="size-4" />
              </Button>
            )}
          </div>
        ))}
        {groups.length < ANTENNA_LIMITS.groups && (
          <Button
            type="button"
            variant="ghost"
            size="sm"
            className="self-start"
            icon={<Plus className="size-4" />}
            onClick={() => setGroups((current) => [...current, ""])}
          >
            {t("antennaEditor.addGroup")}
          </Button>
        )}
      </div>

      <TextField
        label={t("antennaEditor.exclude")}
        placeholder={t("antennaEditor.excludePlaceholder")}
        hint={t("antennaEditor.excludeHint", { max: ANTENNA_LIMITS.exclude })}
        value={exclude}
        onChange={(event) => setExclude(event.target.value)}
      />

      <div className="flex flex-col">
        <SwitchRow
          label={t("antennaEditor.withReplies")}
          checked={withReplies}
          onChange={setWithReplies}
        />
        <SwitchRow
          label={t("antennaEditor.mediaOnly")}
          checked={mediaOnly}
          onChange={setMediaOnly}
        />
        <SwitchRow label={t("antennaEditor.notify")} checked={notify} onChange={setNotify} />
      </div>

      {problem && problem !== "name" && (
        <p className="px-1 text-sm font-bold text-danger">{problem}</p>
      )}
      <Button type="submit" block disabled={Boolean(problem)} loading={save.isPending}>
        {antenna ? t("antennaEditor.save") : t("antennaEditor.create")}
      </Button>
    </form>
  );
}

function useDebounced(value: string, ms = 300): string {
  const [debounced, setDebounced] = useState(value);
  useEffect(() => {
    const timer = setTimeout(() => setDebounced(value.trim()), ms);
    return () => clearTimeout(timer);
  }, [value, ms]);
  return debounced;
}

/** The accounts of a USERS antenna: chips of the chosen ones, and a search to add more. */
function UsersPicker({
  users,
  onChange,
}: {
  users: common.UserSummary[];
  onChange(users: common.UserSummary[]): void;
}) {
  const { t } = useTranslation("notelists");
  const [text, setText] = useState("");
  const results = useUserSearch(useDebounced(text));
  const chosen = new Set(users.map((user) => user.id));
  return (
    <div className="flex flex-col gap-2">
      <p className="px-1 text-sm font-bold text-ink-soft">
        {t("antennaEditor.users")} ·{" "}
        {t("antennaEditor.usersHint", { count: users.length, max: ANTENNA_LIMITS.users })}
      </p>
      {users.length > 0 && (
        <div className="flex flex-wrap gap-1.5">
          {users.map((user) => (
            <span
              key={user.id}
              className="inline-flex items-center gap-1.5 rounded-full bg-surface-2 py-1 pr-1 pl-1.5 text-sm font-bold"
            >
              <Avatar user={user} size="xs" />
              {user.displayName || `@${user.handle}`}
              <button
                type="button"
                aria-label={`${t("list.removeMember", { name: user.displayName || user.handle })}`}
                className="grid size-6 place-items-center rounded-full hover:bg-surface-3"
                onClick={() => onChange(users.filter((item) => item.id !== user.id))}
              >
                <X className="size-3.5" />
              </button>
            </span>
          ))}
        </div>
      )}
      <TextField
        aria-label={t("add.search")}
        placeholder={t("add.search")}
        value={text}
        onChange={(event) => setText(event.target.value)}
      />
      {(results.data ?? [])
        .filter((result) => result.user && !chosen.has(result.user.id))
        .slice(0, 5)
        .map((result) => (
          <ListRow
            key={result.user!.id}
            leading={<Avatar user={result.user} size="sm" />}
            title={result.user!.displayName || `@${result.user!.handle}`}
            subtitle={`@${result.user!.handle}`}
            trailing={
              <Button
                type="button"
                size="sm"
                disabled={users.length >= ANTENNA_LIMITS.users}
                onClick={() => {
                  onChange([...users, result.user!]);
                  setText("");
                }}
              >
                {t("add.add")}
              </Button>
            }
          />
        ))}
    </div>
  );
}
