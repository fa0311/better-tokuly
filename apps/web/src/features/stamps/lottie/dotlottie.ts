/**
 * Loads a dotLottie file (`.lottie`: a ZIP with a manifest, one or more
 * Lottie JSON animations and their images) into something `drawFrame` can
 * draw. A plain Lottie JSON file is accepted too.
 */
import { type Animation, isAnimation } from "./animation";
import type { LoadedAnimation } from "./render";
import { listZipEntries, readZipEntry, type ZipEntry } from "./zip";

interface Manifest {
  animations?: { id?: string }[];
}

/** The entry holding the animation to play: the manifest's first, else the first JSON file. */
export function animationEntry(entries: ZipEntry[], manifest: Manifest | undefined): ZipEntry | undefined {
  const id = manifest?.animations?.[0]?.id;
  // dotLottie 1 keeps animations in `animations/`, dotLottie 2 in `a/`.
  const named = id ? entries.find((entry) => entry.name === `animations/${id}.json` || entry.name === `a/${id}.json`) : undefined;
  return named ?? entries.find((entry) => entry.name.endsWith(".json") && entry.name !== "manifest.json");
}

async function decodeImage(source: Blob | string): Promise<CanvasImageSource | undefined> {
  try {
    if (typeof source !== "string") return await createImageBitmap(source);
    const image = new Image();
    image.src = source;
    await image.decode();
    return image;
  } catch {
    // A picture that cannot be decoded is left out; the rest still plays.
    return undefined;
  }
}

const IMAGE_TYPES: Record<string, string> = { png: "image/png", jpg: "image/jpeg", jpeg: "image/jpeg", webp: "image/webp", gif: "image/gif", svg: "image/svg+xml" };

async function loadImages(animation: Animation, bytes: Uint8Array, entries: ZipEntry[]): Promise<Map<string, CanvasImageSource>> {
  const images = new Map<string, CanvasImageSource>();
  await Promise.all(
    (animation.assets ?? []).map(async (asset) => {
      if (asset.layers || !asset.p) return;
      let image: CanvasImageSource | undefined;
      if (asset.p.startsWith("data:")) {
        image = await decodeImage(asset.p);
      } else {
        const entry = entries.find((candidate) => candidate.name === asset.p || candidate.name.endsWith(`/${asset.p}`));
        if (!entry) return;
        const extension = asset.p.slice(asset.p.lastIndexOf(".") + 1).toLowerCase();
        image = await decodeImage(new Blob([new Uint8Array(await readZipEntry(bytes, entry))], { type: IMAGE_TYPES[extension] ?? "" }));
      }
      if (image) images.set(asset.id, image);
    }),
  );
  return images;
}

/** Parses the bytes of a `.lottie` (or plain Lottie JSON) file. */
export async function parseDotLottie(bytes: Uint8Array): Promise<LoadedAnimation> {
  const decoder = new TextDecoder();
  let entries: ZipEntry[] = [];
  let json: string;
  // "PK": a ZIP archive.
  if (bytes[0] === 0x50 && bytes[1] === 0x4b) {
    entries = listZipEntries(bytes);
    const manifestEntry = entries.find((entry) => entry.name === "manifest.json");
    const manifest = manifestEntry ? (JSON.parse(decoder.decode(await readZipEntry(bytes, manifestEntry))) as Manifest) : undefined;
    const entry = animationEntry(entries, manifest);
    if (!entry) throw new Error("the dotLottie file holds no animation");
    json = decoder.decode(await readZipEntry(bytes, entry));
  } else {
    json = decoder.decode(bytes);
  }
  const animation: unknown = JSON.parse(json);
  if (!isAnimation(animation)) throw new Error("not a Lottie animation");
  return { animation, images: await loadImages(animation, bytes, entries) };
}

const cache = new Map<string, Promise<LoadedAnimation>>();

/** Downloads and parses an animation; every URL is fetched once and shared. */
export function loadDotLottie(url: string): Promise<LoadedAnimation> {
  let pending = cache.get(url);
  if (!pending) {
    pending = fetch(url)
      .then((response) => {
        if (!response.ok) throw new Error(`could not load the animation (${response.status})`);
        return response.arrayBuffer();
      })
      .then((buffer) => parseDotLottie(new Uint8Array(buffer)));
    // A failed download may work next time.
    pending.catch(() => cache.delete(url));
    cache.set(url, pending);
  }
  return pending;
}
