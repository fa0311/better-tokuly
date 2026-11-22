import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { type geo, toDate } from "@tokuly/api";
import type { TFunction } from "i18next";
import { Share2, Sparkles } from "lucide-react";
import { useEffect } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { Button } from "#/components/ui/button";
import { Dialog } from "#/components/ui/dialog";
import { EmptyState } from "#/components/ui/page";
import { errorMessage } from "#/lib/errors";
import { formatDistance } from "#/lib/format";
import { useMe } from "#/stores/session";
import { footprintClient, footprintKeys, regionName } from "./api";
import { celebrate } from "./celebrate";
import { cellArt } from "./hexes";
import { formatPercent } from "./levels";
import { flagEmoji } from "./meter";

/** What was uncovered since the person last looked. Shared by the teaser and the dialog. */
export function useRecap() {
  return useQuery({ queryKey: footprintKeys.recap, queryFn: () => footprintClient().getRecap({ markSeen: false }) });
}

/** The nudge on the footprint page when there is something new to look at. */
export function RecapTeaser({ recap, onOpen }: { recap: geo.GetRecapResponse; onOpen(): void }) {
  const { t } = useTranslation("footprints");
  const count = Number(recap.newCellCount);
  if (count === 0) return null;
  return (
    <button type="button" onClick={onOpen} className="pressable sticker flex w-full animate-pop-in items-center gap-3 rounded-card bg-sun p-4 text-left text-[#17122b]">
      <span className="animate-wiggle text-4xl" aria-hidden>
        🎁
      </span>
      <span className="min-w-0 flex-1">
        <span className="block font-black">{t("recap.teaser", { count })}</span>
        <span className="block text-sm font-bold opacity-75">{t("recap.teaserBody")}</span>
      </span>
      <Sparkles className="size-6 shrink-0" />
    </button>
  );
}

/**
 * The recap as a card to show off: new cells drawn as the shape they make,
 * distance, places entered for the first time. Closing it marks the recap
 * as seen, so the next one starts from now.
 */
export function RecapDialog({ open, onOpenChange }: { open: boolean; onOpenChange(open: boolean): void }) {
  const { t, i18n } = useTranslation("footprints");
  const queryClient = useQueryClient();
  const me = useMe();
  const { data: recap } = useRecap();
  const count = Number(recap?.newCellCount ?? 0);

  useEffect(() => {
    if (open && count > 0) celebrate();
  }, [open, count]);

  const markSeen = useMutation({
    mutationFn: () => footprintClient().getRecap({ markSeen: true }),
    onSuccess: () => queryClient.invalidateQueries({ queryKey: footprintKeys.recap }),
  });
  const close = (next: boolean) => {
    onOpenChange(next);
    if (!next && count > 0) markSeen.mutate();
  };

  const share = useMutation({
    mutationFn: async () => {
      const blob = await drawRecapImage(recap!, {
        title: t("recap.cardTitle"),
        headline: t("recap.cardHeadline", { count }),
        since: sinceText(recap!, i18n.language, t),
        lines: recapLines(recap!, t),
        by: me?.user ? `@${me.user.handle}` : "",
      });
      const file = new File([blob], "tokuly-footprints.png", { type: "image/png" });
      if (navigator.canShare?.({ files: [file] })) {
        await navigator.share({ files: [file], text: t("recap.shareText", { count }) });
        return;
      }
      const url = URL.createObjectURL(blob);
      const link = document.createElement("a");
      link.href = url;
      link.download = file.name;
      link.click();
      setTimeout(() => URL.revokeObjectURL(url), 10_000);
      toast.success(t("recap.saved"));
    },
    onError: (error) => {
      // Closing the share sheet is not a failure.
      if (!(error instanceof DOMException && error.name === "AbortError")) toast.error(errorMessage(error, { namespace: "footprints" }));
    },
  });

  return (
    <Dialog
      open={open}
      onOpenChange={close}
      title={t("recap.title")}
      footer={
        count > 0 ? (
          <Button variant="lime" icon={<Share2 className="size-4" />} loading={share.isPending} onClick={() => share.mutate()}>
            {t("recap.share")}
          </Button>
        ) : undefined
      }
    >
      {!recap ? null : count === 0 ? (
        <EmptyState emoji="🌫️" title={t("recap.emptyTitle")} body={t("recap.emptyBody")} />
      ) : (
        <RecapCard recap={recap} />
      )}
    </Dialog>
  );
}

type Translate = TFunction<"footprints">;

function sinceText(recap: geo.GetRecapResponse, language: string, t: Translate): string {
  const since = toDate(recap.since);
  return since ? t("recap.since", { date: since.toLocaleDateString(language, { month: "short", day: "numeric" }) }) : "";
}

/** The facts of a recap as short lines, the same on the card and on the image. */
function recapLines(recap: geo.GetRecapResponse, t: Translate): string[] {
  const lines: string[] = [];
  if (recap.distanceMeters >= 10) lines.push(`👟 ${t("recap.distance", { distance: formatDistance(recap.distanceMeters) })}`);
  for (const region of recap.newRegions.slice(0, 3)) lines.push(`${flagEmoji(region.countryCode)} ${t("recap.newRegion", { region: regionName(region) })}`);
  for (const stat of recap.regions.slice(0, 2)) lines.push(`📈 ${t("recap.regionNow", { region: regionName(stat.region), percent: formatPercent(stat.percent) })}`);
  return lines;
}

