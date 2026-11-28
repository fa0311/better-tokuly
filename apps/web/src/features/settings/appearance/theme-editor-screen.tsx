import { useQueryClient } from "@tanstack/react-query";
import type { identity } from "@tokuly/api";
import {
  type ContrastProblem,
  contrastProblems,
  hasCustomColors,
  NO_CUSTOM_COLORS,
  type ThemeColors,
  type ThemeName,
  TOKEN_GROUPS,
  themeFile,
} from "@tokuly/core/theme";
import type { ColorName } from "@tokuly/core/tokens";
import { CircleCheck, Download, RotateCcw, TriangleAlert, Upload } from "lucide-react";
import { useEffect, useId, useRef, useState } from "react";
import { toast } from "sonner";
import { Button } from "#/components/ui/button";
import { ConfirmDialog } from "#/components/ui/confirm";
import { Skeleton } from "#/components/ui/page";
import { Tabs } from "#/components/ui/tabs";
import { cn } from "#/lib/cn";
import { useTranslation } from "#/lib/use-translation";
import { changeSettings, useSettings } from "../data";
import { Group, SettingsPage } from "../shell/settings-page";
import {
  type AccountColors,
  accountColors,
  applyColors,
  baseColors,
  customColorsMessage,
  themeColors,
} from "./colors";
import { ImportThemeDialog } from "./import-theme-dialog";
import { ThemePreview } from "./theme-preview";

/** How long the editor waits after the last change before saving it to the account. */
const SAVE_DELAY = 400;

function resolvedTheme(): ThemeName {
  return document.documentElement.dataset.theme === "dark" ? "dark" : "light";
}

/**
 * Recolours the design tokens one by one (`@tokuly/core/theme`), per theme,
 * like Misskey's theme editor: each change shows in the whole app and in a
 * preview of the theme being edited, and is saved to the account shortly
 * after. A readability check warns about text below WCAG AA; a theme
 * travels as a JSON file.
 */
export function ThemeEditorScreen() {
  const { t } = useTranslation("themeEditor");
  const appearance = useSettings().data?.appearance;
  return (
    <SettingsPage title={t("title")} back="/settings/appearance" wide>
      <p className="px-1 text-sm leading-relaxed text-ink-soft">{t("lead")}</p>
      {appearance ? <Editor appearance={appearance} /> : <Skeleton className="mt-5 h-96" />}
    </SettingsPage>
  );
}

function Editor({ appearance }: { appearance: identity.AppearanceSettings }) {
  const { t } = useTranslation("themeEditor");
  const queryClient = useQueryClient();
  // The theme the app shows now is the one edited first.
  const [editing, setEditing] = useState<ThemeName>(resolvedTheme);
  const { accent } = accountColors(appearance);
  const [custom, setCustom] = useState<ThemeColors>(() => accountColors(appearance).custom);
  const [importing, setImporting] = useState(false);
  const [resetting, setResetting] = useState(false);

  // Saved a moment after the last change (a colour picker reports every step of a drag),
  // and on leaving the screen.
  const unsaved = useRef<{ timer: number; custom: ThemeColors } | null>(null);
  const save = (next: ThemeColors) =>
    changeSettings(queryClient, "appearance", { customColors: customColorsMessage(next) });
  useEffect(
    () => () => {
      const left = unsaved.current;
      if (!left) return;
      window.clearTimeout(left.timer);
      save(left.custom);
    },
    [],
  );

  const change = (next: ThemeColors) => {
    setCustom(next);
    const look: AccountColors = { accent, custom: next };
    applyColors(look);
    if (unsaved.current) window.clearTimeout(unsaved.current.timer);
    unsaved.current = {
      custom: next,
      timer: window.setTimeout(() => {
        unsaved.current = null;
        save(next);
      }, SAVE_DELAY),
    };
  };

  const base = baseColors(accent, editing);
  const theme = themeColors({ accent, custom }, editing);
  const problems = contrastProblems(theme);

  const setToken = (name: ColorName, color: string | undefined) => {
    const edited = { ...custom[editing] };
    if (color === undefined || color === base[name]) delete edited[name];
    else edited[name] = color;
    change({ ...custom, [editing]: edited });
  };

  const exportFile = () => {
    const url = URL.createObjectURL(new Blob([themeFile(custom)], { type: "application/json" }));
    const link = document.createElement("a");
    link.href = url;
    link.download = "tokuly-theme.json";
    link.click();
    URL.revokeObjectURL(url);
    toast.success(t("actions.exported"));
  };

  return (
    <div className="mt-5 xl:grid xl:grid-cols-[minmax(0,1fr)_20rem] xl:items-start xl:gap-6">
      <div className="xl:order-2 xl:sticky xl:top-20">
        <Tabs<ThemeName>
          className="w-full [&>button]:flex-1 [&>button]:justify-center"
          items={(["light", "dark"] as const).map((value) => ({
            value,
            label: t(`themes.${value}`),
          }))}
          value={editing}
          onChange={setEditing}
        />
        <ThemePreview theme={theme} className="mt-3" />
        <ContrastReport problems={problems} />
        <div className="mt-4 flex flex-wrap gap-2">
          <Button
            size="sm"
            variant="secondary"
            icon={<Download className="size-4" />}
            onClick={exportFile}
          >
            {t("actions.export")}
          </Button>
          <Button
            size="sm"
            variant="secondary"
            icon={<Upload className="size-4" />}
            onClick={() => setImporting(true)}
          >
            {t("actions.import")}
          </Button>
          <Button
            size="sm"
            variant="ghost"
            icon={<RotateCcw className="size-4" />}
            disabled={!hasCustomColors(custom)}
            onClick={() => setResetting(true)}
          >
            {t("actions.reset")}
          </Button>
        </div>
      </div>

      <div className="mt-2 xl:order-1 xl:mt-0">
        {TOKEN_GROUPS.map(({ group, tokens }) => (
          <Group
            key={group}
            title={t(`groups.${group}`)}
            lead={group === "brand" ? t("brandHint") : undefined}
            flush
          >
            <ul>
              {tokens.map((name) => (
                <TokenRow
                  key={`${editing}:${name}`}
                  name={name}
                  color={theme[name]}
                  changed={custom[editing][name] !== undefined}
                  lowContrast={problems.some(
                    (problem) => problem.text === name || problem.on === name,
                  )}
                  onChange={(color) => setToken(name, color)}
                  onReset={() => setToken(name, undefined)}
                />
              ))}
            </ul>
          </Group>
        ))}
      </div>

      <ImportThemeDialog
        open={importing}
        onOpenChange={setImporting}
        onImport={(colors) => change(colors)}
      />
      <ConfirmDialog
        open={resetting}
        onOpenChange={setResetting}
        title={t("reset.title")}
        description={t("reset.description")}
        confirmLabel={t("reset.confirm")}
        danger
        onConfirm={() => {
          change(NO_CUSTOM_COLORS);
          setResetting(false);
          toast.success(t("reset.done"));
        }}
      />
    </div>
  );
}

