import { useMutation, useQueryClient } from "@tanstack/react-query";
import { ask, type MessageInitShape, toMillis } from "@tokuly/api";
import { Plus, X } from "lucide-react";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { Button } from "#/components/ui/button";
import { Card } from "#/components/ui/card";
import { TextField } from "#/components/ui/field";
import { LoadMore } from "#/components/ui/load-more";
import { Page, Skeleton } from "#/components/ui/page";
import { SelectField } from "#/components/ui/select";
import { SwitchRow } from "#/components/ui/switch";
import { compactAge } from "#/lib/format";
import { askKeys, boxClient, useSenderBlocks, useSettings } from "./api";
import { BOX_PROMPT_MAX, charCount, MUTED_WORD_MAX, MUTED_WORDS_MAX, parseMutedWords } from "./model";
import { Counter, SectionTitle, toastError } from "./parts";

type SettingsChange = MessageInitShape<typeof ask.UpdateSettingsRequestSchema>;

/** Settings of the question box, the word filter, compliment polls, and the list of blocked senders. */
export function SettingsScreen() {
  const { t } = useTranslation("ask");
  const queryClient = useQueryClient();
  const { data: settings, isPending } = useSettings();

  // Switches flip at once; the server's answer (or a failure) settles what is shown.
  const update = useMutation({
    mutationFn: async (change: SettingsChange) => (await boxClient().updateSettings(change)).settings!,
    onSuccess: (saved) => {
      queryClient.setQueryData(askKeys.settings, saved);
      void queryClient.invalidateQueries({ queryKey: ["ask", "box"] });
    },
    onError: (error) => {
      toastError(error);
      void queryClient.invalidateQueries({ queryKey: askKeys.settings });
    },
  });
  const flip = (field: "boxOpen" | "allowSigned" | "allowImages" | "inPolls" | "allowHints") => (value: boolean) => {
    queryClient.setQueryData<ask.Settings>(askKeys.settings, (known) => (known ? { ...known, [field]: value } : known));
    update.mutate({ [field]: value });
  };

  if (isPending || !settings) {
    return (
      <Page title={t("settings.title")} back="/ask">
        <Skeleton className="h-64" />
      </Page>
    );
  }

  return (
    <Page title={t("settings.title")} back="/ask">
      <SectionTitle className="mt-0">{t("settings.box")}</SectionTitle>
      <Card className="flex flex-col gap-3 p-3">
        <SwitchRow label={t("hub.openSwitch")} hint={t("hub.openHint")} checked={settings.boxOpen} onChange={flip("boxOpen")} />
        <PromptField key={settings.boxPrompt} saved={settings.boxPrompt} saving={update.isPending} onSave={(boxPrompt) => update.mutate({ boxPrompt }, { onSuccess: () => toast.success(t("settings.promptSaved")) })} />
        <SelectField
          className="px-3"
          label={t("settings.audience")}
          value={settings.audience === ask.AskerAudience.SIGNED_IN ? "signedIn" : "friends"}
          onChange={(value) => {
            const audience = value === "signedIn" ? ask.AskerAudience.SIGNED_IN : ask.AskerAudience.FRIENDS;
            queryClient.setQueryData<ask.Settings>(askKeys.settings, (known) => (known ? { ...known, audience } : known));
            update.mutate({ audience });
          }}
          options={[
            { value: "friends", label: t("settings.audienceFriends"), hint: t("settings.audienceFriendsHint") },
            { value: "signedIn", label: t("settings.audienceSignedIn"), hint: t("settings.audienceSignedInHint") },
          ]}
        />
        <SwitchRow label={t("settings.allowSigned")} hint={t("settings.allowSignedHint")} checked={settings.allowSigned} onChange={flip("allowSigned")} />
        <SwitchRow label={t("settings.allowImages")} hint={t("settings.allowImagesHint")} checked={settings.allowImages} onChange={flip("allowImages")} />
      </Card>

      <SectionTitle>{t("settings.filter")}</SectionTitle>
      <Card>
        <MutedWords words={settings.mutedWords} saving={update.isPending} onChange={(words) => update.mutate({ mutedWords: { words } })} />
      </Card>

      <SectionTitle>{t("settings.polls")}</SectionTitle>
      <Card className="p-3">
        <SwitchRow label={t("settings.inPolls")} hint={t("settings.inPollsHint")} checked={settings.inPolls} onChange={flip("inPolls")} />
        <SwitchRow label={t("settings.allowHints")} hint={t("settings.allowHintsHint")} checked={settings.allowHints} onChange={flip("allowHints")} />
      </Card>

      <SectionTitle>{t("settings.blocks")}</SectionTitle>
      <SenderBlocks />
    </Page>
  );
}

