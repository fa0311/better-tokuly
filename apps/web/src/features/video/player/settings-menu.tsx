import type { video } from "@tokuly/api";
import { Check, ChevronLeft, ChevronRight } from "lucide-react";
import { type ReactNode, useEffect, useRef, useState } from "react";
import { cn } from "#/lib/cn";
import { useTranslation } from "#/lib/use-translation";
import type { MediaState } from "./engine";
import { RATES } from "./shortcuts";

type Panel = "main" | "speed" | "quality" | "captions";

export interface SettingsMenuProps {
  state: MediaState;
  subtitles: video.SubtitleTrack[];
  trackId: string | null;
  /** Speed is set by someone else (a watch party's host). */
  rateLocked: boolean;
  onRate(rate: number): void;
  onLevel(level: number): void;
  onTrack(trackId: string | null): void;
  onClose(): void;
}

/**
 * The gear menu: speed, quality and subtitles. It is drawn inside the player
 * (not in a portal), so it is still there in fullscreen.
 */
export function SettingsMenu({
  state,
  subtitles,
  trackId,
  rateLocked,
  onRate,
  onLevel,
  onTrack,
  onClose,
}: SettingsMenuProps) {
  const { t } = useTranslation("video");
  const [panel, setPanel] = useState<Panel>("main");
  const root = useRef<HTMLDivElement>(null);

  // Opening the menu (or one of its pages) moves the focus into it.
  // refocus when the page of the menu changes.
  useEffect(() => {
    root.current?.querySelector<HTMLElement>("button:not(:disabled)")?.focus();
  }, [panel]);

  const quality = (height: number) => `${height}p`;
  const playing = state.levels.find((level) => level.index === state.playingLevel);
  const currentQuality =
    state.level === -1
      ? playing
        ? t("player.autoAt", { quality: quality(playing.height) })
        : t("player.auto")
      : quality(state.levels.find((level) => level.index === state.level)?.height ?? 0);
  const currentTrack = subtitles.find((track) => track.id === trackId);
  const rateLabel = (rate: number) => (rate === 1 ? t("player.normalSpeed") : `${rate}×`);

  const choose = (action: () => void) => () => {
    action();
    onClose();
  };

  return (
    <div
      ref={root}
      role="dialog"
      aria-label={t("player.settings")}
      data-testid="player-settings"
      className="absolute right-2 bottom-16 z-10 flex max-h-[calc(100%-5rem)] w-60 max-w-[calc(100%-1rem)] flex-col overflow-y-auto rounded-2xl bg-black/90 p-1.5 text-sm font-bold backdrop-blur"
      onKeyDown={(event) => {
        if (event.key !== "Escape") return;
        event.stopPropagation();
        if (panel === "main") onClose();
        else setPanel("main");
      }}
    >
      {panel === "main" ? (
        <>
          <Row
            label={t("player.speed")}
            value={rateLabel(state.rate)}
            disabled={rateLocked}
            onClick={() => setPanel("speed")}
          />
          {state.levels.length > 0 && (
            <Row
              label={t("player.quality")}
              value={currentQuality}
              onClick={() => setPanel("quality")}
            />
          )}
          {subtitles.length > 0 && (
            <Row
              label={t("player.captions")}
              value={currentTrack ? currentTrack.label || currentTrack.language : t("player.off")}
              onClick={() => setPanel("captions")}
            />
          )}
        </>
      ) : (
        <>
          <button
            type="button"
            className="flex h-10 items-center gap-1 rounded-xl px-2 text-left hover:bg-white/15"
            onClick={() => setPanel("main")}
          >
            <ChevronLeft className="size-4" />
            {t(
              panel === "speed"
                ? "player.speed"
                : panel === "quality"
                  ? "player.quality"
                  : "player.captions",
            )}
          </button>
          <div className="my-1 border-t border-white/20" />
          {panel === "speed" &&
            RATES.map((rate) => (
              <Option
                key={rate}
                selected={state.rate === rate}
                onClick={choose(() => onRate(rate))}
              >
                {rateLabel(rate)}
              </Option>
            ))}
          {panel === "quality" && (
            <>
              <Option selected={state.level === -1} onClick={choose(() => onLevel(-1))}>
                {t("player.auto")}
              </Option>
              {[...state.levels]
                .sort((a, b) => b.height - a.height)
                .map((level) => (
                  <Option
                    key={level.index}
                    selected={state.level === level.index}
                    onClick={choose(() => onLevel(level.index))}
                  >
                    {quality(level.height)}
                  </Option>
                ))}
            </>
          )}
          {panel === "captions" && (
            <>
              <Option selected={trackId === null} onClick={choose(() => onTrack(null))}>
                {t("player.off")}
              </Option>
              {subtitles.map((track) => (
                <Option
                  key={track.id}
                  selected={trackId === track.id}
                  onClick={choose(() => onTrack(track.id))}
                >
                  <span lang={track.language}>{track.label || track.language}</span>
                </Option>
              ))}
            </>
          )}
        </>
      )}
    </div>
  );
}

function Row({
  label,
  value,
  disabled,
  onClick,
}: {
  label: string;
  value: string;
  disabled?: boolean;
  onClick(): void;
}) {
  return (
    <button
      type="button"
      disabled={disabled}
      className="flex h-10 items-center gap-2 rounded-xl px-3 text-left hover:bg-white/15 disabled:opacity-40"
      onClick={onClick}
    >
      <span className="flex-1">{label}</span>
      <span className="max-w-24 truncate text-white/70">{value}</span>
      <ChevronRight className="size-4 text-white/70" />
    </button>
  );
}

function Option({
  selected,
  onClick,
  children,
}: {
  selected: boolean;
  onClick(): void;
  children: ReactNode;
}) {
  return (
    // biome-ignore lint/a11y/useSemanticElements: styled choices of one setting; a native radio cannot carry this layout.
    <button
      type="button"
      role="radio"
      aria-checked={selected}
      className={cn(
        "flex h-10 shrink-0 items-center gap-2 rounded-xl px-3 text-left hover:bg-white/15",
        selected && "text-white",
      )}
      onClick={onClick}
    >
      <Check className={cn("size-4", !selected && "invisible")} />
      <span className="min-w-0 flex-1 truncate">{children}</span>
    </button>
  );
}
