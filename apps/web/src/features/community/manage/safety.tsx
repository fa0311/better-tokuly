import { useMutation, useQueryClient } from "@tanstack/react-query";
import type { community } from "@tokuly/api";
import { communityKeys } from "@tokuly/core/community";
import { X } from "lucide-react";
import { useState } from "react";
import { toast } from "sonner";
import { Button } from "#/components/ui/button";
import { TextField } from "#/components/ui/field";
import { Skeleton } from "#/components/ui/page";
import { SelectField } from "#/components/ui/select";
import { errorMessage } from "#/lib/errors";
import { useTranslation } from "#/lib/use-translation";
import { moderationClient, NS, useChannels, useSafetySettings } from "../api";
import { Notice, SectionTitle, useDurationLabel } from "../parts";

const SLOW = [0, 10, 30, 60, 300, 900, 3600, 21_600] as const;
const THRESHOLDS = [0, 2, 3, 5, 10, 20] as const;

/** Muted words, the auto-hide threshold and each channel's slow mode. */
export function SafetySection({ community: c }: { community: community.Community }) {
  const { t } = useTranslation(NS);
  const durationLabel = useDurationLabel();
  const queryClient = useQueryClient();
  const safety = useSafetySettings(c.id);
  const channels = useChannels(c.id);
  const [word, setWord] = useState("");
  const onError = (error: unknown) => toast.error(errorMessage(error, { namespace: NS }));

  const update = useMutation({
    mutationFn: (change: { mutedWords?: string[]; autoHideReports?: number }) =>
      moderationClient().updateSafetySettings({
        communityId: c.id,
        mutedWords: change.mutedWords ? { values: change.mutedWords } : undefined,
        autoHideReports: change.autoHideReports,
      }),
    onSuccess: ({ settings }) => {
      queryClient.setQueryData(communityKeys.safety(c.id), settings);
      toast.success(t("saved"));
    },
    onError,
  });
  const slow = useMutation({
    mutationFn: (input: { channelId: string; seconds: number }) =>
      moderationClient().setSlowMode(input),
    onSuccess: () => {
      void queryClient.invalidateQueries({ queryKey: communityKeys.channels(c.id) });
      toast.success(t("saved"));
    },
    onError,
  });

  if (safety.isPending) return <Skeleton className="h-48" />;
  const words = safety.data?.mutedWords ?? [];
  const add = () => {
    const next = word.trim();
    if (!next || words.includes(next)) return;
    update.mutate({ mutedWords: [...words, next] });
    setWord("");
  };
  return (
    <div>
      <SectionTitle>{t("manage.safety.words")}</SectionTitle>
      <p className="px-1 text-sm text-ink-soft">{t("manage.safety.wordsHint")}</p>
      <form
        className="mt-2 flex items-start gap-2"
        onSubmit={(event) => {
          event.preventDefault();
          add();
        }}
      >
        <TextField
          className="min-w-0 flex-1"
          aria-label={t("manage.safety.word")}
          placeholder={t("manage.safety.word")}
          value={word}
          maxLength={40}
          onChange={(event) => setWord(event.target.value)}
        />
        <Button type="submit" className="mt-0.5" disabled={!word.trim() || update.isPending}>
          {t("manage.safety.addWord")}
        </Button>
      </form>
      {words.length === 0 ? (
        <p className="mt-2 px-1 text-sm text-ink-faint">{t("manage.safety.noWords")}</p>
      ) : (
        <ul className="mt-3 flex flex-wrap gap-2">
          {words.map((each) => (
            <li
              key={each}
              className="inline-flex h-8 items-center gap-1 rounded-full bg-surface pr-1 pl-3 text-sm font-bold"
            >
              {each}
              <button
                type="button"
                aria-label={t("manage.safety.removeWord", { word: each })}
                disabled={update.isPending}
                onClick={() =>
                  update.mutate({ mutedWords: words.filter((other) => other !== each) })
                }
                className="grid size-6 place-items-center rounded-full hover:bg-surface-2"
              >
                <X className="size-3.5" />
              </button>
            </li>
          ))}
        </ul>
      )}

      <SectionTitle>{t("manage.safety.autoHide")}</SectionTitle>
      <SelectField
        hint={t("manage.safety.autoHideHint", {
          count: safety.data?.effectiveAutoHideReports ?? 0,
        })}
        value={String(safety.data?.autoHideReports ?? 0)}
        onChange={(next) => update.mutate({ autoHideReports: Number(next) })}
        options={[...new Set([...THRESHOLDS, safety.data?.autoHideReports ?? 0])]
          .sort((a, b) => a - b)
          .map((value) => ({
            value: String(value),
            label:
              value === 0
                ? t("manage.safety.autoHideDefault")
                : t("manage.safety.autoHideCount", { count: value }),
          }))}
      />

      <SectionTitle>{t("manage.safety.slow")}</SectionTitle>
      <p className="px-1 text-sm text-ink-soft">{t("manage.safety.slowHint")}</p>
      <div className="mt-2 flex flex-col gap-2">
        {channels.data?.map((channel) => (
          <div key={channel.id} className="flex items-center gap-3 rounded-card bg-surface p-3">
            <p className="min-w-0 flex-1 truncate font-bold">
              {channel.emoji} {channel.name}
            </p>
            <SelectField
              className="w-36"
              value={String(channel.slowModeSeconds)}
              onChange={(next) => slow.mutate({ channelId: channel.id, seconds: Number(next) })}
              options={[...new Set([...SLOW, channel.slowModeSeconds])]
                .sort((a, b) => a - b)
                .map((value) => ({
                  value: String(value),
                  label: value === 0 ? t("manage.safety.slowOff") : durationLabel(value),
                }))}
            />
          </div>
        ))}
      </div>
      <Notice className="mt-3">{t("manage.safety.slowStaff")}</Notice>
    </div>
  );
}
