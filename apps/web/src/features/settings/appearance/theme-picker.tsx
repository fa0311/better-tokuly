import { Check } from "lucide-react";
import type { CSSProperties } from "react";
import { useTranslation } from "react-i18next";
import type { ThemeSetting } from "#/lib/appearance";
import { cn } from "#/lib/cn";

/**
 * The colours of each theme, for drawing a miniature of it next to the other
 * one. The live tokens only ever hold the active theme, so these few are
 * repeated here (from `styles.css`) as overrides of the same variables.
 */
const MINIATURE: Record<"light" | "dark", CSSProperties> = {
  light: {
    "--bg": "#f4f1ff",
    "--surface": "#ffffff",
    "--surface-3": "#e4def5",
    "--ink": "#17122b",
    "--shadow": "#17122b",
  } as CSSProperties,
  dark: {
    "--bg": "#0d0b14",
    "--surface": "#17132a",
    "--surface-3": "#2d274c",
    "--ink": "#f6f3ff",
    "--shadow": "#000000",
  } as CSSProperties,
};

/** A tiny screen in one theme: a card with two lines and a button. */
function Miniature({ theme, className }: { theme: "light" | "dark"; className?: string }) {
  return (
    <span
      aria-hidden
      style={MINIATURE[theme]}
      className={cn("absolute inset-0 flex flex-col justify-end gap-1.5 bg-bg p-2.5", className)}
    >
      <span className="sticker-sm flex flex-col gap-1 rounded-lg bg-surface p-1.5">
        <span className="h-1.5 w-3/4 rounded-full bg-ink" />
        <span className="h-1.5 w-1/2 rounded-full bg-surface-3" />
      </span>
      <span className="sticker-sm h-3.5 w-10 self-end rounded-full bg-brand" />
    </span>
  );
}

const THEMES: ThemeSetting[] = ["system", "light", "dark"];

/** Chooses between following the device, always light and always dark. */
export function ThemePicker({
  value,
  onChange,
}: {
  value: ThemeSetting;
  onChange(theme: ThemeSetting): void;
}) {
  const { t } = useTranslation("settings");
  return (
    <div
      role="radiogroup"
      aria-label={t("appearance.theme.title")}
      className="grid grid-cols-3 gap-3"
    >
      {THEMES.map((theme) => {
        const selected = theme === value;
        return (
          // biome-ignore lint/a11y/useSemanticElements: a tile with a preview, not a bare radio input.
          <button
            key={theme}
            type="button"
            role="radio"
            aria-checked={selected}
            onClick={() => onChange(theme)}
            className={cn(
              "pressable flex flex-col gap-2 rounded-2xl border-[2.5px] p-2 text-left",
              selected
                ? "border-ink bg-brand-soft shadow-pop-sm"
                : "border-line hover:border-ink-faint",
            )}
          >
            <span className="relative block aspect-[4/3] w-full overflow-hidden rounded-xl border-2 border-ink">
              {theme === "system" ? (
                <>
                  <Miniature theme="light" />
                  {/* The device decides: half of each. */}
                  <Miniature
                    theme="dark"
                    className="[clip-path:polygon(100%_0,100%_100%,0_100%)]"
                  />
                </>
              ) : (
                <Miniature theme={theme} />
              )}
            </span>
            <span className="flex items-center gap-1 px-1 text-sm font-black">
              <span className="min-w-0 flex-1 truncate">
                {t(`appearance.theme.options.${theme}`)}
              </span>
              {selected && <Check className="size-4 shrink-0 text-brand" />}
            </span>
          </button>
        );
      })}
    </div>
  );
}
