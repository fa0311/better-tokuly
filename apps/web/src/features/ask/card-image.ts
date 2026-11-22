import QRCode from "qrcode";
import { wrapText } from "./model";

/**
 * The image cards people post to their stories: a question box ("ask me
 * anything", with the link and optionally its QR code) and one answered
 * question. Drawn on a canvas in the app's sticker look — the palette is the
 * light theme's, whatever theme the app is in, because the picture leaves the
 * app.
 */
const INK = "#17122b";
const PAPER = "#ffffff";
const BRAND = "#7c5cff";
const BRAND_SOFT = "#e9e3ff";
const LIME = "#c6f24e";
const PINK = "#ff5c9d";
const SUN = "#ffd23f";
const SKY = "#4fc3ff";
const FONT = '"M PLUS Rounded 1c", ui-rounded, "Hiragino Maru Gothic ProN", system-ui, sans-serif';

/** Story format: 9:16. */
export const CARD_WIDTH = 1080;
export const CARD_HEIGHT = 1920;

export interface BoxCard {
  kind: "box";
  /** The small label on the card ("質問箱"). */
  label: string;
  /** The owner's prompt line. */
  prompt: string;
  handle: string;
  /** The line above the link ("ここから質問してね!"). */
  callToAction: string;
  url: string;
  withQr: boolean;
}

export interface AnswerCard {
  kind: "answer";
  /** The label on the question ("匿名さんからの質問"). */
  label: string;
  question: string;
  answer: string;
  handle: string;
  callToAction: string;
  url: string;
}

export type Card = BoxCard | AnswerCard;

const font = (weight: number, size: number) => `${weight} ${size}px ${FONT}`;

function roundRect(
  context: CanvasRenderingContext2D,
  x: number,
  y: number,
  width: number,
  height: number,
  radius: number,
): void {
  context.beginPath();
  context.roundRect(x, y, width, height, radius);
}

/** A sticker: filled shape, ink outline, hard shadow underneath. */
function sticker(
  context: CanvasRenderingContext2D,
  x: number,
  y: number,
  width: number,
  height: number,
  radius: number,
  fill: string,
  shadow = 16,
): void {
  context.fillStyle = INK;
  roundRect(context, x, y + shadow, width, height, radius);
  context.fill();
  context.fillStyle = fill;
  roundRect(context, x, y, width, height, radius);
  context.fill();
  context.lineWidth = 8;
  context.strokeStyle = INK;
  context.stroke();
}

function backdrop(context: CanvasRenderingContext2D): void {
  context.fillStyle = BRAND;
  context.fillRect(0, 0, CARD_WIDTH, CARD_HEIGHT);
  // Confetti: the same on every card, so two posts of one person look like a set.
  const dots: [number, number, number, string][] = [
    [120, 210, 70, LIME],
    [940, 330, 46, SUN],
    [1010, 1180, 90, PINK],
    [70, 1420, 54, SKY],
    [860, 1730, 64, LIME],
    [230, 1800, 36, SUN],
    [520, 120, 30, PINK],
  ];
  for (const [x, y, radius, colour] of dots) {
    context.beginPath();
    context.arc(x, y, radius, 0, Math.PI * 2);
    context.fillStyle = colour;
    context.fill();
    context.lineWidth = 7;
    context.strokeStyle = INK;
    context.stroke();
  }
}

/** A pill with centred text; returns its height. */
function pill(
  context: CanvasRenderingContext2D,
  text: string,
  centreX: number,
  y: number,
  fill: string,
  size: number,
): number {
  context.font = font(900, size);
  const width = Math.min(CARD_WIDTH - 160, context.measureText(text).width + size * 1.6);
  const height = size * 2;
  sticker(context, centreX - width / 2, y, width, height, height / 2, fill, 10);
  context.fillStyle = INK;
  context.textAlign = "center";
  context.textBaseline = "middle";
  context.fillText(text, centreX, y + height / 2 + size * 0.04, width - size);
  return height;
}

