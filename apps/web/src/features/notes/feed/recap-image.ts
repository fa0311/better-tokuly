import type { feed } from "@tokuly/api";

const WIDTH = 1080;
const HEIGHT = 1350;
const FONT = '"M PLUS Rounded 1c", "Hiragino Maru Gothic ProN", system-ui, sans-serif';
/** The picture is its own artefact: ink on white whatever theme the app is in. */
const INK = "#17122b";
const PAPER = "#ffffff";

/** A colour of the design system, read from the page so the picture matches the app (and the person's accent). */
function token(name: string): string {
  return getComputedStyle(document.documentElement).getPropertyValue(`--${name}`).trim() || INK;
}

/** Breaks text into lines no wider than `maxWidth`. Japanese has no spaces to break at, so it goes character by character. */
function wrap(context: CanvasRenderingContext2D, text: string, maxWidth: number, maxLines: number): string[] {
  const lines: string[] = [];
  let line = "";
  for (const char of Array.from(text)) {
    if (context.measureText(line + char).width > maxWidth && line) {
      lines.push(line);
      line = char;
      if (lines.length === maxLines) break;
    } else {
      line += char;
    }
  }
  if (lines.length < maxLines && line) lines.push(line);
  else if (lines.length === maxLines && line) lines[maxLines - 1] = `${lines[maxLines - 1]!.slice(0, -1)}…`;
  return lines;
}

function roundRect(context: CanvasRenderingContext2D, x: number, y: number, width: number, height: number, radius: number, fill: string): void {
  context.beginPath();
  context.roundRect(x, y, width, height, radius);
  context.fillStyle = fill;
  context.fill();
  context.lineWidth = 8;
  context.strokeStyle = INK;
  context.stroke();
}

export interface RecapImageLabels {
  /** Captions of the four number tiles. */
  activities: string;
  activeDays: string;
  bumps: string;
  newFriends: string;
}

/**
 * Draws a recap as one picture to post anywhere: the title, four numbers
 * and the first highlights, in the app's sticker look.
 */
export async function renderRecapImage(recap: feed.Recap, labels: RecapImageLabels): Promise<Blob> {
  // The app's font has to be ready, or the canvas falls back to a system face.
  await Promise.all([document.fonts.load(`900 64px ${FONT}`), document.fonts.load(`700 40px ${FONT}`)]).catch(() => {});
  const canvas = document.createElement("canvas");
  canvas.width = WIDTH;
  canvas.height = HEIGHT;
  const context = canvas.getContext("2d");
  if (!context) throw new Error("canvas unavailable");

  context.fillStyle = token("brand");
  context.fillRect(0, 0, WIDTH, HEIGHT);
  // Confetti dots behind the card.
  const dots: [number, number, number, string][] = [
    [120, 140, 70, token("lime")],
    [960, 210, 46, token("sun")],
    [1000, 1180, 90, token("pink")],
    [90, 1230, 54, token("sky")],
    [540, 70, 28, PAPER],
  ];
  for (const [x, y, radius, color] of dots) {
    context.beginPath();
    context.arc(x, y, radius, 0, Math.PI * 2);
    context.fillStyle = color;
    context.fill();
    context.lineWidth = 8;
    context.strokeStyle = INK;
    context.stroke();
  }

  // The card, with its hard shadow.
  roundRect(context, 90, 196, 900, 990, 72, INK);
  roundRect(context, 90, 180, 900, 990, 72, PAPER);

  context.fillStyle = INK;
  context.textBaseline = "top";
  context.font = `900 68px ${FONT}`;
  let y = 250;
  for (const line of wrap(context, recap.title, 780, 2)) {
    context.fillText(line, 150, y);
    y += 84;
  }
  context.font = `700 34px ${FONT}`;
  context.globalAlpha = 0.65;
  context.fillText(`${recap.startDate.replaceAll("-", "/")} – ${recap.endDate.replaceAll("-", "/")}`, 150, y + 4);
  context.globalAlpha = 1;
  y += 84;

  const tiles: [string, number, string][] = [
    [labels.activities, recap.activityCount, token("lime")],
    [labels.activeDays, recap.activeDayCount, token("sun")],
    [labels.bumps, recap.bumpCount, token("sky")],
    [labels.newFriends, recap.newFriendCount, token("pink")],
  ];
  tiles.forEach(([label, value, color], index) => {
    const x = 150 + (index % 2) * 400;
    const top = y + Math.floor(index / 2) * 200;
    roundRect(context, x, top, 380, 176, 40, color);
    context.fillStyle = INK;
    context.font = `900 84px ${FONT}`;
    context.fillText(value.toLocaleString(), x + 32, top + 22);
    context.font = `700 32px ${FONT}`;
    context.fillText(label, x + 32, top + 116);
  });
  y += 430;

  context.fillStyle = INK;
  context.font = `700 38px ${FONT}`;
  for (const highlight of recap.highlights.slice(0, 3)) {
    for (const line of wrap(context, `✦ ${highlight.text}`, 780, 2)) {
      context.fillText(line, 150, y);
      y += 52;
    }
    y += 10;
  }

  context.fillStyle = PAPER;
  context.font = `900 56px ${FONT}`;
  context.fillText("Tokuly", 150, 1224);

  return new Promise((resolve, reject) => canvas.toBlob((blob) => (blob ? resolve(blob) : reject(new Error("image export failed"))), "image/png"));
}