function RecapCard({ recap }: { recap: geo.GetRecapResponse }) {
  const { t, i18n } = useTranslation("footprints");
  const count = Number(recap.newCellCount);
  const art = cellArt(recap.newCells, 320, 180, 12);
  return (
    <div className="sticker overflow-hidden rounded-card bg-brand text-on-brand">
      <div className="px-5 pt-5">
        <p className="text-sm font-black opacity-80">{sinceText(recap, i18n.language, t)}</p>
        <p className="mt-1 text-3xl font-black leading-tight">{t("recap.cardHeadline", { count })}</p>
      </div>
      {art.length > 0 && (
        <svg viewBox="0 0 320 180" className="mt-2 w-full" role="img" aria-label={t("recap.artLabel")}>
          {art.map((ring, index) => (
            <polygon
              key={index}
              points={ring.map(([x, y]) => `${x.toFixed(1)},${y.toFixed(1)}`).join(" ")}
              className="fill-lime stroke-ink"
              strokeWidth={1.2}
              strokeLinejoin="round"
            />
          ))}
        </svg>
      )}
      <ul className="flex flex-col gap-1.5 bg-surface px-5 py-4 text-sm font-bold text-ink">
        {recapLines(recap, t).map((line) => (
          <li key={line}>{line}</li>
        ))}
      </ul>
    </div>
  );
}

interface RecapImageText {
  title: string;
  headline: string;
  since: string;
  lines: string[];
  by: string;
}

/**
 * The recap as a picture (1080 × 1350, the portrait size feeds like) for
 * sharing outside the app. Drawn by hand on a canvas: the page's own styles
 * cannot be photographed by a browser.
 */
async function drawRecapImage(recap: geo.GetRecapResponse, text: RecapImageText): Promise<Blob> {
  const [width, height] = [1080, 1350];
  const canvas = document.createElement("canvas");
  canvas.width = width;
  canvas.height = height;
  const context = canvas.getContext("2d");
  if (!context) throw new Error("canvas unavailable");
  // The picture always uses the light palette, whatever theme the app is in.
  const colour = { brand: "#7c5cff", pink: "#ff5c9d", ink: "#17122b", lime: "#c6f24e", paper: "#ffffff", soft: "#5d5778" };
  const font = (size: number, weight = 900) => `${weight} ${size}px "M PLUS Rounded 1c", ui-rounded, system-ui, sans-serif`;
  await document.fonts.load(font(64)).catch(() => {});

  const gradient = context.createLinearGradient(0, 0, width, height);
  gradient.addColorStop(0, colour.brand);
  gradient.addColorStop(1, colour.pink);
  context.fillStyle = gradient;
  context.fillRect(0, 0, width, height);

  context.fillStyle = colour.paper;
  context.textBaseline = "top";
  context.font = font(44);
  context.fillText(text.title, 80, 90);
  context.font = font(34, 700);
  context.globalAlpha = 0.85;
  context.fillText(text.since, 80, 150);
  context.globalAlpha = 1;
  context.font = font(text.headline.length > 14 ? 76 : 96);
  context.fillText(text.headline, 80, 220, width - 160);

  // The shape of what was uncovered.
  const artTop = 380;
  const artHeight = 480;
  context.lineJoin = "round";
  context.lineWidth = 4;
  context.strokeStyle = colour.ink;
  context.fillStyle = colour.lime;
  for (const ring of cellArt(recap.newCells, width - 160, artHeight, 10)) {
    context.beginPath();
    for (const [index, [x, y]] of ring.entries()) {
      if (index === 0) context.moveTo(80 + x, artTop + y);
      else context.lineTo(80 + x, artTop + y);
    }
    context.closePath();
    context.fill();
    context.stroke();
  }

  // The facts on a white sticker.
  const boxTop = artTop + artHeight + 40;
  const boxHeight = height - boxTop - 150;
  context.fillStyle = colour.ink;
  context.beginPath();
  context.roundRect(60, boxTop + 12, width - 120, boxHeight, 48);
  context.fill();
  context.fillStyle = colour.paper;
  context.beginPath();
  context.roundRect(60, boxTop, width - 120, boxHeight, 48);
  context.fill();
  context.lineWidth = 6;
  context.stroke();
  context.fillStyle = colour.ink;
  context.font = font(40, 800);
  for (const [index, line] of text.lines.slice(0, Math.floor((boxHeight - 60) / 64)).entries()) context.fillText(line, 110, boxTop + 44 + index * 64, width - 220);

  context.fillStyle = colour.paper;
  context.font = font(48);
  context.fillText("Tokuly", 80, height - 110);
  context.font = font(34, 700);
  context.textAlign = "right";
  context.fillText(text.by, width - 80, height - 100);

  return new Promise((resolve, reject) => canvas.toBlob((blob) => (blob ? resolve(blob) : reject(new Error("image failed"))), "image/png"));
}
