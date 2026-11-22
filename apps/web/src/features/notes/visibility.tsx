import { common } from "@tokuly/api";
import { Check, Globe, Lock, UsersRound } from "lucide-react";
import type { ComponentType } from "react";
import { useTranslation } from "react-i18next";
import { cn } from "#/lib/cn";

interface VisibilityOption {
  value: common.Visibility;
  /** Key under `visibility.` in the notes strings. */
  key: "public" | "friends" | "private";
  icon: ComponentType<{ className?: string }>;
}

/** Who a note can be shown to, widest first. */
export const VISIBILITY_OPTIONS: VisibilityOption[] = [
  { value: common.Visibility.PUBLIC, key: "public", icon: Globe },
  { value: common.Visibility.FRIENDS, key: "friends", icon: UsersRound },
  { value: common.Visibility.PRIVATE, key: "private", icon: Lock },
];

export function visibilityOption(visibility: common.Visibility): VisibilityOption {
  return VISIBILITY_OPTIONS.find((option) => option.value === visibility) ?? VISIBILITY_OPTIONS[0]!;
}

/** A small marker on notes that are not public: an icon with the audience as its label. */
export function VisibilityMark({ visibility, className }: { visibility: common.Visibility; className?: string }) {
  const { t } = useTranslation("notes");
  if (visibility === common.Visibility.PUBLIC || visibility === common.Visibility.UNSPECIFIED) return null;
  const option = visibilityOption(visibility);
  const Icon = option.icon;
  return (
    <span role="img" aria-label={t(`visibility.${option.key}.label`)} title={t(`visibility.${option.key}.label`)} className={cn("inline-flex shrink-0 text-ink-faint", className)}>
      <Icon className="size-3.5" />
    </span>
  );
}

/** The three audiences as a list of choices, each with a line saying who that is. */
export function VisibilityChoices({ value, onChange, disabled }: { value: common.Visibility; onChange(value: common.Visibility): void; disabled?: boolean }) {
  const { t } = useTranslation("notes");
  return (
    <div role="radiogroup" aria-label={t("visibility.title")} className="flex flex-col gap-1.5">
      {VISIBILITY_OPTIONS.map((option) => {
        const Icon = option.icon;
        const selected = option.value === value;
        return (
          // biome-ignore lint/a11y/useSemanticElements: a radio that carries an icon and a description, not a bare radio input.
          <button
            key={option.key}
            type="button"
            role="radio"
            aria-checked={selected}
            disabled={disabled}
            onClick={() => onChange(option.value)}
            className={cn("flex items-center gap-3 rounded-2xl border-2 px-3 py-2.5 text-left transition-colors disabled:opacity-60", selected ? "border-ink bg-brand-soft" : "border-line hover:border-ink-faint")}
          >
            <span className={cn("grid size-9 shrink-0 place-items-center rounded-xl border-2 border-ink", selected ? "bg-lime text-[#17122b]" : "bg-surface")}>
              <Icon className="size-[18px]" />
            </span>
            <span className="min-w-0 flex-1">
              <span className="block font-black">{t(`visibility.${option.key}.label`)}</span>
              <span className="block text-sm text-ink-soft">{t(`visibility.${option.key}.hint`)}</span>
            </span>
            {selected && <Check className="size-5 shrink-0 text-brand" strokeWidth={3} />}
          </button>
        );
      })}
    </div>
  );
}
