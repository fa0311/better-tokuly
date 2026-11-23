import { common } from "@tokuly/api";
import { useTranslation } from "react-i18next";
import { SelectField } from "#/components/ui/select";
import { SwitchRow } from "#/components/ui/switch";

const { Visibility } = common;

/** Who sees a short and what others may do with it. */
export interface ShortSettings {
  visibility: common.Visibility;
  commentsEnabled: boolean;
  allowRemix: boolean;
  allowDuet: boolean;
}

export const DEFAULT_SETTINGS: ShortSettings = {
  // The server's own default: friends first, the world on purpose.
  visibility: Visibility.FRIENDS,
  commentsEnabled: true,
  allowRemix: true,
  allowDuet: true,
};

const VISIBILITIES = [
  [Visibility.PUBLIC, "public"],
  [Visibility.FRIENDS, "friends"],
  [Visibility.NO_FEED, "noFeed"],
  [Visibility.UNLISTED, "unlisted"],
  [Visibility.PRIVATE, "private"],
] as const;

/** The name of a visibility in the `shorts` strings (`visibility.<key>.label`). */
export function visibilityKey(visibility: common.Visibility): string {
  return VISIBILITIES.find(([value]) => value === visibility)?.[1] ?? "friends";
}

/** The audience and the switches of a short, for the post form and the editor. */
export function SettingsFields({
  value,
  onChange,
}: {
  value: ShortSettings;
  onChange(value: ShortSettings): void;
}) {
  const { t } = useTranslation("shorts");
  return (
    <div className="flex flex-col gap-2">
      <SelectField
        label={t("form.visibility")}
        value={visibilityKey(value.visibility)}
        onChange={(key) =>
          onChange({
            ...value,
            visibility: VISIBILITIES.find(([, name]) => name === key)?.[0] ?? Visibility.FRIENDS,
          })
        }
        options={VISIBILITIES.map(([, key]) => ({
          value: key,
          label: t(`visibility.${key}.label`),
          hint: t(`visibility.${key}.hint`),
        }))}
      />
      <SwitchRow
        label={t("form.comments")}
        hint={t("form.commentsHint")}
        checked={value.commentsEnabled}
        onChange={(commentsEnabled) => onChange({ ...value, commentsEnabled })}
      />
      <SwitchRow
        label={t("form.remix")}
        hint={t("form.remixHint")}
        checked={value.allowRemix}
        onChange={(allowRemix) => onChange({ ...value, allowRemix })}
      />
      <SwitchRow
        label={t("form.duet")}
        hint={t("form.duetHint")}
        checked={value.allowDuet}
        onChange={(allowDuet) => onChange({ ...value, allowDuet })}
      />
    </div>
  );
}
