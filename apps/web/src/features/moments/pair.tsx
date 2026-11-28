import type { common, moments } from "@tokuly/api";
import { momentPicture } from "@tokuly/core/moments";
import { Lock } from "lucide-react";
import { type ReactNode, useState } from "react";
import { imageUrl } from "#/features/notes";
import { cn } from "#/lib/cn";
import { useTranslation } from "#/lib/use-translation";
import { blurUrl } from "./blur";

/**
 * A moment's two pictures: the back camera big, the front camera as an
 * inset that swaps places when tapped. A locked moment shows only the
 * blurred stand-ins, with `locked` (what to do about it) on top.
 */
export function MomentPair({
  moment,
  width = 480,
  locked: lockedContent,
  className,
}: {
  moment: Pick<moments.Moment, "back" | "front" | "locked">;
  /** About how wide the pair is drawn, in CSS pixels (to pick a rendition). */
  width?: number;
  locked?: ReactNode;
  className?: string;
}) {
  const { t } = useTranslation("moments");
  const [swapped, setSwapped] = useState(false);
  const big = swapped ? moment.front : moment.back;
  const small = swapped ? moment.back : moment.front;
  return (
    <div
      className={cn(
        "relative aspect-[3/4] w-full overflow-hidden rounded-card bg-surface-3",
        className,
      )}
      data-testid="moment-pair"
      data-locked={moment.locked || undefined}
    >
      <Picture
        image={big}
        locked={moment.locked}
        width={width}
        label={swapped ? t("card.front") : t("card.back")}
        className="absolute inset-0 size-full"
      />
      {moment.locked ? (
        <div className="absolute inset-0 grid place-items-center bg-black/25 p-4 text-center text-white">
          <div className="flex flex-col items-center gap-3">
            <span className="grid size-12 place-items-center rounded-full bg-black/40">
              <Lock className="size-6" />
            </span>
            {lockedContent}
          </div>
        </div>
      ) : (
        <button
          type="button"
          onClick={() => setSwapped((value) => !value)}
          aria-label={t("card.swap")}
          className="pressable absolute top-3 left-3 w-[30%] overflow-hidden rounded-2xl border-[3px] border-ink shadow-pop-sm"
          data-testid="moment-inset"
        >
          <Picture
            image={small}
            locked={false}
            width={width * 0.3}
            label={swapped ? t("card.back") : t("card.front")}
            className="aspect-[3/4] w-full"
          />
        </button>
      )}
    </div>
  );
}

function Picture({
  image,
  locked,
  width,
  label,
  className,
}: {
  image: common.Image | undefined;
  locked: boolean;
  width: number;
  label: string;
  className?: string;
}) {
  const shown = momentPicture(image, locked);
  if (!shown) return <div className={cn("bg-surface-3", className)} />;
  if (shown.kind === "blur") {
    const url = blurUrl(shown.thumbhash);
    return (
      <div
        className={cn("overflow-hidden", className)}
        style={{ background: shown.color || undefined }}
      >
        {url && (
          <img
            src={url}
            alt=""
            className="size-full scale-110 object-cover blur-xl"
            data-testid="moment-blur"
          />
        )}
      </div>
    );
  }
  return (
    <img
      src={imageUrl(shown.image, width)}
      alt={label}
      loading="lazy"
      className={cn("object-cover", className)}
      style={{ background: shown.image.dominantColor || undefined }}
    />
  );
}