/** The line above the box. Saved with its button (or Enter), not on every key. */
function PromptField({ saved, saving, onSave }: { saved: string; saving: boolean; onSave(prompt: string): void }) {
  const { t } = useTranslation("ask");
  const [value, setValue] = useState(saved);
  const changed = value.trim() !== saved;
  const tooLong = charCount(value.trim()) > BOX_PROMPT_MAX;
  return (
    <form
      className="flex items-start gap-2 px-3"
      onSubmit={(event) => {
        event.preventDefault();
        if (changed && !tooLong) onSave(value.trim());
      }}
    >
      <TextField
        className="min-w-0 flex-1"
        label={t("settings.promptLabel")}
        placeholder={t("settings.promptPlaceholder")}
        value={value}
        onChange={(event) => setValue(event.target.value)}
        hint={
          <span className="flex justify-between gap-2">
            <span>{t("settings.promptHint", { max: BOX_PROMPT_MAX })}</span>
            <Counter text={value.trim()} max={BOX_PROMPT_MAX} />
          </span>
        }
      />
      <Button type="submit" className="mt-[1.85rem]" variant="secondary" disabled={!changed || tooLong} loading={saving && changed}>
        {t("actions.save", { ns: "common" })}
      </Button>
    </form>
  );
}

/** The owner's word filter: chips that can be taken out, and a field to add more (several at once, separated by commas). */
function MutedWords({ words, saving, onChange }: { words: string[]; saving: boolean; onChange(words: string[]): void }) {
  const { t } = useTranslation("ask");
  const [input, setInput] = useState("");

  const add = () => {
    const added = parseMutedWords(input).filter((word) => !words.includes(word));
    if (added.length === 0) return setInput("");
    if (added.some((word) => charCount(word) > MUTED_WORD_MAX)) return void toast.error(t("settings.filterTooLong", { max: MUTED_WORD_MAX }));
    if (words.length + added.length > MUTED_WORDS_MAX) return void toast.error(t("settings.filterTooMany", { max: MUTED_WORDS_MAX }));
    onChange([...words, ...added]);
    setInput("");
  };

  return (
    <div>
      <p className="text-sm leading-relaxed text-ink-soft">{t("settings.filterLead")}</p>
      <form
        className="mt-3 flex gap-2"
        onSubmit={(event) => {
          event.preventDefault();
          add();
        }}
      >
        <TextField className="min-w-0 flex-1" aria-label={t("settings.filter")} placeholder={t("settings.filterPlaceholder")} value={input} onChange={(event) => setInput(event.target.value)} />
        <Button type="submit" variant="secondary" className="mt-0.5" icon={<Plus className="size-4" />} disabled={!input.trim()} loading={saving && Boolean(input.trim())}>
          {t("settings.filterAdd")}
        </Button>
      </form>
      {words.length === 0 ? (
        <p className="mt-3 text-sm font-bold text-ink-faint">{t("settings.filterEmpty")}</p>
      ) : (
        <ul className="mt-3 flex flex-wrap gap-2" data-testid="muted-words">
          {words.map((word) => (
            <li key={word} className="inline-flex items-center gap-1 rounded-full bg-surface-2 py-1 pr-1 pl-3 text-sm font-bold">
              {word}
              <button type="button" aria-label={t("settings.filterRemove", { word })} onClick={() => onChange(words.filter((known) => known !== word))} className="grid size-6 place-items-center rounded-full hover:bg-surface-3">
                <X className="size-3.5" />
              </button>
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}

/** Blocks of unnamed senders: what each was made from, never who. */
function SenderBlocks() {
  const { t } = useTranslation("ask");
  const queryClient = useQueryClient();
  const query = useSenderBlocks();
  const blocks = query.data?.pages.flatMap((page) => page.blocks) ?? [];
  const unblock = useMutation({
    mutationFn: (blockId: string) => boxClient().unblockSender({ blockId }),
    onSuccess: () => toast.success(t("settings.unblocked")),
    onError: toastError,
    onSettled: () => queryClient.invalidateQueries({ queryKey: askKeys.blocks }),
  });
  const SOURCES: Record<number, string> = {
    [ask.BlockSource.QUESTION]: t("settings.source.question"),
    [ask.BlockSource.PROMPT_RESPONSE]: t("settings.source.response"),
    [ask.BlockSource.FLAME]: t("settings.source.flame"),
  };

  return (
    <div>
      <p className="mb-3 px-1 text-sm leading-relaxed text-ink-soft">{t("settings.blocksLead")}</p>
      {query.isPending ? (
        <Skeleton className="h-16" />
      ) : blocks.length === 0 ? (
        <p className="rounded-card bg-surface px-4 py-6 text-center text-sm font-bold text-ink-soft">🕊️ {t("settings.blocksEmpty")}</p>
      ) : (
        <ul className="flex flex-col gap-1 rounded-card bg-surface p-2" data-testid="sender-blocks">
          {blocks.map((block) => (
            <li key={block.id} className="flex items-center gap-3 rounded-2xl px-3 py-2">
              <span className="min-w-0 flex-1">
                <span className="block truncate font-bold">{block.preview}</span>
                <span className="block text-xs font-bold text-ink-faint">
                  {SOURCES[block.source] ?? ""} ・ {compactAge(toMillis(block.createdAt))}
                </span>
              </span>
              <Button size="sm" variant="secondary" loading={unblock.isPending && unblock.variables === block.id} onClick={() => unblock.mutate(block.id)}>
                {t("settings.unblock")}
              </Button>
            </li>
          ))}
          <LoadMore query={query} />
        </ul>
      )}
    </div>
  );
}
