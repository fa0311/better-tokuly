import { useMutation, useQueryClient } from "@tanstack/react-query";
import { type common, plans } from "@tokuly/api";
import { profileKeys } from "@tokuly/core/profile";
import { SmilePlus } from "lucide-react";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { Button } from "#/components/ui/button";
import { Dialog } from "#/components/ui/dialog";
import { TextField } from "#/components/ui/field";
import { SwitchRow } from "#/components/ui/switch";
import { EmojiPicker } from "#/features/engagement";
import { errorMessage } from "#/lib/errors";
import { anniversaryClient, keys } from "../api";
import { todayYmd } from "../dates";
import { FriendChips } from "../friend-picker";
import { ChoiceChips } from "../parts";
import { COUNTDOWN_PRESETS } from "./model";

/** Creates a countdown, or changes one: what, which day, every year or once, and who sees it. */
export function CountdownDialog({
  open,
  onOpenChange,
  countdown,
  initialDate,
  onSaved,
}: {
  open: boolean;
  onOpenChange(open: boolean): void;
  /** The countdown being changed; unset for a new one. */
  countdown?: plans.Countdown;
  /** The day a new countdown starts with (a day tapped in the calendar). */
  initialDate?: string;
  onSaved?(countdown: plans.Countdown): void;
}) {
  const { t } = useTranslation("plans");
  const queryClient = useQueryClient();
  const [title, setTitle] = useState("");
  const [emoji, setEmoji] = useState("");
  const [date, setDate] = useState("");
  const [yearly, setYearly] = useState(false);
  const [sharing, setSharing] = useState<plans.CountdownSharing>(plans.CountdownSharing.PRIVATE);
  const [sharedWith, setSharedWith] = useState<common.UserSummary[]>([]);
  const [wasOpen, setWasOpen] = useState(open);
  if (open !== wasOpen) {
    setWasOpen(open);
    if (open) {
      setTitle(countdown?.title ?? "");
      setEmoji(countdown?.emoji ?? "");
      setDate(countdown?.date ?? initialDate ?? "");
      setYearly(countdown?.repeatsYearly ?? false);
      setSharing(countdown?.sharing || plans.CountdownSharing.PRIVATE);
      setSharedWith(countdown?.sharedWith ?? []);
    }
  }

  const save = useMutation({
    mutationFn: async () => {
      const fields = {
        title: title.trim(),
        emoji,
        date,
        repeatsYearly: yearly,
        sharing,
        sharedWithIds:
          sharing === plans.CountdownSharing.CHOSEN ? sharedWith.map((user) => user.id) : [],
      };
      const response = countdown
        ? await anniversaryClient().updateCountdown({ countdownId: countdown.id, ...fields })
        : await anniversaryClient().createCountdown(fields);
      return response.countdown;
    },
    onSuccess: (saved) => {
      if (saved) queryClient.setQueryData(keys.countdown(saved.id), saved);
      void queryClient.invalidateQueries({ queryKey: keys.countdowns });
      void queryClient.invalidateQueries({ queryKey: keys.calendars });
      // Pinned countdowns are drawn on the profile.
      void queryClient.invalidateQueries({ queryKey: profileKeys.profiles });
      onOpenChange(false);
      toast.success(countdown ? t("countdown.saved") : t("countdown.created"));
      if (saved) onSaved?.(saved);
    },
    onError: (error) => toast.error(errorMessage(error, { namespace: "plans" })),
  });

  const needsPeople = sharing === plans.CountdownSharing.CHOSEN && sharedWith.length === 0;
  // A one-off day must lie ahead; a yearly one may be any day.
  const past = Boolean(date) && !yearly && date < todayYmd();

  return (
    <Dialog
      open={open}
      onOpenChange={onOpenChange}
      title={countdown ? t("countdown.editTitle") : t("countdown.newTitle")}
      footer={
        <Button
          loading={save.isPending}
          disabled={!title.trim() || !date || needsPeople || past}
          onClick={() => save.mutate()}
        >
          {countdown ? t("plan.save") : t("countdown.create")}
        </Button>
      }
    >
      <div className="flex flex-col gap-4">
        {!countdown && (
          <div className="scrollbar-none -mx-1 flex gap-1.5 overflow-x-auto px-1 py-1">
            {COUNTDOWN_PRESETS.map((preset) => (
              <button
                key={preset.key}
                type="button"
                onClick={() => {
                  setEmoji(preset.emoji);
                  if (
                    !title.trim() ||
                    COUNTDOWN_PRESETS.some((other) => t(`countdown.presets.${other.key}`) === title)
                  )
                    setTitle(t(`countdown.presets.${preset.key}`));
                  if (preset.key === "birthday") setYearly(true);
                }}
                className="inline-flex h-9 shrink-0 items-center gap-1 rounded-full border-2 border-line bg-surface px-3 text-sm font-bold hover:border-ink-faint"
              >
                <span aria-hidden>{preset.emoji}</span>
                {t(`countdown.presets.${preset.key}`)}
              </button>
            ))}
          </div>
        )}
        <div className="flex items-end gap-3">
          <EmojiPicker
            side="bottom"
            onPick={setEmoji}
            trigger={
              <button
                type="button"
                aria-label={t("plan.form.emoji")}
                className="pressable sticker grid size-12 shrink-0 place-items-center rounded-2xl bg-brand-soft text-2xl"
              >
                {emoji || <SmilePlus className="size-5 text-ink-soft" />}
              </button>
            }
          />
          <TextField
            className="min-w-0 flex-1"
            label={t("countdown.form.title")}
            placeholder={t("countdown.form.titlePlaceholder")}
            value={title}
            maxLength={60}
            onChange={(event) => setTitle(event.target.value)}
          />
        </div>
        <TextField
          type="date"
          label={t("countdown.form.date")}
          value={date}
          error={past ? t("countdown.form.past") : undefined}
          onChange={(event) => setDate(event.target.value)}
        />
        <SwitchRow
          label={t("countdown.form.yearly")}
          hint={t("countdown.form.yearlyHint")}
          checked={yearly}
          onChange={setYearly}
        />
        <div className="flex flex-col gap-2">
          <p className="px-1 text-sm font-bold text-ink-soft">{t("countdown.form.sharing")}</p>
          <ChoiceChips<plans.CountdownSharing>
            label={t("countdown.form.sharing")}
            value={sharing}
            onChange={setSharing}
            options={[
              { value: plans.CountdownSharing.PRIVATE, label: t("countdown.sharing.private") },
              { value: plans.CountdownSharing.CHOSEN, label: t("countdown.sharing.chosen") },
              { value: plans.CountdownSharing.PROFILE, label: t("countdown.sharing.profile") },
            ]}
          />
          {sharing === plans.CountdownSharing.CHOSEN && (
            <FriendChips
              users={sharedWith}
              onChange={setSharedWith}
              addLabel={t("countdown.form.choose")}
              pickerTitle={t("countdown.form.chooseTitle")}
            />
          )}
          {sharing === plans.CountdownSharing.PROFILE && (
            <p className="px-1 text-xs text-ink-faint">{t("countdown.form.profileHint")}</p>
          )}
        </div>
      </div>
    </Dialog>
  );
}