/** What the readability check found for the theme being edited; read out as it changes. */
function ContrastReport({ problems }: { problems: ContrastProblem[] }) {
  const { t } = useTranslation("themeEditor");
  const name = (token: ColorName) => t(`tokens.${token}`);
  return (
    <section
      aria-label={t("contrast.title")}
      className={cn(
        "mt-3 rounded-2xl border-2 px-4 py-3 text-sm",
        problems.length ? "border-ink bg-sun text-[#17122b]" : "border-line bg-surface",
      )}
      data-testid="contrast-report"
    >
      <p role="status" className="flex items-center gap-2 font-black">
        {problems.length ? (
          <TriangleAlert className="size-4 shrink-0" />
        ) : (
          <CircleCheck className="size-4 shrink-0 text-success" />
        )}
        {problems.length ? t("contrast.problems", { count: problems.length }) : t("contrast.ok")}
      </p>
      {problems.length > 0 && (
        <ul className="mt-1.5 flex flex-col gap-1 pl-6 text-xs font-bold leading-relaxed">
          {problems.map((problem) => (
            <li key={`${problem.text}/${problem.on}`}>
              {t("contrast.problem", {
                text: name(problem.text),
                on: name(problem.on),
                ratio: problem.ratio.toFixed(2),
                min: problem.min,
              })}
            </li>
          ))}
        </ul>
      )}
    </section>
  );
}

function normalizeHex(value: string): string | undefined {
  const hex = value.trim().replace(/^#?/, "#").toLowerCase();
  return /^#[0-9a-f]{6}$/.test(hex) ? hex : undefined;
}

/** One token: a colour picker, its code to type, and a way back to the theme's own colour. */
function TokenRow({
  name,
  color,
  changed,
  lowContrast,
  onChange,
  onReset,
}: {
  name: ColorName;
  color: string;
  changed: boolean;
  lowContrast: boolean;
  onChange(color: string): void;
  onReset(): void;
}) {
  const { t } = useTranslation("themeEditor");
  const label = t(`tokens.${name}`);
  const hexId = useId();
  // What is being typed, until it leaves the field; a complete code applies at once.
  const [typed, setTyped] = useState<string | null>(null);
  const invalid = typed !== null && normalizeHex(typed) === undefined;

  return (
    <li className="flex items-center gap-3 rounded-2xl px-2 py-2" data-token={name}>
      <input
        type="color"
        value={color}
        aria-label={t("pick", { name: label })}
        onChange={(event) => onChange(event.target.value)}
        className="size-10 shrink-0 cursor-pointer appearance-none rounded-full border-[2.5px] border-ink bg-transparent p-0 [&::-moz-color-swatch]:rounded-full [&::-moz-color-swatch]:border-0 [&::-webkit-color-swatch-wrapper]:p-0 [&::-webkit-color-swatch]:rounded-full [&::-webkit-color-swatch]:border-0"
      />
      <div className="min-w-0 flex-1">
        <label htmlFor={hexId} className="block truncate text-[15px] font-bold">
          {label}
        </label>
        <p className="flex flex-wrap items-center gap-x-2 text-xs text-ink-soft">
          <code className="font-mono">--{name}</code>
          {changed && <span className="font-bold text-brand">{t("changed")}</span>}
          {lowContrast && (
            <span className="inline-flex items-center gap-0.5 font-bold text-danger">
              <TriangleAlert className="size-3" />
              {t("contrast.low")}
            </span>
          )}
        </p>
      </div>
      <input
        id={hexId}
        aria-label={t("hex", { name: label })}
        aria-invalid={invalid || undefined}
        spellCheck={false}
        autoComplete="off"
        maxLength={7}
        value={typed ?? color}
        onChange={(event) => {
          setTyped(event.target.value);
          const hex = normalizeHex(event.target.value);
          if (hex) onChange(hex);
        }}
        onBlur={() => setTyped(null)}
        className="h-10 w-[6.5rem] shrink-0 rounded-control border-2 border-line bg-surface px-3 font-mono text-sm text-ink transition-colors focus:border-brand focus:outline-none aria-invalid:border-danger"
      />
      <Button
        size="icon-sm"
        variant="ghost"
        aria-label={t("resetToken", { name: label })}
        title={t("resetToken", { name: label })}
        disabled={!changed}
        onClick={onReset}
        className={cn(!changed && "invisible")}
      >
        <RotateCcw className="size-4" />
      </Button>
    </li>
  );
}
