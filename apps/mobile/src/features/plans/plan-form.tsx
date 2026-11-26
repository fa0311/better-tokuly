import {
  formatDay,
  nameOf,
  type PlanFormValue,
  REMINDER_MINUTES,
  todayYmd,
  type WhenKind,
  withEndTime,
} from "@tokuly/core/plans";
import { UserPlus, X } from "lucide-react-native";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { Pressable, Text, TextInput, View } from "react-native";
import { Avatar } from "#/components/avatar";
import { Button } from "#/components/button";
import { ChoiceRow, SwitchRow } from "#/components/form-rows";
import { ErrorNote, Section } from "#/components/page";
import { TextField } from "#/components/text-field";
import { sticker, useTheme } from "#/lib/theme";
import { usePlanZone } from "./data";
import { FriendPicker } from "./friend-picker";
import { DaySheet, TimeSheet } from "./when-sheets";

/** A few emoji to stand for a plan in one tap; the field takes any other. */
const EMOJI = ["🎤", "🍜", "🎮", "🏖️", "🎬", "☕", "⚽", "🎉"];
const WHEN: WhenKind[] = ["tbd", "date", "time"];

type Picking = "date" | "time" | "endDate" | "endTime" | "invitees" | null;

/**
 * The fields of a plan, for a new one (`withInvitees`) and for an edit: what,
 * when (to be decided, a day, or a day and a time, each with an optional
 * end), where, who, a note, and the reminders and limits.
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
  problem: string | null;
}) {
  const { t, i18n } = useTranslation(["plans", "mobile"]);
  const { colors } = useTheme();
  const zone = usePlanZone();
  const today = todayYmd(zone);
  const [picking, setPicking] = useState<Picking>(null);
  const set = (patch: Partial<PlanFormValue>) => onChange({ ...value, ...patch });
  const dayText = (day: string) =>
    day ? formatDay(day, i18n.language, today) : t("mobile:plans.choose");

  return (
    <View className="gap-4">
      <View className="gap-1.5">
        <Text className="px-1 text-sm font-bold text-ink-soft">{t("plans:plan.form.emoji")}</Text>
        <View className="flex-row flex-wrap gap-2">
          {EMOJI.map((emoji) => {
            const chosen = value.emoji === emoji;
            return (
              <Pressable
                key={emoji}
                testID={`plan-emoji-${emoji}`}
                accessibilityRole="radio"
                accessibilityLabel={emoji}
                accessibilityState={{ selected: chosen }}
                onPress={() => set({ emoji: chosen ? "" : emoji })}
                className={`size-11 items-center justify-center rounded-2xl ${chosen ? "bg-brand-soft" : "bg-surface"}`}
                style={chosen ? sticker(colors, "sm") : undefined}
              >
                <Text className="text-2xl leading-8">{emoji}</Text>
              </Pressable>
            );
          })}
        </View>
      </View>
      <TextField
        testID="plan-form-title"
        label={t("plans:plan.form.title")}
        placeholder={t("plans:plan.form.titlePlaceholder")}
        value={value.title}
        maxLength={80}
        onChangeText={(title) => set({ title })}
      />

      <View className="gap-2">
        <Text className="px-1 text-sm font-bold text-ink-soft">{t("plans:plan.form.when")}</Text>
        <View className="flex-row gap-2">
          {WHEN.map((when) => {
            const chosen = value.when === when;
            const label = { tbd: "whenTbd", date: "whenDate", time: "whenTime" }[when];
            return (
              <Pressable
                key={when}
                testID={`plan-when-${when}`}
                accessibilityRole="radio"
                accessibilityState={{ selected: chosen }}
                onPress={() => set({ when })}
                className={`flex-1 rounded-2xl px-2 py-3 ${chosen ? "bg-brand-soft" : "bg-surface"}`}
                style={chosen ? sticker(colors, "sm") : undefined}
              >
                <Text className="text-center text-sm font-black text-ink">
                  {t(`plans:plan.form.${label}`)}
                </Text>
              </Pressable>
            );
          })}
        </View>
        {value.when === "tbd" ? (
          <Text className="px-1 text-xs text-ink-soft">{t("plans:plan.form.tbdHint")}</Text>
        ) : (
          <View className="gap-2">
            <View className="flex-row gap-2">
              <PickButton
                testID="plan-form-date"
                label={t("plans:plan.form.date")}
                text={dayText(value.date)}
                onPress={() => setPicking("date")}
              />
              {value.when === "time" && (
                <PickButton
                  testID="plan-form-time"
                  label={t("plans:plan.form.time")}
                  text={value.time || t("mobile:plans.choose")}
                  onPress={() => setPicking("time")}
                />
              )}
            </View>
            <View className="overflow-hidden rounded-card bg-surface">
              <SwitchRow
                testID="plan-form-has-end"
                label={t("plans:plan.form.hasEnd")}
                value={value.hasEnd}
                onChange={(hasEnd) => set({ hasEnd })}
              />
            </View>
            {value.hasEnd && (
              <View className="flex-row gap-2">
                <PickButton
                  testID="plan-form-end-date"
                  label={t("plans:plan.form.endDate")}
                  text={dayText(value.endDate || value.date)}
                  onPress={() => setPicking("endDate")}
                />
                {value.when === "time" && (
                  <PickButton
                    testID="plan-form-end-time"
                    label={t("plans:plan.form.endTime")}
                    text={value.endTime || t("mobile:plans.choose")}
                    onPress={() => setPicking("endTime")}
                  />
                )}
              </View>
            )}
          </View>
        )}
      </View>

      <TextField
        testID="plan-form-place"
        label={t("plans:plan.form.place")}
        placeholder={t("plans:plan.form.placePlaceholder")}
        value={value.placeName}
        maxLength={80}
        onChangeText={(placeName) => set({ placeName })}
      />
      {value.placeName.trim() !== "" && (
        <TextField
          testID="plan-form-address"
          label={t("plans:plan.form.address")}
          placeholder={t("plans:plan.form.addressPlaceholder")}
          value={value.address}
          maxLength={200}
          onChangeText={(address) => set({ address })}
        />
      )}

      {withInvitees && (
        <View className="gap-2">
          <Text className="px-1 text-sm font-bold text-ink-soft">
            {t("plans:plan.form.invitees")}
          </Text>
          {value.invitees.map((user) => (
            <View
              key={user.id}
              className="flex-row items-center gap-3 rounded-2xl bg-surface px-3 py-2"
            >
              <Avatar user={user} size="sm" />
              <Text numberOfLines={1} className="flex-1 font-black text-ink">
                {nameOf(user)}
              </Text>
              <Pressable
                accessibilityRole="button"
                accessibilityLabel={t("plans:picker.remove", { name: nameOf(user) })}
                onPress={() =>
                  set({ invitees: value.invitees.filter((other) => other.id !== user.id) })
                }
                className="size-9 items-center justify-center rounded-full active:bg-surface-2"
              >
                <X size={18} color={colors["ink-soft"]} />
              </Pressable>
            </View>
          ))}
          <Button
            testID="plan-form-invite"
            variant="secondary"
            icon={<UserPlus size={18} color={colors.ink} />}
            onPress={() => setPicking("invitees")}
          >
            {t("plans:plan.form.addInvitees")}
          </Button>
        </View>
      )}

      <View className="gap-1.5">
        <Text className="px-1 text-sm font-bold text-ink-soft">
          {t("plans:plan.form.description")}
        </Text>
        <TextInput
          testID="plan-form-description"
          accessibilityLabel={t("plans:plan.form.description")}
          placeholder={t("plans:plan.form.descriptionPlaceholder")}
          placeholderTextColor={colors["ink-faint"]}
          multiline
          maxLength={2000}
          value={value.description}
          onChangeText={(description) => set({ description })}
          className="min-h-24 rounded-control border-2 border-line bg-surface px-4 py-3 text-base text-ink"
          style={{ textAlignVertical: "top" }}
        />
      </View>

      <Section title={t("plans:plan.form.more")}>
        <SwitchRow
          testID="plan-form-remind-day-before"
          label={t("plans:plan.form.remindDayBefore")}
          value={value.remindDayBefore}
          onChange={(remindDayBefore) => set({ remindDayBefore })}
        />
        <ChoiceRow
          testID="plan-form-remind-before"
          label={t("plans:plan.form.remindBefore")}
          value={String(value.remindMinutesBefore)}
          choices={REMINDER_MINUTES.map((minutes) => ({
            value: String(minutes),
            label:
              minutes === 0
                ? t("plans:plan.form.remindOff")
                : t("plans:plan.form.remindMinutes", { count: minutes }),
          }))}
          onChange={(minutes) => set({ remindMinutesBefore: Number(minutes) })}
        />
        <SwitchRow
          testID="plan-form-guests-invite"
          label={t("plans:plan.form.guestsCanInvite")}
          hint={t("plans:plan.form.guestsCanInviteHint")}
          value={value.guestsCanInvite}
          onChange={(guestsCanInvite) => set({ guestsCanInvite })}
        />
        <View className="px-4 py-3">
          <TextField
            testID="plan-form-capacity"
            label={t("plans:plan.form.capacity")}
            hint={t("plans:plan.form.capacityHint")}
            placeholder={t("plans:plan.form.capacityPlaceholder")}
            keyboardType="number-pad"
            value={value.capacity}
            maxLength={3}
            onChangeText={(capacity) => set({ capacity: capacity.replace(/\D/g, "") })}
          />
        </View>
      </Section>

      {problem && <ErrorNote>{problem}</ErrorNote>}

      <DaySheet
        visible={picking === "date" || picking === "endDate"}
        title={t(picking === "endDate" ? "plans:plan.form.endDate" : "plans:plan.form.date")}
        value={picking === "endDate" ? value.endDate || value.date : value.date}
        today={today}
        onChange={(day) => set(picking === "endDate" ? { endDate: day } : { date: day })}
        onClose={() => setPicking(null)}
      />
      <TimeSheet
        visible={picking === "time" || picking === "endTime"}
        title={t(picking === "endTime" ? "plans:plan.form.endTime" : "plans:plan.form.time")}
        value={picking === "endTime" ? value.endTime : value.time}
        onChange={(time) =>
          picking === "endTime" ? onChange(withEndTime(value, time)) : set({ time })
        }
        onClose={() => setPicking(null)}
      />
      {withInvitees && (
        <FriendPicker
          visible={picking === "invitees"}
          title={t("plans:plan.form.inviteesTitle")}
          initial={value.invitees}
          confirmLabel={t("plans:picker.confirm")}
          onConfirm={(invitees) => {
            set({ invitees });
            setPicking(null);
          }}
          onClose={() => setPicking(null)}
        />
      )}
    </View>
  );
}

/** A labelled button showing the day or time chosen so far. */
function PickButton({
  label,
  text,
  onPress,
  testID,
}: {
  label: string;
  text: string;
  onPress(): void;
  testID: string;
}) {
  return (
    <Pressable
      testID={testID}
      accessibilityRole="button"
      accessibilityLabel={label}
      accessibilityHint={text}
      onPress={onPress}
      className="flex-1 gap-0.5 rounded-control border-2 border-line bg-surface px-4 py-2 active:bg-surface-2"
    >
      <Text className="text-xs font-bold text-ink-soft">{label}</Text>
      <Text className="text-base font-black text-ink">{text}</Text>
    </Pressable>
  );
}
