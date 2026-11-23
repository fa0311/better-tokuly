import { ImagePlus } from "lucide-react";
import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { Slider } from "#/components/ui/slider";
import { cn } from "#/lib/cn";
import { clock } from "../model";
import { ChoiceChips } from "../sort-chips";
import { captureFrames } from "./video-tools";

/** Which picture stands for a short. */
export type CoverChoice =
  /** The frame the server picks. */
  | { kind: "poster" }
  /** The frame at this moment of the video (seconds from its start). */
  | { kind: "frame"; seconds: number }
  /** A picture of the creator's own. */
  | { kind: "image"; file: File; previewUrl: string };

/** How many frames the filmstrip shows. */
const FRAMES = 8;

/**
 * Chooses the cover: left to the server, a frame picked on a filmstrip of
 * the video (with a slider for the moments in between), or an uploaded
 * picture. `src` is the video to take frames from; `from`/`to` bound the
 * part that will be posted.
 */
export function CoverPicker({
  src,
  from,
  to,
  crossOrigin,
  value,
  onChange,
}: {
  src: string;
  from: number;
  to: number;
  /** The video is served by another origin than a local file's (frames then need its permission). */
  crossOrigin?: boolean;
  value: CoverChoice;
  onChange(value: CoverChoice): void;
}) {
  const { t } = useTranslation("shorts");
  const preview = useRef<HTMLVideoElement>(null);
  const input = useRef<HTMLInputElement>(null);
  const [frames, setFrames] = useState<{ time: number; url: string }[]>([]);
  const length = Math.max(0.1, to - from);
  const wantsFrames = value.kind === "frame";

  // The filmstrip is made when it is first needed, and again when the part to post changes.
  useEffect(() => {
    if (!wantsFrames || !src) return;
    const controller = new AbortController();
    const times = Array.from(
      { length: FRAMES },
      (_, index) => from + (length * (index + 0.5)) / FRAMES,
    );
    const timer = setTimeout(() => {
      void captureFrames(src, times, { crossOrigin, signal: controller.signal }).then((urls) => {
        if (!controller.signal.aborted)
          setFrames(urls.map((url, index) => ({ time: times[index] ?? from, url })));
      });
    }, 250);
    return () => {
      clearTimeout(timer);
      controller.abort();
    };
  }, [wantsFrames, src, from, length, crossOrigin]);

  // The big preview shows the chosen moment itself.
  const seconds = value.kind === "frame" ? Math.min(to, Math.max(from, value.seconds)) : from;
  useEffect(() => {
    if (preview.current && wantsFrames) preview.current.currentTime = seconds;
  }, [wantsFrames, seconds]);

  return (
    <div className="flex flex-col gap-3">
      <ChoiceChips
        label={t("form.cover.label")}
        value={value.kind}
        onChange={(kind) => {
          if (kind === "poster") onChange({ kind });
          else if (kind === "frame") onChange({ kind, seconds: from });
          else input.current?.click();
        }}
        options={[
          { value: "poster", label: t("form.cover.poster") },
          { value: "frame", label: t("form.cover.frame") },
          { value: "image", label: t("form.cover.image") },
        ]}
      />
      <input
        ref={input}
        type="file"
        accept="image/png,image/jpeg,image/webp"
        className="hidden"
        data-testid="cover-file"
        aria-label={t("form.cover.image")}
        onChange={(event) => {
          const file = event.target.files?.[0];
          event.target.value = "";
          if (file) onChange({ kind: "image", file, previewUrl: URL.createObjectURL(file) });
        }}
      />
      {value.kind === "poster" && (
        <p className="text-sm text-ink-soft">{t("form.cover.posterHint")}</p>
      )}
      {value.kind === "image" && (
        <div className="flex items-center gap-3">
          <img
            src={value.previewUrl}
            alt={t("form.cover.preview")}
            className="aspect-[9/16] w-24 rounded-2xl border-2 border-ink object-cover"
          />
          <button
            type="button"
            className="pressable sticker-sm inline-flex h-9 items-center gap-1.5 rounded-full bg-surface px-4 text-sm font-bold"
            onClick={() => input.current?.click()}
          >
            <ImagePlus className="size-4" />
            {t("form.cover.replace")}
          </button>
        </div>
      )}
      {value.kind === "frame" && (
        <div className="flex gap-3">
          <video
            ref={preview}
            src={src}
            muted
            playsInline
            preload="auto"
            aria-label={t("form.cover.preview")}
            className="aspect-[9/16] w-24 shrink-0 rounded-2xl border-2 border-ink bg-black object-cover"
            onLoadedMetadata={(event) => {
              event.currentTarget.currentTime = seconds;
            }}
          />
          <div className="flex min-w-0 flex-1 flex-col justify-center gap-2">
            {frames.length > 0 && (
              <div
                className="flex overflow-hidden rounded-2xl border-2 border-ink"
                data-testid="cover-filmstrip"
              >
                {frames.map((frame, index) => {
                  const next = frames[index + 1]?.time ?? to + 1;
                  const selected =
                    seconds >= frame.time - length / FRAMES / 2 &&
                    seconds < next - length / FRAMES / 2;
                  return (
                    <button
                      key={frame.time}
                      type="button"
                      aria-label={t("form.cover.at", { time: clock(frame.time - from) })}
                      aria-pressed={selected}
                      onClick={() => onChange({ kind: "frame", seconds: frame.time })}
                      className={cn(
                        "relative h-16 min-w-0 flex-1 outline-none",
                        selected && "z-10 ring-4 ring-brand ring-inset",
                      )}
                    >
                      <img src={frame.url} alt="" className="size-full object-cover" />
                    </button>
                  );
                })}
              </div>
            )}
            <Slider
              label={t("form.cover.position")}
              valueText={clock(seconds - from)}
              min={0}
              max={Math.round(length * 10)}
              value={Math.round((seconds - from) * 10)}
              onChange={(tenths) => onChange({ kind: "frame", seconds: from + tenths / 10 })}
            />
            <p className="text-sm text-ink-soft">
              {t("form.cover.frameHint", { time: clock(seconds - from) })}
            </p>
          </div>
        </div>
      )}
    </div>
  );
}
