import { useMutation, useQueryClient } from "@tanstack/react-query";
import { type identity, toMillis } from "@tokuly/api";
import { type FormEvent, useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { Button } from "#/components/ui/button";
import { Dialog } from "#/components/ui/dialog";
import { TextField } from "#/components/ui/field";
import { accountClient } from "#/lib/api";
import { cn } from "#/lib/cn";
import { errorMessage } from "#/lib/errors";
import { compactDuration } from "#/lib/format";
import { adoptMe } from "../data";
import { Group } from "../shell/settings-page";
import { graphemes } from "./graphemes";

/** One-tap statuses; the text is `profile.status.presets.<key>`. */
const PRESETS = [
  { key: "free", emoji: "🙌" },
  { key: "eating", emoji: "🍚" },
  { key: "studying", emoji: "📚" },
  { key: "working", emoji: "💼" },
  { key: "gaming", emoji: "🎮" },
  { key: "moving", emoji: "🚃" },
  { key: "home", emoji: "🏠" },
  { key: "music", emoji: "🎧" },
  { key: "sleepy", emoji: "😴" },
] as const;

type DurationId = "30m" | "1h" | "4h" | "today" | "week" | "forever";

const DURATIONS: DurationId[] = ["30m", "1h", "4h", "today", "week", "forever"];

/** Seconds a status lasts; 0 keeps it until it is cleared. */
function durationSeconds(id: DurationId, now = new Date()): number {
  switch (id) {
    case "30m":
      return 30 * 60;
    case "1h":
      return 3600;
    case "4h":
      return 4 * 3600;
    case "today": {
      const midnight = new Date(now.getFullYear(), now.getMonth(), now.getDate() + 1);
      return Math.max(60, Math.round((midnight.getTime() - now.getTime()) / 1000));
    }
    case "week":
      return 7 * 24 * 3600;
    case "forever":
      return 0;
  }
}

const MAX_TEXT = 60;

/** The API takes one or two emoji: keep what was typed last. */
const lastTwoEmoji = (value: string) => graphemes(value.trim()).slice(-2).join("");

/** The status that is showing right now (the cached account may hold one that ran out). */
function currentStatus(me: identity.Me): identity.Status | undefined {
  const status = me.status;
  if (!status || (!status.emoji && !status.text)) return undefined;
  if (status.expiresAt && toMillis(status.expiresAt) <= Date.now()) return undefined;
  return status;
}

/** The mood shown on the profile and next to the map pin, with its editor. */
export function StatusCard({ me }: { me: identity.Me }) {
  const { t } = useTranslation("settings");
  const [editing, setEditing] = useState(false);
  const status = currentStatus(me);

  return (
    <Group title={t("profile.status.title")} lead={t("profile.status.lead")}>
      <div className="flex items-center gap-3">
        {status ? (
          <div className="min-w-0 flex-1">
            <p
              data-testid="status-pill"
              className="sticker-sm inline-flex max-w-full items-center gap-2 rounded-full bg-surface px-4 py-1.5 text-sm font-bold"
            >
              {status.emoji && <span>{status.emoji}</span>}
              <span className="truncate">{status.text}</span>
            </p>
            <p className="mt-1.5 px-1 text-xs font-bold text-ink-faint">
              {status.expiresAt
                ? t("profile.status.remaining", {
                    duration: compactDuration(toMillis(status.expiresAt) - Date.now()),
                  })
                : t("profile.status.untilCleared")}
            </p>
          </div>
        ) : (
          <p className="min-w-0 flex-1 text-sm text-ink-soft">{t("profile.status.none")}</p>
        )}
        <Button
          variant={status ? "secondary" : "lime"}
          size="sm"
          className="shrink-0"
          onClick={() => setEditing(true)}
        >
          {status ? t("profile.status.edit") : t("profile.status.set")}
        </Button>
      </div>
      {editing && <StatusDialog me={me} status={status} onClose={() => setEditing(false)} />}
    </Group>
  );
}

function StatusDialog({
  me,
  status,
  onClose,
}: {
  me: identity.Me;
  status: identity.Status | undefined;
  onClose(): void;
}) {
  const { t } = useTranslation(["settings", "common"]);
  const queryClient = useQueryClient();
  const [emoji, setEmoji] = useState(status?.emoji ?? "");
  const [text, setText] = useState(status?.text ?? "");
  const [duration, setDuration] = useState<DurationId>("4h");

  const save = useMutation({
    mutationFn: (input: { emoji: string; text: string; expiresInSeconds: number }) =>
      accountClient().setStatus(input),
    onSuccess: (response, input) => {
      adoptMe({ ...me, status: response.status });
      void queryClient.invalidateQueries({ queryKey: ["identity", "profile"] });
      toast.success(
        input.emoji || input.text
          ? t("settings:profile.status.saved")
          : t("settings:profile.status.cleared"),
      );
      onClose();
    },
    onError: (error) => toast.error(errorMessage(error)),
  });

  const trimmed = text.trim();
  const empty = !emoji && !trimmed;
  const tooLong = graphemes(trimmed).length > MAX_TEXT;
  const submit = (event: FormEvent) => {
    event.preventDefault();
    if (!empty && !tooLong)
      save.mutate({
        emoji: lastTwoEmoji(emoji),
        text: trimmed,
        expiresInSeconds: durationSeconds(duration),
      });
  };

  return (
    <Dialog
      open
      onOpenChange={(open) => !open && onClose()}
      title={t("settings:profile.status.dialogTitle")}
    >
      <form onSubmit={submit} className="flex flex-col gap-5">
        <div className="flex gap-2">
          <TextField
            className="w-16 shrink-0 [&_input]:px-0 [&_input]:text-center [&_input]:text-2xl"
            aria-label={t("settings:profile.status.emoji")}
            placeholder="🙂"
            value={emoji}
            // Mid-conversion text of a Japanese keyboard is left alone until it is committed.
            onChange={(event) =>
              setEmoji(
                (event.nativeEvent as InputEvent).isComposing
                  ? event.target.value
                  : lastTwoEmoji(event.target.value),
              )
            }
            onCompositionEnd={(event) => setEmoji(lastTwoEmoji(event.currentTarget.value))}
            autoComplete="off"
          />
          <TextField
            className="min-w-0 flex-1"
            aria-label={t("settings:profile.status.text")}
            placeholder={t("settings:profile.status.placeholder")}
            value={text}
            onChange={(event) => setText(event.target.value)}
            error={tooLong ? t("settings:profile.status.tooLong", { max: MAX_TEXT }) : undefined}
            autoComplete="off"
            autoFocus
          />
        </div>

        <div>
          <p className="px-1 pb-2 text-sm font-bold text-ink-soft">
            {t("settings:profile.status.quick")}
          </p>
          <div className="flex flex-wrap gap-2">
            {PRESETS.map((preset) => {
              const label = t(`settings:profile.status.presets.${preset.key}`);
              const active = emoji === preset.emoji && text === label;
              return (
                <button
                  key={preset.key}
                  type="button"
                  aria-pressed={active}
                  onClick={() => {
                    setEmoji(preset.emoji);
                    setText(label);
                  }}
                  className={cn(
                    "pressable inline-flex h-9 items-center gap-1.5 rounded-full border-2 px-3 text-sm font-bold",
                    active
                      ? "border-ink bg-lime text-[#17122b]"
                      : "border-line bg-surface hover:border-ink-faint",
                  )}
                >
                  <span>{preset.emoji}</span>
                  {label}
                </button>
              );
            })}
          </div>
        </div>

        <fieldset>
          <legend className="px-1 pb-2 text-sm font-bold text-ink-soft">
            {t("settings:profile.status.duration")}
          </legend>
          <div className="flex flex-wrap gap-2">
            {DURATIONS.map((id) => (
              <label
                key={id}
                className={cn(
                  "inline-flex h-9 cursor-pointer items-center rounded-full border-2 px-3.5 text-sm font-bold has-[:focus-visible]:outline-[3px] has-[:focus-visible]:outline-offset-2 has-[:focus-visible]:outline-brand",
                  duration === id
                    ? "border-ink bg-brand text-on-brand"
                    : "border-line bg-surface hover:border-ink-faint",
                )}
              >
                <input
                  type="radio"
                  name="status-duration"
                  className="sr-only"
                  checked={duration === id}
                  onChange={() => setDuration(id)}
                />
                {t(`settings:profile.status.durations.${id}`)}
              </label>
            ))}
          </div>
        </fieldset>

        <div className="flex justify-end gap-2">
          {status && (
            <Button
              variant="ghost"
              className="mr-auto text-danger"
              disabled={save.isPending}
              onClick={() => save.mutate({ emoji: "", text: "", expiresInSeconds: 0 })}
            >
              {t("settings:profile.status.clear")}
            </Button>
          )}
          <Button variant="ghost" onClick={onClose} disabled={save.isPending}>
            {t("common:actions.cancel")}
          </Button>
          <Button type="submit" loading={save.isPending} disabled={empty || tooLong}>
            {t("settings:profile.status.save")}
          </Button>
        </div>
      </form>
    </Dialog>
  );
}