/** Draws wrapped text centred in a column and returns the height it took. */
function paragraph(
  context: CanvasRenderingContext2D,
  text: string,
  centreX: number,
  top: number,
  width: number,
  size: number,
  weight: number,
  maxLines: number,
  colour = INK,
): number {
  context.font = font(weight, size);
  context.fillStyle = colour;
  context.textAlign = "center";
  context.textBaseline = "top";
  const lines = wrapText(text, width, maxLines, (value) => context.measureText(value).width);
  const lineHeight = size * 1.45;
  for (const [index, line] of lines.entries())
    context.fillText(line, centreX, top + index * lineHeight);
  return lines.length * lineHeight;
}

/** How many lines `text` needs at `size` (to pick a size that fits). */
function lineCount(
  context: CanvasRenderingContext2D,
  text: string,
  width: number,
  size: number,
  weight: number,
): number {
  context.font = font(weight, size);
  return wrapText(text, width, 99, (value) => context.measureText(value).width).length;
}

/** The largest of `sizes` at which `text` fits into `maxLines`; the smallest when none does. */
function fitSize(
  context: CanvasRenderingContext2D,
  text: string,
  width: number,
  weight: number,
  sizes: number[],
  maxLines: (size: number) => number,
): number {
  return (
    sizes.find((size) => lineCount(context, text, width, size, weight) <= maxLines(size)) ??
    sizes[sizes.length - 1]!
  );
}

