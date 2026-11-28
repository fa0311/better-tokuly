import { useQuery, useQueryClient } from "@tanstack/react-query";
import { Link, useBlocker, useNavigate } from "@tanstack/react-router";
import { common, live, media, shorts } from "@tokuly/api";
import { clock, imageUrl, MAX_ASPECT, MAX_SECONDS } from "@tokuly/core/shorts";
import { Plus, Scissors, Shuffle, UploadCloud, X } from "lucide-react";
import { type DragEvent, useEffect, useRef, useState } from "react";
import { toast } from "sonner";
import { Button } from "#/components/ui/button";
import { ConfirmDialog } from "#/components/ui/confirm";
import { Dialog } from "#/components/ui/dialog";
import { EmptyState, ListRow, Page, Skeleton } from "#/components/ui/page";
import { SelectField } from "#/components/ui/select";
import { Slider } from "#/components/ui/slider";
import { SwitchRow } from "#/components/ui/switch";
import { EmojiText } from "#/features/emoji";
import { useMyChannels } from "#/features/video";
import { api } from "#/lib/api";
import { cn } from "#/lib/cn";
import { errorMessage } from "#/lib/errors";
import { uploadAndProcess, uploadFile } from "#/lib/upload";
import { useTranslation } from "#/lib/use-translation";
import { useMe } from "#/stores/session";
import { shortClient, shortKeys, useShort } from "../api";
import { useSound } from "../sound-screen";
import { CaptionField } from "./caption-field";
import { type CoverChoice, CoverPicker } from "./cover-picker";
import type { CreateSearch } from "./create-search";
import { FormSection } from "./form-section";
import { circleOf, DEFAULT_SETTINGS, SettingsFields, type ShortSettings } from "./settings-fields";
import { type SoundChoice, SoundField, soundInput } from "./sound-field";
import { canTrim, probeVideo, trimVideo } from "./video-tools";

const clipClient = () => api().service(live.ClipService);

/** The video a short is made from: a file of the creator's, or a clip they cut. */
type Source =
  | { kind: "file"; file: File; url: string; duration: number }
  | { kind: "clip"; clip: live.Clip; url: string; duration: number };

type Phase =
  | { step: "idle" }
  | { step: "trimming" | "uploading"; progress: number }
  | { step: "cover" | "creating" };

/** What a file or a part of one was uploaded as: kept so that a retry after a failure does not upload it again. */
interface Uploaded {
  key: string;
  assetId: string;
}

const fileKey = (file: File, ...extra: number[]) =>
  [file.name, file.size, file.lastModified, ...extra].join(":");

/**
 * `/shorts/new`: pick a vertical video (or a clip), cut it to at most three
 * minutes, choose the cover, write the caption, and post. The file goes up
 * when "post" is pressed; the short is processed and goes live by itself.
 */
