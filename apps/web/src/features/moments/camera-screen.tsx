import { useMutation, useQueryClient } from "@tanstack/react-query";
import { useNavigate } from "@tanstack/react-router";
import { media, moments } from "@tokuly/api";
import {
  CAPTION_MAX,
  cleanCaption,
  countdownText,
  momentPhase,
  rememberPostedMoment,
} from "@tokuly/core/moments";
import { Circle, ImagePlus, MapPin, RefreshCcw, Send } from "lucide-react";
import { useEffect, useRef, useState } from "react";
import { toast } from "sonner";
import { Button } from "#/components/ui/button";
import { EmptyState, Page, Skeleton } from "#/components/ui/page";
import { SwitchRow } from "#/components/ui/switch";
import { Tabs } from "#/components/ui/tabs";
import { cn } from "#/lib/cn";
import { errorMessage } from "#/lib/errors";
import { uploadAndProcess } from "#/lib/upload";
import { useTranslation } from "#/lib/use-translation";
import { useHideTabBar } from "#/stores/shell";
import { momentClient, useToday } from "./api";
import { type Facing, openCamera, openRig, type Rig, snap, stopRig } from "./camera";
import { LiveVideo } from "./live-video";
import { useNow } from "./use-now";

interface Shot {
  blob: Blob;
  url: string;
}

type Audience = "friends" | "close";

/**
 * Takes today's moment: the back and the front camera (at once where the
 * device runs both, otherwise one after the other), with the window's
 * countdown on top, then an optional caption, the place and who sees it.
 */
export function MomentCameraScreen() {
  const { t } = useTranslation("moments");
  const today = useToday();
  useHideTabBar();
  const now = useNow(250, Boolean(today.data?.day && !today.data.posted));
  const phase = momentPhase(today.data, now);

  return (
    <Page title={t("camera.title")} back={{ fallback: "/moments" }}>
      {today.isPending ? (
        <Skeleton className="aspect-[3/4] w-full" />
      ) : phase.kind === "waiting" ? (
        <EmptyState emoji="⏳" title={t("camera.noPrompt")} body={t("today.waitingBody")} />
      ) : phase.kind === "posted" ? (
        <EmptyState emoji="📸" title={t("camera.already")} />
      ) : (
        <>
          <p
            className={cn(
              "sticker mb-3 rounded-full px-4 py-2 text-center text-sm font-black tabular-nums",
              phase.kind === "open" ? "bg-sun text-[#17122b]" : "bg-surface-2 text-ink-soft",
            )}
            role="timer"
            data-testid="moment-countdown"
          >
            {phase.kind === "open"
              ? `⚡ ${t("camera.open", { time: countdownText(phase.leftMs) })}`
              : t("camera.late")}
          </p>
          <Composer settings={today.data?.settings} />
        </>
      )}
    </Page>
  );
}

