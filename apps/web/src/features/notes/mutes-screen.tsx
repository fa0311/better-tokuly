import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import type { notes } from "@tokuly/api";
import { notesKeys } from "@tokuly/core/notes";
import { Plus, X } from "lucide-react";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { Avatar } from "#/components/ui/avatar";
import { Button } from "#/components/ui/button";
import { Page } from "#/components/ui/page";
import { errorMessage } from "#/lib/errors";
import { muteClient } from "./api";

/** `/settings/notes`: muted words (plain or patterns) and people whose renotes are hidden. */
export function MutesScreen() {
  const { t } = useTranslation("notes");
  const queryClient = useQueryClient();
  const settings = useQuery({
    queryKey: notesKeys.mutes,
    queryFn: () => muteClient().getMuteSettings({}),
  });
  const [word, setWord] = useState("");
  const [regex, setRegex] = useState(false);
  const words = settings.data?.words ?? [];
  const refresh = () => {
    void queryClient.invalidateQueries({ queryKey: notesKeys.mutes });
    void queryClient.invalidateQueries({ queryKey: notesKeys.timelines });
  };
  const saveWords = useMutation({
    mutationFn: (next: Pick<notes.MutedWord, "text" | "regex">[]) =>
      muteClient().updateMutedWords({ words: next }),
    onSuccess: () => {
      setWord("");
      refresh();
      toast.success(t("mutes.saved"));
    },
    onError: (error) => toast.error(errorMessage(error, { namespace: "notes" })),
  });
  const show = useMutation({
    mutationFn: (userId: string) => muteClient().showRenotesFrom({ userId }),
    onSuccess: refresh,
    onError: (error) => toast.error(errorMessage(error, { namespace: "notes" })),
  });
  const plain = words.map(({ text, regex }) => ({ text, regex }));

  return (
    <Page title={t("mutes.title")} back="/settings">
      <section className="rounded-card bg-surface p-4">
        <h2 className="font-black">{t("mutes.words")}</h2>
        <p className="text-sm text-ink-soft">{t("mutes.wordsHint")}</p>
        <form
          className="mt-3 flex flex-wrap items-center gap-2"
          onSubmit={(event) => {
            event.preventDefault();
            if (word.trim()) saveWords.mutate([...plain, { text: word.trim(), regex }]);
          }}
        >
          <input
            value={word}
            maxLength={100}
            aria-label={t("mutes.newWord")}
            placeholder={t("mutes.newWord")}
            onChange={(event) => setWord(event.target.value)}
            className="h-10 min-w-0 flex-1 rounded-xl border-2 border-line bg-surface px-3 font-bold outline-none focus:border-ink"
          />
          <label className="flex items-center gap-1.5 text-sm font-bold">
            <input
              type="checkbox"
              checked={regex}
              onChange={(event) => setRegex(event.target.checked)}
            />
            {t("mutes.regex")}
          </label>
          <Button type="submit" size="sm" loading={saveWords.isPending} disabled={!word.trim()}>
            <Plus className="size-4" />
            {t("mutes.add")}
          </Button>
        </form>
        <ul className="mt-3 flex flex-wrap gap-1.5" data-testid="muted-words">
          {words.map((muted, index) => (
            <li
              key={`${muted.regex}:${muted.text}`}
              className="flex items-center gap-1 rounded-full bg-surface-2 py-1 pr-1 pl-3 text-sm font-bold"
            >
              {muted.regex ? <code>/{muted.text}/</code> : muted.text}
              <button
                type="button"
                aria-label={t("mutes.remove", { word: muted.text })}
                onClick={() => saveWords.mutate(plain.filter((_, i) => i !== index))}
                className="grid size-6 place-items-center rounded-full hover:bg-surface-3"
              >
                <X className="size-3.5" />
              </button>
            </li>
          ))}
        </ul>
      </section>
      <section className="mt-4 rounded-card bg-surface p-4">
        <h2 className="font-black">{t("mutes.renotes")}</h2>
        <p className="text-sm text-ink-soft">{t("mutes.renotesHint")}</p>
        <ul className="mt-3 flex flex-col gap-2">
          {(settings.data?.renotesHiddenFrom ?? []).map((user) => (
            <li key={user.id} className="flex items-center gap-3">
              <Avatar user={user} size="sm" />
              <span className="min-w-0 flex-1 truncate font-bold">
                {user.displayName || `@${user.handle}`}
              </span>
              <Button variant="ghost" size="sm" onClick={() => show.mutate(user.id)}>
                {t("mutes.showRenotes")}
              </Button>
            </li>
          ))}
        </ul>
      </section>
    </Page>
  );
}
