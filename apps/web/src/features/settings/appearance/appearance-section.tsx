import type { identity } from "@tokuly/api";
import { Check } from "lucide-react";
import { useTranslation } from "react-i18next";
import { Skeleton } from "#/components/ui/page";
import { SelectField } from "#/components/ui/select";
import { Slider } from "#/components/ui/slider";
import { SwitchRow } from "#/components/ui/switch";
import type { ThemeSetting } from "#/lib/appearance";
import { cn } from "#/lib/cn";
import { useChangeSettings, useSettings } from "../data";
import { Group, SettingsPage } from "../shell/settings-page";
import { ACCENTS, accentTokens, isAccentColor } from "./accent";
import { MAX_LEVEL, setAppearance, useAppearance } from "./appearance-store";
import { DopaminePreview } from "./dopamine-preview";
import { ThemePicker } from "./theme-picker";

/** Map styles the map can draw (`MapStyleId` of the map feature). */
const MAP_STYLES = ["tokuly", "light", "dark", "mono"] as const;
type MapStyle = (typeof MAP_STYLES)[number];

type DistanceUnit = "km" | "mi";

/**
 * How Tokuly looks and feels. Theme and ドパガキ度 take effect in this browser
 * the moment they change (and are remembered by it); everything here is also
 * saved to the account, so a new device starts out the same.
 */
export function AppearanceSection() {
  const { t } = useTranslation("settings");
  const { theme, level } = useAppearance();
  const settings = useSettings().data?.appearance;
  const change = useChangeSettings();
  const save = (patch: Partial<identity.AppearanceSettings>) => change("appearance", patch);

  const changeTheme = (next: ThemeSetting) => {
    setAppearance({ theme: next });
    save({ theme: next });
  };

  return (
    <SettingsPage title={t("sections.appearance.title")}>
      <Group title={t("appearance.theme.title")}>
        <ThemePicker value={theme} onChange={changeTheme} />
        <p className="mt-3 px-1 text-sm text-ink-soft">{t(`appearance.theme.hints.${theme}`)}</p>
      </Group>

      <Group title={t("appearance.dopamine.title")} lead={t("appearance.dopamine.lead")}>
        <div className="flex items-baseline justify-between gap-3">
          <p className="text-lg font-black" data-testid="dopamine-level">
            <span className="mr-2 inline-grid size-8 place-items-center rounded-full border-2 border-ink bg-lime text-base text-[#17122b]">{level}</span>
            {t(`appearance.dopamine.levels.${level}.name`)}
          </p>
          <p className="text-xs font-bold text-ink-faint">0 – {MAX_LEVEL}</p>
        </div>
        <p className="mt-1 text-sm leading-relaxed text-ink-soft">{t(`appearance.dopamine.levels.${level}.body`)}</p>
        <Slider
          className="mt-2"
          label={t("appearance.dopamine.title")}
          valueText={`${level}: ${t(`appearance.dopamine.levels.${level}.name`)}`}
          value={level}
          min={0}
          max={MAX_LEVEL}
          step={1}
          // Felt while dragging; saved to the account once let go.
          onChange={(next) => setAppearance({ level: next })}
          onCommit={(next) => save({ dopamineLevel: next })}
        />
        <div aria-hidden className="flex justify-between px-1 text-xs font-bold text-ink-faint">
          <span>{t("appearance.dopamine.calm")}</span>
          <span>{t("appearance.dopamine.wild")}</span>
        </div>
        <DopaminePreview level={level} />
      </Group>

      {settings ? (
        <>
          <Group title={t("appearance.accent.title")} lead={t("appearance.accent.lead")}>
            <AccentPicker value={settings.accentColor} onChange={(accentColor) => save({ accentColor })} />
          </Group>

          <Group title={t("appearance.map.title")} flush>
            <SelectField<MapStyle>
              className="px-3 py-3"
              label={t("appearance.map.style")}
              value={(MAP_STYLES as readonly string[]).includes(settings.mapStyle) ? (settings.mapStyle as MapStyle) : "tokuly"}
              onChange={(mapStyle) => save({ mapStyle })}
              options={MAP_STYLES.map((style) => ({ value: style, label: t(`appearance.map.styles.${style}.label`), hint: t(`appearance.map.styles.${style}.hint`) }))}
            />
            <SelectField<DistanceUnit>
              className="px-3 py-3"
              label={t("appearance.map.unit")}
              value={settings.distanceUnit === "mi" ? "mi" : "km"}
              onChange={(distanceUnit) => save({ distanceUnit })}
              options={[
                { value: "km", label: t("appearance.map.units.km") },
                { value: "mi", label: t("appearance.map.units.mi") },
              ]}
            />
          </Group>

          <Group title={t("appearance.feel.title")} flush>
            <SwitchRow label={t("appearance.feel.sounds.label")} hint={t("appearance.feel.sounds.hint")} checked={settings.sounds} onChange={(sounds) => save({ sounds })} />
            <SwitchRow label={t("appearance.feel.haptics.label")} hint={t("appearance.feel.haptics.hint")} checked={settings.haptics} onChange={(haptics) => save({ haptics })} />
            <SwitchRow label={t("appearance.feel.autoplay.label")} hint={t("appearance.feel.autoplay.hint")} checked={settings.autoplay} onChange={(autoplay) => save({ autoplay })} />
          </Group>
        </>
      ) : (
        <Skeleton className="mt-7 h-64" />
      )}
    </SettingsPage>
  );
}

/** Swatches of the offered accent colours; a colour set elsewhere shows as an extra swatch. */
function AccentPicker({ value, onChange }: { value: string; onChange(color: string): void }) {
  const { t } = useTranslation("settings");
  const current = value.toLowerCase();
  const offered: { id: string; color: string }[] = [...ACCENTS];
  if (isAccentColor(current) && !offered.some((accent) => accent.color === current)) offered.push({ id: "custom", color: current });

  return (
    <div role="radiogroup" aria-label={t("appearance.accent.title")} className="flex flex-wrap gap-3">
      {offered.map((accent) => {
        const selected = accent.color === current;
        const tokens = accentTokens(accent.color).light;
        return (
          // biome-ignore lint/a11y/useSemanticElements: a colour swatch, not a bare radio input.
          <button
            key={accent.id}
            type="button"
            role="radio"
            aria-checked={selected}
            aria-label={t(`appearance.accent.colors.${accent.id}`)}
            title={t(`appearance.accent.colors.${accent.id}`)}
            onClick={() => onChange(accent.color)}
            className={cn("pressable grid size-11 place-items-center rounded-full border-[2.5px] border-ink", selected && "shadow-pop-sm ring-4 ring-brand-soft")}
            style={{ background: tokens.brand, color: tokens.onBrand }}
          >
            {selected && <Check className="size-5" />}
          </button>
        );
      })}
    </div>
  );
}