export function CreateScreen({ search }: { search: CreateSearch }) {
  const { t } = useTranslation("shorts");
  const navigate = useNavigate();
  const queryClient = useQueryClient();
  const me = useMe();
  const channelsQuery = useMyChannels();
  const channels = channelsQuery.data?.channels ?? [];
  const [channelId, setChannelId] = useState("");
  const channel = channels.find((entry) => entry.summary?.id === channelId) ?? channels[0];

  const input = useRef<HTMLInputElement>(null);
  const abort = useRef<AbortController | null>(null);
  const uploadedVideo = useRef<Uploaded | null>(null);
  const uploadedCover = useRef<Uploaded | null>(null);
  const [source, setSource] = useState<Source | null>(null);
  const [range, setRange] = useState<[number, number]>([0, 0]);
  const [cover, setCover] = useState<CoverChoice>({ kind: "poster" });
  const [caption, setCaption] = useState(search.tag ? `#${search.tag} ` : "");
  const [settings, setSettings] = useState<ShortSettings>(DEFAULT_SETTINGS);
  const [notify, setNotify] = useState(true);
  const [sound, setSound] = useState<SoundChoice>({ kind: "own", title: "" });
  const [phase, setPhase] = useState<Phase>({ step: "idle" });
  const [dragging, setDragging] = useState(false);
  const [clipsOpen, setClipsOpen] = useState(false);
  const trimmable = canTrim();

  // "Use this sound", "remix this short", "make a short of this clip": what the link asked for.
  const linkedSound = useSound(search.sound);
  const [soundApplied, setSoundApplied] = useState(false);
  if (linkedSound.data && !soundApplied) {
    setSoundApplied(true);
    if (linkedSound.data.reusable) setSound({ kind: "existing", sound: linkedSound.data });
  }
  const remixOf = useShort(search.remix);
  const linkedClip = useQuery({
    queryKey: ["live", "clip", search.clip ?? ""],
    queryFn: async () => (await clipClient().getClip({ clipId: search.clip ?? "" })).clip!,
    enabled: Boolean(search.clip),
    retry: false,
  });

  const release = (current: Source | null) => {
    if (current?.kind === "file") URL.revokeObjectURL(current.url);
  };
  const sourceRef = useRef(source);
  useEffect(() => {
    sourceRef.current = source;
  });
  useEffect(
    () => () => {
      abort.current?.abort();
      release(sourceRef.current);
    },
    [],
  );

  const take = (next: Source) => {
    release(source);
    setSource(next);
    setRange([0, Math.min(next.duration, MAX_SECONDS)]);
    setCover({ kind: "poster" });
  };

  const chooseFile = async (file: File) => {
    if (!file.type.startsWith("video/") && !/\.(mp4|mov|m4v|webm|mkv)$/i.test(file.name)) {
      toast.error(t("create.notVideo"));
      return;
    }
    const url = URL.createObjectURL(file);
    try {
      const facts = await probeVideo(url);
      if (facts.width > 0 && facts.width / facts.height > MAX_ASPECT + 0.01) {
        URL.revokeObjectURL(url);
        toast.error(t("errors:SHORT_NOT_VERTICAL"));
        return;
      }
      if (facts.durationSeconds > MAX_SECONDS + 0.5 && !trimmable) {
        URL.revokeObjectURL(url);
        toast.error(t("errors:SHORT_TOO_LONG"));
        return;
      }
      take({ kind: "file", file, url, duration: facts.durationSeconds });
    } catch {
      // The browser cannot open it, the server may still: post it as it is, without trimming or a filmstrip.
      take({ kind: "file", file, url, duration: 0 });
    }
  };
  const chooseClip = (clip: live.Clip) => {
    setClipsOpen(false);
    take({
      kind: "clip",
      clip,
      url: clip.playback?.mp4Url ?? "",
      duration: clip.durationSeconds,
    });
  };
  const [clipApplied, setClipApplied] = useState(false);
  if (linkedClip.data && !clipApplied) {
    setClipApplied(true);
    if (linkedClip.data.status === live.ClipStatus.READY) chooseClip(linkedClip.data);
  }

  const [from, to] = range;
  const trimmed =
    source?.kind === "file" && source.duration > 0 && (from > 0.05 || to < source.duration - 0.05);
  const tooLong = to - from > MAX_SECONDS + 0.05;
  const busy = phase.step !== "idle";

  // Leaving gives up what was chosen and typed, and an upload under way.
  const handedOver = useRef(false);
  const atRisk = () => source !== null && !handedOver.current;
  const blocker = useBlocker({
    shouldBlockFn: atRisk,
    enableBeforeUnload: atRisk,
    withResolver: true,
  });

  const post = async () => {
    if (!source || !channel?.summary) return;
    const controller = new AbortController();
    abort.current = controller;
    try {
      let assetId: string;
      if (source.kind === "clip") {
        // The clip's own media (set for its creator once it is ready).
        assetId = source.clip.assetId;
      } else if (
        uploadedVideo.current?.key === fileKey(source.file, trimmed ? from : 0, trimmed ? to : 0)
      ) {
        // Up already, from an attempt that failed later on.
        assetId = uploadedVideo.current.assetId;
      } else {
        const key = fileKey(source.file, trimmed ? from : 0, trimmed ? to : 0);
        let file: File = source.file;
        if (trimmed) {
          setPhase({ step: "trimming", progress: 0 });
          file = await trimVideo(source.url, from, to, {
            signal: controller.signal,
            onProgress: (progress) => setPhase({ step: "trimming", progress }),
          });
        }
        setPhase({ step: "uploading", progress: 0 });
        const asset = await uploadFile(file, {
          purpose: media.AssetPurpose.SHORT,
          signal: controller.signal,
          onProgress: (progress) => setPhase({ step: "uploading", progress }),
        });
        assetId = asset.id;
        uploadedVideo.current = { key, assetId };
      }
      let coverAssetId = "";
      if (cover.kind === "image") {
        const key = fileKey(cover.file);
        if (uploadedCover.current?.key === key) {
          coverAssetId = uploadedCover.current.assetId;
        } else {
          setPhase({ step: "cover" });
          coverAssetId = (
            await uploadAndProcess(cover.file, {
              purpose: media.AssetPurpose.THUMBNAIL,
              signal: controller.signal,
            })
          ).id;
          uploadedCover.current = { key, assetId: coverAssetId };
        }
      }
      if (controller.signal.aborted) throw new DOMException("posting cancelled", "AbortError");
      setPhase({ step: "creating" });
      const coverInput =
        cover.kind === "frame"
          ? {
              cover: {
                case: "offsetMs" as const,
                value: Math.round((cover.seconds - from) * 1000),
              },
            }
          : cover.kind === "image"
            ? { cover: { case: "assetId" as const, value: coverAssetId } }
            : undefined;
      const { short } = await shortClient().createShort({
        channelId: channel.summary.id,
        assetId,
        caption: caption.trim(),
        visibility: settings.visibility,
        circleId: circleOf(settings),
        commentsEnabled: settings.commentsEnabled,
        allowRemix: settings.allowRemix,
        allowDuet: settings.allowDuet,
        notifySubscribers: notify,
        cover: coverInput,
        sound: soundInput(sound),
        source:
          source.kind === "clip"
            ? { type: common.SubjectType.CLIP, id: source.clip.id }
            : undefined,
        remix:
          search.remix && remixOf.data
            ? {
                shortId: remixOf.data.id,
                kind: search.kind === "duet" ? shorts.RemixKind.DUET : shorts.RemixKind.REMIX,
              }
            : undefined,
      });
      handedOver.current = true;
      void queryClient.invalidateQueries({ queryKey: shortKeys.lists });
      toast.success(
        short?.status === shorts.ShortStatus.PUBLISHED ? t("create.published") : t("create.done"),
      );
      void navigate({ to: "/shorts/studio" });
    } catch (error) {
      setPhase({ step: "idle" });
      if (controller.signal.aborted) {
        toast.message(t("create.cancelled"));
        return;
      }
      // Pressing "post" again tries once more, without uploading what is up already.
      toast.error(errorMessage(error, { namespace: "shorts" }), {
        description: t("create.retryHint"),
      });
    }
  };
  const cancel = () => abort.current?.abort();

  if (channelsQuery.isPending) {
    return (
      <Page title={t("create.title")} back="/shorts">
        <Skeleton className="h-72 rounded-card" />
      </Page>
    );
  }
  if (!channel) {
    return (
      <Page title={t("create.title")} back="/shorts">
        <EmptyState
          emoji="📡"
          title={t("create.needChannel")}
          body={t("create.needChannelBody")}
          action={
            <Link
              to="/studio/channels/new"
              className="pressable sticker inline-flex h-11 items-center gap-2 rounded-full bg-brand px-5 font-bold text-on-brand"
            >
              <Plus className="size-4" />
              {t("create.makeChannel")}
            </Link>
          }
        />
      </Page>
    );
  }

  const percent = "progress" in phase ? Math.round(phase.progress * 100) : 100;

  return (
    <Page title={t("create.title")} back="/shorts" wide>
      <input
        ref={input}
        type="file"
        accept="video/*,.mkv"
        className="hidden"
        data-testid="short-file"
        aria-label={t("create.choose")}
        onChange={(event) => {
          const file = event.target.files?.[0];
          event.target.value = "";
          if (file) void chooseFile(file);
        }}
      />
      {search.remix && remixOf.data && (
        <p
          className="mb-4 flex items-center gap-2 rounded-2xl border-2 border-ink bg-brand-soft px-4 py-3 text-sm font-bold"
          data-testid="remix-banner"
        >
          <Shuffle className="size-5 shrink-0" aria-hidden />
          <span className="min-w-0 flex-1">
            {t(search.kind === "duet" ? "create.duetOf" : "create.remixOf", {
              name: remixOf.data.channel?.name ?? "",
            })}
          </span>
        </p>
      )}

      {!source ? (
        <div className="mx-auto flex max-w-2xl flex-col gap-3">
          <button
            type="button"
            className={cn(
              "flex flex-col items-center gap-3 rounded-card border-[3px] border-dashed px-6 py-14 text-center transition-colors",
              dragging
                ? "border-brand bg-brand-soft"
                : "border-line bg-surface hover:border-ink-faint",
            )}
            onClick={() => input.current?.click()}
            onDragOver={(event) => {
              event.preventDefault();
              setDragging(true);
            }}
            onDragLeave={() => setDragging(false)}
            onDrop={(event: DragEvent) => {
              event.preventDefault();
              setDragging(false);
              const file = event.dataTransfer.files[0];
              if (file) void chooseFile(file);
            }}
          >
            <span className="sticker grid size-16 place-items-center rounded-3xl bg-pink text-white">
              <UploadCloud className="size-8" />
            </span>
            <span className="text-xl font-black">{t("create.choose")}</span>
            <span className="max-w-xs text-sm leading-relaxed text-ink-soft">
              {t("create.hint")}
            </span>
          </button>
          <Button
            variant="secondary"
            icon={<Scissors className="size-5" />}
            onClick={() => setClipsOpen(true)}
          >
            {t("create.fromClip")}
          </Button>
        </div>
      ) : (
        <div className="grid gap-5 md:grid-cols-[minmax(0,18rem)_minmax(0,1fr)] md:items-start">
          <div className="mx-auto w-full max-w-[16rem] md:sticky md:top-20 md:max-w-none">
            <div className="sticker relative aspect-[9/16] overflow-hidden rounded-[2rem] bg-black">
              {source.url ? (
                <TrimPreview src={source.url} from={from} to={to} />
              ) : (
                <img
                  src={imageUrl(source.kind === "clip" ? source.clip.thumbnail : undefined, 300)}
                  alt=""
                  className="size-full object-cover"
                />
              )}
              <button
                type="button"
                aria-label={t("create.remove")}
                disabled={busy}
                className="absolute top-3 right-3 grid size-10 place-items-center rounded-full bg-black/55 text-white disabled:opacity-40"
                onClick={() => {
                  release(source);
                  setSource(null);
                }}
              >
                <X className="size-5" />
              </button>
            </div>
            <p className="mt-2 truncate text-center text-sm font-bold text-ink-soft">
              {source.kind === "clip" ? (
                <EmojiText text={t("create.clipSource", { title: source.clip.title })} />
              ) : (
                source.file.name
              )}
            </p>
          </div>

          <div className="flex min-w-0 flex-col gap-4">
            {source.kind === "file" && source.duration > 0 && trimmable && (
              <FormSection title={t("create.trim.title")} lead={t("create.trim.lead")}>
                <Slider
                  label={t("create.trim.start")}
                  valueText={clock(from)}
                  min={0}
                  max={Math.floor(source.duration * 10)}
                  value={Math.round(from * 10)}
                  disabled={busy}
                  onChange={(tenths) => {
                    const start = Math.min(tenths / 10, Math.max(0, to - 1));
                    setRange([start, to]);
                    setCover((current) =>
                      current.kind === "frame" ? { kind: "frame", seconds: start } : current,
                    );
                  }}
                />
                <Slider
                  label={t("create.trim.end")}
                  valueText={clock(to)}
                  min={0}
                  max={Math.floor(source.duration * 10)}
                  value={Math.round(to * 10)}
                  disabled={busy}
                  onChange={(tenths) =>
                    setRange([from, Math.max(tenths / 10, Math.min(source.duration, from + 1))])
                  }
                />
                <p
                  className={cn("text-sm font-bold", tooLong ? "text-danger" : "text-ink-soft")}
                  role={tooLong ? "alert" : undefined}
                  data-testid="trim-summary"
                >
                  {tooLong
                    ? t("create.trim.tooLong", { length: clock(to - from) })
                    : trimmed
                      ? t("create.trim.cut", {
                          from: clock(from),
                          to: clock(to),
                          length: clock(to - from),
                        })
                      : t("create.trim.whole", { length: clock(to - from) })}
                </p>
              </FormSection>
            )}

            <FormSection title={t("create.details")}>
              {channels.length > 1 && (
                <SelectField
                  label={t("form.channel")}
                  value={channel.summary?.id ?? ""}
                  onChange={setChannelId}
                  options={channels.map((entry) => ({
                    value: entry.summary?.id ?? "",
                    label: entry.summary?.name ?? "",
                    hint: `@${entry.summary?.handle ?? ""}`,
                  }))}
                />
              )}
              <CaptionField value={caption} onChange={setCaption} />
            </FormSection>

            {source.url && source.duration > 0 && (
              <FormSection title={t("form.cover.title")}>
                <CoverPicker
                  src={source.url}
                  from={from}
                  to={to}
                  crossOrigin={source.kind === "clip"}
                  value={cover}
                  onChange={setCover}
                />
              </FormSection>
            )}

            <FormSection title={t("form.sound.title")} lead={t("form.sound.lead")}>
              <SoundField value={sound} onChange={setSound} />
            </FormSection>

            <FormSection title={t("form.audience")}>
              <SettingsFields value={settings} onChange={setSettings} />
              <SwitchRow
                label={t("form.notify")}
                hint={t("form.notifyHint")}
                checked={notify}
                onChange={setNotify}
              />
            </FormSection>

            {busy && (
              <section
                className="sticker rounded-card bg-surface p-4"
                aria-label={t("create.progress")}
              >
                <p className="font-bold" data-testid="post-status">
                  {phase.step === "trimming"
                    ? t("create.trimming", { percent })
                    : phase.step === "uploading"
                      ? t("create.uploading", { percent })
                      : phase.step === "cover"
                        ? t("create.coverUploading")
                        : t("create.creating")}
                </p>
                <div
                  className="mt-3 h-3 overflow-hidden rounded-full border-2 border-ink bg-surface-3"
                  role="progressbar"
                  aria-label={t("create.progress")}
                  aria-valuemin={0}
                  aria-valuemax={100}
                  aria-valuenow={percent}
                >
                  <div
                    className="h-full bg-lime transition-[width] duration-200"
                    style={{ width: `${percent}%` }}
                  />
                </div>
              </section>
            )}

            <div className="flex justify-end gap-2">
              {busy && phase.step !== "creating" && (
                <Button variant="ghost" onClick={cancel}>
                  {t("create.cancel")}
                </Button>
              )}
              <Button size="lg" loading={busy} disabled={tooLong} onClick={() => void post()}>
                {t("create.post")}
              </Button>
            </div>
          </div>
        </div>
      )}

      {clipsOpen && me?.user && (
        <ClipChooser userId={me.user.id} onPick={chooseClip} onClose={() => setClipsOpen(false)} />
      )}
      <ConfirmDialog
        open={blocker.status === "blocked"}
        onOpenChange={(open) => !open && blocker.reset?.()}
        title={t("create.leaveTitle")}
        description={t("create.leaveBody")}
        confirmLabel={t("create.leave")}
        danger
        onConfirm={() => blocker.proceed?.()}
      />
    </Page>
  );
}