async function drawFooter(
  context: CanvasRenderingContext2D,
  card: Card,
  top: number,
): Promise<void> {
  const centre = CARD_WIDTH / 2;
  const qr = card.kind === "box" && card.withQr;
  let y = top;
  if (qr) {
    const size = 300;
    const code = document.createElement("canvas");
    await QRCode.toCanvas(code, card.url, {
      margin: 1,
      width: size,
      color: { dark: INK, light: PAPER },
    });
    sticker(context, centre - size / 2 - 24, y, size + 48, size + 48, 40, PAPER, 12);
    context.drawImage(code, centre - size / 2, y + 24, size, size);
    y += size + 48 + 56;
  }
  y += pill(context, card.callToAction, centre, y, LIME, 44) + 40;
  context.font = font(800, 38);
  context.fillStyle = PAPER;
  context.textAlign = "center";
  context.textBaseline = "top";
  context.fillText(card.url.replace(/^https?:\/\//, ""), centre, y, CARD_WIDTH - 120);
}

async function drawBox(context: CanvasRenderingContext2D, card: BoxCard): Promise<void> {
  const centre = CARD_WIDTH / 2;
  const cardX = 90;
  const cardWidth = CARD_WIDTH - cardX * 2;
  const textWidth = cardWidth - 140;
  const size = fitSize(context, card.prompt, textWidth, 900, [92, 76, 62, 52], (value) =>
    Math.floor(520 / (value * 1.45)),
  );
  const textHeight =
    Math.min(
      lineCount(context, card.prompt, textWidth, size, 900),
      Math.floor(520 / (size * 1.45)),
    ) *
    size *
    1.45;
  const cardHeight = 230 + textHeight + 110;
  const footer = card.withQr ? 640 : 240;
  const cardY = Math.max(240, (CARD_HEIGHT - cardHeight - footer - 120) / 2);

  sticker(context, cardX, cardY, cardWidth, cardHeight, 72, PAPER, 20);
  pill(context, card.label, centre, cardY - 48, SUN, 46);
  context.font = font(900, 120);
  context.textAlign = "center";
  context.textBaseline = "top";
  context.fillText("💌", centre, cardY + 70);
  paragraph(
    context,
    card.prompt,
    centre,
    cardY + 230,
    textWidth,
    size,
    900,
    Math.floor(520 / (size * 1.45)),
  );
  pill(context, `@${card.handle}`, centre, cardY + cardHeight - 44, BRAND_SOFT, 40);
  await drawFooter(context, card, cardY + cardHeight + 150);
}

async function drawAnswer(context: CanvasRenderingContext2D, card: AnswerCard): Promise<void> {
  const centre = CARD_WIDTH / 2;
  const cardX = 90;
  const cardWidth = CARD_WIDTH - cardX * 2;
  const textWidth = cardWidth - 120;
  const questionSize = fitSize(context, card.question, textWidth, 900, [64, 54, 46, 40], (value) =>
    Math.floor(420 / (value * 1.45)),
  );
  const questionLines = Math.min(
    lineCount(context, card.question, textWidth, questionSize, 900),
    Math.floor(420 / (questionSize * 1.45)),
  );
  const answerText = card.answer || "🖼️";
  const answerSize = fitSize(context, answerText, textWidth, 700, [56, 48, 42, 36], (value) =>
    Math.floor(520 / (value * 1.45)),
  );
  const answerLines = Math.min(
    lineCount(context, answerText, textWidth, answerSize, 700),
    Math.floor(520 / (answerSize * 1.45)),
  );
  const questionHeight = 110 + questionLines * questionSize * 1.45 + 70;
  const answerHeight = 90 + answerLines * answerSize * 1.45 + 110;
  const total = questionHeight + answerHeight - 8;
  const cardY = Math.max(220, (CARD_HEIGHT - total - 360) / 2);

  // The question on top (tinted), the answer below it, as one sticker.
  sticker(context, cardX, cardY, cardWidth, total, 72, PAPER, 20);
  context.save();
  roundRect(context, cardX, cardY, cardWidth, total, 72);
  context.clip();
  context.fillStyle = BRAND_SOFT;
  context.fillRect(cardX, cardY, cardWidth, questionHeight);
  context.restore();
  context.fillStyle = INK;
  context.fillRect(cardX, cardY + questionHeight - 4, cardWidth, 8);
  roundRect(context, cardX, cardY, cardWidth, total, 72);
  context.lineWidth = 8;
  context.strokeStyle = INK;
  context.stroke();

  pill(context, card.label, centre, cardY - 44, SUN, 40);
  paragraph(
    context,
    card.question,
    centre,
    cardY + 110,
    textWidth,
    questionSize,
    900,
    questionLines,
  );
  paragraph(
    context,
    answerText,
    centre,
    cardY + questionHeight + 90,
    textWidth,
    answerSize,
    700,
    answerLines,
  );
  pill(context, `@${card.handle}`, centre, cardY + total - 44, LIME, 40);
  await drawFooter(context, card, cardY + total + 130);
}

/** Every string of a card, so the web font's subsets for exactly these glyphs are loaded before drawing. */
function textOf(card: Card): string {
  return card.kind === "box"
    ? `${card.label}${card.prompt}${card.handle}${card.callToAction}${card.url}`
    : `${card.label}${card.question}${card.answer}${card.handle}${card.callToAction}${card.url}`;
}

/** Renders a card to a PNG. */
export async function renderCard(card: Card): Promise<Blob> {
  const text = textOf(card);
  // The rounded font arrives in unicode-range pieces; ask for the ones this text needs. Drawing works without them too.
  await Promise.all([
    document.fonts.load(font(900, 48), text),
    document.fonts.load(font(800, 48), text),
    document.fonts.load(font(700, 48), text),
  ]).catch(() => {});
  const canvas = document.createElement("canvas");
  canvas.width = CARD_WIDTH;
  canvas.height = CARD_HEIGHT;
  const context = canvas.getContext("2d");
  if (!context) throw new Error("canvas unavailable");
  backdrop(context);
  if (card.kind === "box") await drawBox(context, card);
  else await drawAnswer(context, card);
  return new Promise((resolve, reject) =>
    canvas.toBlob(
      (blob) => (blob ? resolve(blob) : reject(new Error("canvas export failed"))),
      "image/png",
    ),
  );
}
