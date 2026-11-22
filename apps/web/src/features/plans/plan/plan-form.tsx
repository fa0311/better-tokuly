import type { common } from "@tokuly/api";
import { ChevronDown, SmilePlus } from "lucide-react";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { TextArea, TextField } from "#/components/ui/field";
import { SelectField } from "#/components/ui/select";
import { SwitchRow } from "#/components/ui/switch";
import { EmojiPicker } from "#/features/engagement";
import { cn } from "#/lib/cn";
import { FriendChips } from "../friend-picker";
import { CURRENCIES } from "../money";
import { ChoiceChips } from "../parts";
import { type PlanFormValue, REMINDER_MINUTES, type WhenKind } from "./model";

const QUICK_EMOJI = ["🎤", "🍜", "🎬", "🛍️", "⚽", "📚", "🎡", "☕", "🎮", "🎂"];

/**
 * The fields of a plan, for creating and for editing one. The caller owns
 * the value; `invitees` is shown only when `onInviteesChange` is given (a new
 * plan — people are added to an existing one from its page).
 */
export function PlanForm({
  value,
  onChange,
  withInvitees,
  problem,
}: {
  value: PlanFormValue;
  onChange(value: PlanFormValue): void;
  withInvitees?: boolean;
  problem?: string | null;
}) {
  const { t } = useTranslation("plans");
  const [more, setMore] = useState(false);
  const set = (patch: Partial<PlanFormValue>) => onChange({ ...value, ...patch });
  const reminderOptions = REMINDER_MINUTES.includes(
    value.remindMinutesBefore as (typeof REMINDER_MINUTES)[number],
  )
    ? [...REMINDER_MINUTES]
    : [...REMINDER_MINUTES, value.remindMinutesBefore];

  return (
    <div className="flex flex-col gap-5">
      <div className="flex items-end gap-3">
        <EmojiPicker
          side="bottom"
          onPick={(emoji) => set({ emoji })}
          trigger={
            <button
              type="button"
              aria-label={t("plan.form.emoji")}
              className="pressable sticker grid size-14 shrink-0 place-items-center rounded-2xl bg-brand-soft text-3xl"
            >
              {value.emoji || <SmilePlus className="size-6 text-ink-soft" />}
            </button>
          }
        />
        <TextField
          className="min-w-0 flex-1"
          label={t("plan.form.title")}
          placeholder={t("plan.form.titlePlaceholder")}
          value={value.title}
          maxLength={80}
          required
          onChange={(event) => set({ title: event.target.value })}
        />
      </div>
      <div className="scrollbar-none -mx-1 -mt-2 flex gap-1.5 overflow-x-auto px-1 py-1">
        {QUICK_EMOJI.map((emoji) => (
          <button
            key={emoji}
            type="button"
            aria-label={emoji}
            aria-pressed={value.emoji === emoji}
            onClick={() => set({ emoji: value.emoji === emoji ? "" : emoji })}
            className={cn(
              "grid size-10 shrink-0 place-items-center rounded-xl border-2 text-xl",
              value.emoji === emoji ? "border-ink bg-lime" : "border-transparent bg-surface-2",
            )}
          >
            {emoji}
          </button>
        ))}
      </div>

      <div className="flex flex-col gap-2">
        <p className="px-1 text-sm font-bold text-ink-soft">{t("plan.form.when")}</p>
        <ChoiceChips<WhenKind>
          label={t("plan.form.when")}
          value={value.when}
          onChange={(when) => set({ when })}
          options={[
            { value: "tbd", label: t("plan.form.whenTbd") },
            { value: "date", label: t("plan.form.whenDate") },
            { value: "time", label: t("plan.form.whenTime") },
          ]}
        />
        {value.when === "tbd" ? (
          <p className="px-1 text-sm text-ink-soft">{t("plan.form.tbdHint")}</p>
        ) : (
          <div className="flex flex-col gap-3 rounded-card bg-surface-2 p-3">
            <div className="grid grid-cols-2 gap-3">
              <TextField
                type="date"
                label={t("plan.form.date")}
                value={value.date}
                onChange={(event) => set({ date: event.target.value })}
                className={value.when === "date" ? "col-span-2" : undefined}
              />
              {value.when === "time" && (
                <TextField
                  type="time"
                  label={t("plan.form.time")}
                  value={value.time}
                  onChange={(event) => set({ time: event.target.value })}
                />
              )}
            </div>
            <SwitchRow
              label={t("plan.form.hasEnd")}
              checked={value.hasEnd}
              onChange={(hasEnd) => set({ hasEnd, endDate: value.endDate || value.date })}
            />
            {value.hasEnd && (
              <div className="grid grid-cols-2 gap-3">
                <TextField
                  type="date"
                  label={t("plan.form.endDate")}
                  value={value.endDate}
                  min={value.date}
                  onChange={(event) => set({ endDate: event.target.value })}
                  className={value.when === "date" ? "col-span-2" : undefined}
                />
                {value.when === "time" && (
                  <TextField
                    type="time"
                    label={t("plan.form.endTime")}
                    value={value.endTime}
                    onChange={(event) => set({ endTime: event.target.value })}
                  />
                )}
              </div>
            )}
          </div>
        )}
      </div>

      <div className="flex flex-col gap-3">
        <TextField
          label={t("plan.form.place")}
          placeholder={t("plan.form.placePlaceholder")}
          value={value.placeName}
          maxLength={80}
          onChange={(event) => set({ placeName: event.target.value })}
        />
        {value.placeName.trim() && (
          <TextField
            aria-label={t("plan.form.address")}
            placeholder={t("plan.form.addressPlaceholder")}
            value={value.address}
            maxLength={200}
            onChange={(event) => set({ address: event.target.value })}
          />
        )}
      </div>

      {withInvitees && (
        <FriendChips
          label={t("plan.form.invitees")}
          users={value.invitees}
          onChange={(invitees: common.UserSummary[]) => set({ invitees })}
          addLabel={t("plan.form.addInvitees")}
          pickerTitle={t("plan.form.inviteesTitle")}
        />
      )}

      <TextArea
        label={t("plan.form.description")}
        placeholder={t("plan.form.descriptionPlaceholder")}
        value={value.description}
        maxLength={2000}
        onChange={(event) => set({ description: event.target.value })}
      />

      <div>
        <button
          type="button"
          aria-expanded={more}
          onClick={() => setMore(!more)}
          className="flex items-center gap-1 px-1 text-sm font-bold text-ink-soft hover:text-ink"
        >
          {t("plan.form.more")}
          <ChevronDown className={cn("size-4 transition-transform", more && "rotate-180")} />
        </button>
        {more && (
          <div className="mt-2 flex flex-col gap-2 rounded-card bg-surface-2 p-3">
            <SwitchRow
              label={t("plan.form.remindDayBefore")}
              checked={value.remindDayBefore}
              onChange={(remindDayBefore) => set({ remindDayBefore })}
            />
            <SelectField
              label={t("plan.form.remindBefore")}
              value={String(value.remindMinutesBefore)}
              onChange={(minutes) => set({ remindMinutesBefore: Number(minutes) })}
              options={reminderOptions.map((minutes) => ({
                value: String(minutes),
                label:
                  minutes === 0
                    ? t("plan.form.remindOff")
                    : t("plan.form.remindMinutes", { count: minutes }),
              }))}
            />
            <SwitchRow
              label={t("plan.form.guestsCanInvite")}
              hint={t("plan.form.guestsCanInviteHint")}
              checked={value.guestsCanInvite}
              onChange={(guestsCanInvite) => set({ guestsCanInvite })}
            />
            <TextField
              type="number"
              inputMode="numeric"
              min={1}
              label={t("plan.form.capacity")}
              hint={t("plan.form.capacityHint")}
              placeholder={t("plan.form.capacityPlaceholder")}
              value={value.capacity}
              onChange={(event) => set({ capacity: event.target.value })}
            />
            <SelectField
              label={t("plan.form.currency")}
              value={value.currency}
              onChange={(currency) => set({ currency })}
              options={[...new Set([...CURRENCIES, value.currency])].map((code) => ({
                value: code,
                label: code,
              }))}
            />
          </div>
        )}
      </div>

      {problem && (
        <p role="alert" className="px-1 text-sm font-bold text-danger">
          {problem}
        </p>
      )}
    </div>
  );
}
