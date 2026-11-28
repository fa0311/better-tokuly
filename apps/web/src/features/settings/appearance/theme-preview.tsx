import type { Colors } from "@tokuly/core/tokens";
import type { CSSProperties } from "react";
import { cn } from "#/lib/cn";
import { useTranslation } from "#/lib/use-translation";

/**
 * A small screen drawn in the theme being edited, whichever theme the app
 * shows: the tokens are set on this element, so everything inside uses them.
 * It holds every kind of thing the tokens colour: the page, cards on their
 * levels, the three inks, the brand's button and its soft fill, the bright
 * fills and the states.
 */
export function ThemePreview({ theme, className }: { theme: Colors; className?: string }) {
  const { t } = useTranslation("themeEditor");
  const variables = Object.fromEntries(
    Object.entries(theme).map(([name, color]) => [`--${name}`, color]),
  ) as CSSProperties;

  return (
    <figure
      aria-label={t("preview.title")}
      style={variables}
      className={cn(
        "sticker overflow-hidden rounded-card border-[2.5px] border-ink bg-bg p-3 text-ink",
        className,
      )}
      data-testid="theme-preview"
    >
      <div className="sticker rounded-2xl bg-surface p-3">
        <div className="flex items-center gap-3">
          <span className="grid size-11 shrink-0 place-items-center rounded-full border-2 border-ink bg-sky text-lg">
            🐣
          </span>
          <div className="min-w-0 flex-1">
            <p className="flex items-center gap-1.5 truncate font-black">
              {t("preview.name")}
              <span className="rounded-full bg-live px-1.5 text-[10px] font-black text-white">
                {t("preview.live")}
              </span>
            </p>
            <p className="truncate text-sm text-ink-soft">{t("preview.body")}</p>
            <p className="truncate text-xs text-ink-faint">{t("preview.hint")}</p>
          </div>
        </div>
        <div className="mt-3 flex gap-2 border-t-2 border-line pt-3">
          <span className="sticker-sm flex h-8 flex-1 items-center justify-center rounded-full bg-brand text-sm font-bold text-on-brand">
            {t("preview.primary")}
          </span>
          <span className="sticker-sm flex h-8 items-center rounded-full bg-surface-2 px-3 text-sm font-bold">
            {t("preview.secondary")}
          </span>
        </div>
      </div>
      <div className="mt-3 flex items-center gap-2 rounded-2xl bg-surface-3 p-2">
        <span className="rounded-full bg-brand-soft px-2.5 py-1 text-xs font-black text-brand">
          {t("preview.chip")}
        </span>
        <span className="text-xs font-bold text-danger">{t("preview.danger")}</span>
        <span className="ml-auto flex gap-1" aria-hidden>
          {(["lime", "pink", "sky", "sun", "coral", "mint", "success"] as const).map((fill) => (
            <span
              key={fill}
              className="size-3.5 rounded-full border-[1.5px] border-ink"
              style={{ background: `var(--${fill})` }}
            />
          ))}
        </span>
      </div>
    </figure>
  );
}