/** The chosen video, playing the part that will be posted over and over. */
function TrimPreview({ src, from, to }: { src: string; from: number; to: number }) {
  const video = useRef<HTMLVideoElement>(null);
  // Moving an end of the range shows that moment.
  useEffect(() => {
    if (video.current && Math.abs(video.current.currentTime - from) > 0.2)
      video.current.currentTime = from;
  }, [from]);
  return (
    <video
      ref={video}
      src={src}
      data-testid="short-preview"
      className="size-full object-cover"
      muted
      playsInline
      autoPlay
      loop
      onTimeUpdate={(event) => {
        const element = event.currentTarget;
        if (to > from && (element.currentTime >= to || element.currentTime < from - 0.3))
          element.currentTime = from;
      }}
    />
  );
}

/** The clips the creator cut (from live streams and videos), to make a short of one. */
function ClipChooser({
  userId,
  onPick,
  onClose,
}: {
  userId: string;
  onPick(clip: live.Clip): void;
  onClose(): void;
}) {
  const { t } = useTranslation("shorts");
  const query = useQuery({
    queryKey: ["live", "clips", "creator", userId],
    queryFn: async () =>
      (
        await clipClient().listClips({
          filter: { case: "creatorId", value: userId },
          page: { limit: 30, cursor: "" },
        })
      ).clips,
  });
  const clips = (query.data ?? []).filter((clip) => clip.status === live.ClipStatus.READY);
  return (
    <Dialog open onOpenChange={(open) => !open && onClose()} title={t("create.clips.title")}>
      {query.isPending ? (
        <Skeleton className="h-40" />
      ) : query.isError ? (
        <EmptyState emoji="😵‍💫" title={errorMessage(query.error, { namespace: "shorts" })} />
      ) : clips.length === 0 ? (
        <EmptyState emoji="✂️" title={t("create.clips.empty")} body={t("create.clips.emptyBody")} />
      ) : (
        <ul className="flex flex-col">
          {clips.map((clip) => (
            <li key={clip.id}>
              <ListRow
                onClick={() => onPick(clip)}
                leading={
                  <img
                    src={imageUrl(clip.thumbnail ?? clip.playback?.poster, 48)}
                    alt=""
                    className="size-12 shrink-0 rounded-xl border-2 border-ink object-cover"
                  />
                }
                title={<EmojiText text={clip.title} />}
                subtitle={
                  <EmojiText text={`${clock(clip.durationSeconds)} · ${clip.sourceTitle}`} />
                }
              />
            </li>
          ))}
        </ul>
      )}
    </Dialog>
  );
}
