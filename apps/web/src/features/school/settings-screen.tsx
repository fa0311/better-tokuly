import { useMutation, useQueryClient } from "@tanstack/react-query";
import { type MessageInitShape, school } from "@tokuly/api";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { Panel } from "#/components/ui/card";
import { TextField } from "#/components/ui/field";
import { SelectField } from "#/components/ui/select";
import { SwitchRow } from "#/components/ui/switch";
import { keys, refresh, settingsClient, toastError, useSchoolSettings } from "./api";
import { Block, LoadFailed, RowsSkeleton, SchoolPage } from "./parts";

type Patch = MessageInitShape<typeof school.UpdateSchoolSettingsRequestSchema>;

/** The school settings: reminders, who sees the studying, the ranking, the daily goal, the pomodoro lengths. */
export function SchoolSettingsScreen() {
  const { t } = useTranslation("school");
  const queryClient = useQueryClient();
  const settings = useSchoolSettings();
  const update = useMutation({
    mutationFn: (patch: Patch) => settingsClient().updateSchoolSettings(patch),
    onMutate: (patch) => {
      // Switches answer at once; the server's version replaces this when it arrives.
      const { pomodoro: _pomodoro, ...simple } = patch as Record<string, unknown>;
      queryClient.setQueryData<school.SchoolSettings>(keys.settings, (current) =>
        current ? ({ ...current, ...simple } as school.SchoolSettings) : current,
      );
    },
    onSuccess: (response) => {
      queryClient.setQueryData(keys.settings, response.settings);
      refresh(queryClient, "study", "studying");
    },
    onError: (error) => {
      toastError(error);
      void queryClient.invalidateQueries({ queryKey: keys.settings });
    },
  });
  const data = settings.data;

  return (
    <SchoolPage back="/school" title={t("settings.title")}>
      {settings.isPending ? (
        <RowsSkeleton rows={5} />
      ) : !data ? (
        <LoadFailed />
      ) : (
        <>
          <Block title={t("settings.study")}>
            <Panel className="flex flex-col gap-3">
              <NumberSetting
                label={t("settings.goal")}
                hint={t("settings.goalHint")}
                value={data.dailyGoalMinutes}
                min={0}
                max={1440}
                onCommit={(dailyGoalMinutes) => update.mutate({ dailyGoalMinutes })}
              />
              <SelectField
                label={t("settings.visibility")}
                hint={t("settings.visibilityHint")}
                value={String(data.studyVisibility || school.StudyVisibility.FRIENDS)}
                onChange={(value) =>
                  update.mutate({ studyVisibility: Number(value) as school.StudyVisibility })
                }
                options={[
                  {
                    value: String(school.StudyVisibility.FRIENDS),
                    label: t("settings.visibilityFriends"),
                  },
                  {
                    value: String(school.StudyVisibility.NOBODY),
                    label: t("settings.visibilityNobody"),
                  },
                ]}
              />
              <div className="-mx-3 flex flex-col">
                <SwitchRow
                  label={t("settings.shareActivity")}
                  hint={t("settings.shareActivityHint")}
                  checked={data.shareStudyActivity}
                  onChange={(shareStudyActivity) => update.mutate({ shareStudyActivity })}
                />
                <SwitchRow
                  label={t("settings.ranking")}
                  hint={t("settings.rankingHint")}
                  checked={data.joinRanking}
                  onChange={(joinRanking) => update.mutate({ joinRanking })}
                />
                <SwitchRow
                  label={t("settings.cheers")}
                  hint={t("settings.cheersHint")}
                  checked={data.cheerNotifications}
                  onChange={(cheerNotifications) => update.mutate({ cheerNotifications })}
                />
              </div>
            </Panel>
          </Block>

          <Block title={t("settings.pomodoro")}>
            <Panel className="grid grid-cols-2 gap-3">
              {(
                [
                  ["workMinutes", 25, 180],
                  ["breakMinutes", 5, 60],
                  ["longBreakMinutes", 15, 120],
                  ["rounds", 4, 12],
                ] as const
              ).map(([field, fallback, max]) => (
                <NumberSetting
                  key={field}
                  label={t(`settings.pomodoroFields.${field}`)}
                  value={data.pomodoro?.[field] || fallback}
                  min={1}
                  max={max}
                  onCommit={(value) =>
                    update.mutate({
                      pomodoro: {
                        workMinutes: data.pomodoro?.workMinutes || 25,
                        breakMinutes: data.pomodoro?.breakMinutes || 5,
                        longBreakMinutes: data.pomodoro?.longBreakMinutes || 15,
                        rounds: data.pomodoro?.rounds || 4,
                        [field]: value,
                      },
                    })
                  }
                />
              ))}
            </Panel>
          </Block>

          <Block title={t("settings.reminders")}>
            <Panel className="flex flex-col gap-3">
              <NumberSetting
                label={t("settings.classSoon")}
                hint={t("settings.classSoonHint")}
                value={data.classSoonMinutes}
                min={0}
                max={120}
                onCommit={(classSoonMinutes) => update.mutate({ classSoonMinutes })}
              />
              <div className="-mx-3">
                <SwitchRow
                  label={t("settings.assignmentReminders")}
                  hint={t("settings.assignmentRemindersHint")}
                  checked={data.assignmentReminders}
                  onChange={(assignmentReminders) => update.mutate({ assignmentReminders })}
                />
              </div>
            </Panel>
          </Block>
        </>
      )}
    </SchoolPage>
  );
}

/** A number that is saved when the field is left (or Enter is pressed), not on every keystroke. */
function NumberSetting({
  label,
  hint,
  value,
  min,
  max,
  onCommit,
}: {
  label: string;
  hint?: string;
  value: number;
  min: number;
  max: number;
  onCommit(value: number): void;
}) {
  const [text, setText] = useState(String(value));
  const [shown, setShown] = useState(value);
  // The saved value changed underneath (the server's answer): show it.
  if (shown !== value) {
    setShown(value);
    setText(String(value));
  }
  const commit = () => {
    const next = Math.min(max, Math.max(min, Math.round(Number(text))));
    if (Number.isNaN(next)) setText(String(value));
    else if (next !== value) onCommit(next);
    else setText(String(value));
  };
  return (
    <TextField
      label={label}
      hint={hint}
      type="number"
      inputMode="numeric"
      min={min}
      max={max}
      value={text}
      onChange={(event) => setText(event.target.value)}
      onBlur={commit}
      onKeyDown={(event) => {
        if (event.key === "Enter") commit();
      }}
    />
  );
}