function Composer({ settings }: { settings: moments.MomentSettings | undefined }) {
  const { t } = useTranslation("moments");
  const queryClient = useQueryClient();
  const navigate = useNavigate();
  const [rig, setRig] = useState<Rig | null>(null);
  // Whatever camera is open now, to let it go with the screen.
  const openRigRef = useRef<Rig | null>(null);
  useEffect(() => {
    openRigRef.current = rig;
  }, [rig]);
  const [back, setBack] = useState<Shot | null>(null);
  const [front, setFront] = useState<Shot | null>(null);
  const [retakes, setRetakes] = useState(0);
  const [caption, setCaption] = useState("");
  const [withPlace, setWithPlace] = useState(settings?.shareLocation ?? false);
  const [audience, setAudience] = useState<Audience>(
    settings?.audience === moments.MomentAudience.CLOSE_FRIENDS ? "close" : "friends",
  );
  const backVideo = useRef<HTMLVideoElement | null>(null);
  const frontVideo = useRef<HTMLVideoElement | null>(null);
  const backPicker = useRef<HTMLInputElement>(null);
  const frontPicker = useRef<HTMLInputElement>(null);
  // The stream whose picture the preview shows: a shot is taken only from that one.
  const [playing, setPlaying] = useState<MediaStream | null>(null);
  const shown = rig?.kind === "dual" ? rig.back : rig?.kind === "single" ? rig.stream : null;
  const done = Boolean(back && front);
  // One camera: after the back picture, the same camera turns to the front.
  const needFacing: Facing = back ? "user" : "environment";
  const live =
    shown !== null && playing === shown && (rig?.kind !== "single" || rig.facing === needFacing);

  // The cameras run until both pictures are taken, and are let go with the screen.
  useEffect(() => {
    if (done) return;
    let cancelled = false;
    openRig().then((next) => {
      if (cancelled) return stopRig(next);
      openRigRef.current = next;
      setRig(next);
    });
    return () => {
      cancelled = true;
      stopRig(openRigRef.current);
      openRigRef.current = null;
      setRig(null);
    };
  }, [done]);

  useEffect(() => {
    if (rig?.kind !== "single" || rig.facing === needFacing || done) return;
    let cancelled = false;
    stopRig(rig);
    openCamera(needFacing).then(
      (stream) => {
        const next: Rig = { kind: "single", stream, facing: needFacing };
        if (cancelled) return stopRig(next);
        openRigRef.current = next;
        setRig(next);
      },
      () => !cancelled && setRig({ kind: "unavailable" }),
    );
    return () => {
      cancelled = true;
    };
  }, [rig, needFacing, done]);

  // Pictures not posted are let go with the screen.
  const shots = useRef<Shot[]>([]);
  useEffect(
    () => () => {
      for (const shot of shots.current) URL.revokeObjectURL(shot.url);
    },
    [],
  );
  const keep = (blob: Blob): Shot => {
    const shot = { blob, url: URL.createObjectURL(blob) };
    shots.current.push(shot);
    return shot;
  };

  const shoot = async () => {
    if (!live) return;
    if (rig?.kind === "dual" && backVideo.current && frontVideo.current) {
      const [backBlob, frontBlob] = await Promise.all([
        snap(backVideo.current, false),
        snap(frontVideo.current, true),
      ]);
      if (backBlob && frontBlob) {
        setBack(keep(backBlob));
        setFront(keep(frontBlob));
      }
    } else if (rig?.kind === "single" && backVideo.current) {
      const blob = await snap(backVideo.current, rig.facing === "user");
      if (!blob) return;
      if (!back) setBack(keep(blob));
      else setFront(keep(blob));
    }
  };

  const retake = () => {
    setBack(null);
    setFront(null);
    setRetakes((count) => Math.min(99, count + 1));
  };

  const pick = (files: FileList | null, side: "back" | "front") => {
    const file = files?.[0];
    if (!file?.type.startsWith("image/")) return;
    (side === "back" ? setBack : setFront)(keep(file));
  };

  const cleaned = cleanCaption(caption);
  const post = useMutation({
    mutationFn: async () => {
      if (!back || !front) throw new Error("two pictures");
      const [location, backAsset, frontAsset] = await Promise.all([
        withPlace ? here().catch(() => null) : Promise.resolve(null),
        uploadAndProcess(named(back.blob, "back"), { purpose: media.AssetPurpose.MOMENT }),
        uploadAndProcess(named(front.blob, "front"), { purpose: media.AssetPurpose.MOMENT }),
      ]);
      if (withPlace && !location) toast(t("camera.locationFailed"));
      return momentClient().postMoment({
        backAssetId: backAsset.id,
        frontAssetId: frontAsset.id,
        caption: cleaned ?? "",
        location: location ?? undefined,
        retakes,
        audience:
          audience === "close"
            ? moments.MomentAudience.CLOSE_FRIENDS
            : moments.MomentAudience.FRIENDS,
      });
    },
    onSuccess: ({ moment }) => {
      if (moment) rememberPostedMoment(queryClient, moment);
      toast.success(t("camera.posted"));
      void navigate({ to: "/moments" });
    },
    onError: (error) => toast.error(errorMessage(error)),
  });

  const singleFacing = rig?.kind === "single" ? rig.facing : "environment";
  return (
    <div className="flex flex-col gap-4">
      <div className="relative aspect-[3/4] w-full overflow-hidden rounded-card bg-black">
        {done && back && front ? (
          <>
            <img
              src={back.url}
              alt={t("card.back")}
              className="absolute inset-0 size-full object-cover"
            />
            <img
              src={front.url}
              alt={t("card.front")}
              className="absolute top-3 left-3 aspect-[3/4] w-[30%] rounded-2xl border-[3px] border-ink object-cover shadow-pop-sm"
            />
          </>
        ) : rig?.kind === "unavailable" ? (
          <div className="flex size-full flex-col items-center justify-center gap-3 p-6 text-center text-white">
            <p className="text-5xl">📸</p>
            <p className="text-sm">{t("camera.unavailable")}</p>
            <Button
              variant="secondary"
              icon={<ImagePlus className="size-4" />}
              onClick={() => (back ? frontPicker : backPicker).current?.click()}
            >
              {back ? t("camera.pickFront") : t("camera.pickBack")}
            </Button>
          </div>
        ) : rig?.kind === "dual" ? (
          <>
            <LiveVideo
              stream={rig.back}
              mirror={false}
              videoRef={backVideo}
              onLive={() => setPlaying(rig.back)}
              className="absolute inset-0 size-full"
              testId="moment-preview-back"
            />
            <LiveVideo
              stream={rig.front}
              mirror
              videoRef={frontVideo}
              className="absolute top-3 left-3 aspect-[3/4] w-[30%] rounded-2xl border-[3px] border-ink"
              testId="moment-preview-front"
            />
          </>
        ) : rig?.kind === "single" ? (
          <>
            <LiveVideo
              key={rig.facing}
              stream={rig.stream}
              mirror={rig.facing === "user"}
              videoRef={backVideo}
              onLive={() => setPlaying(rig.stream)}
              className="absolute inset-0 size-full"
              testId="moment-preview-back"
            />
            {back && (
              <img
                src={back.url}
                alt={t("card.back")}
                className="absolute top-3 left-3 aspect-[3/4] w-[30%] rounded-2xl border-[3px] border-ink object-cover"
              />
            )}
            <span className="glass absolute bottom-3 left-1/2 -translate-x-1/2 rounded-full px-3 py-1 text-xs font-black">
              {back ? t("camera.shootFront") : t("camera.shootBack")}
            </span>
          </>
        ) : (
          <Skeleton className="size-full rounded-none" />
        )}
      </div>

      {!done ? (
        <div className="flex items-center justify-center gap-6">
          <button
            type="button"
            onClick={() => (back ? frontPicker : backPicker).current?.click()}
            aria-label={back ? t("camera.pickFront") : t("camera.pickBack")}
            className="grid size-11 place-items-center rounded-full text-ink-soft hover:bg-surface-2"
          >
            <ImagePlus className="size-5" />
          </button>
          <button
            type="button"
            onClick={() => void shoot()}
            disabled={!live || rig?.kind === "unavailable"}
            aria-label={t("camera.shoot")}
            className="pressable sticker grid size-18 place-items-center rounded-full bg-surface text-ink disabled:opacity-40"
            data-facing={singleFacing}
          >
            <Circle className="size-10" />
          </button>
          <span className="size-11" aria-hidden />
        </div>
      ) : (
        <>
          <div className="flex justify-center">
            <Button variant="secondary" icon={<RefreshCcw className="size-4" />} onClick={retake}>
              {t("camera.retake")}
            </Button>
          </div>
          <label className="flex flex-col gap-1.5">
            <span className="text-sm font-bold">{t("camera.caption")}</span>
            <input
              value={caption}
              onChange={(event) => setCaption(event.target.value)}
              maxLength={CAPTION_MAX * 2}
              aria-invalid={cleaned === null || undefined}
              className="h-11 rounded-control border-2 border-line bg-surface px-4 text-[15px] outline-none placeholder:text-ink-faint focus:border-brand aria-invalid:border-danger"
            />
          </label>
          <SwitchRow
            label={
              <span className="flex items-center gap-1.5">
                <MapPin className="size-4" />
                {t("camera.location")}
              </span>
            }
            hint={t("camera.locationHint")}
            checked={withPlace}
            onChange={setWithPlace}
          />
          <div className="flex items-center justify-between gap-3">
            <span className="text-sm font-bold">{t("camera.audience")}</span>
            <Tabs<Audience>
              value={audience}
              onChange={setAudience}
              items={[
                { value: "friends", label: t("camera.friends") },
                { value: "close", label: t("camera.closeFriends") },
              ]}
            />
          </div>
          <Button
            block
            size="lg"
            variant="lime"
            icon={<Send className="size-5" />}
            loading={post.isPending}
            disabled={cleaned === null}
            onClick={() => post.mutate()}
          >
            {post.isPending ? t("camera.posting") : t("camera.post")}
          </Button>
        </>
      )}

      <input
        ref={backPicker}
        type="file"
        accept="image/*"
        capture="environment"
        className="hidden"
        data-testid="moment-pick-back"
        onChange={(event) => {
          pick(event.target.files, "back");
          event.target.value = "";
        }}
      />
      <input
        ref={frontPicker}
        type="file"
        accept="image/*"
        capture="user"
        className="hidden"
        data-testid="moment-pick-front"
        onChange={(event) => {
          pick(event.target.files, "front");
          event.target.value = "";
        }}
      />
    </div>
  );
}

/** A picture from the camera gets a file name; a picked file keeps its own. */
const named = (blob: Blob, side: string) =>
  blob instanceof File
    ? blob
    : new File([blob], `moment-${side}-${Date.now()}.jpg`, { type: blob.type });

/** Where the device is now, for a moment that carries its place. */
function here(): Promise<{ lat: number; lng: number }> {
  return new Promise((resolve, reject) => {
    if (!navigator.geolocation) return reject(new Error("no geolocation"));
    navigator.geolocation.getCurrentPosition(
      (position) => resolve({ lat: position.coords.latitude, lng: position.coords.longitude }),
      reject,
      { enableHighAccuracy: true, timeout: 8000, maximumAge: 30_000 },
    );
  });
}
