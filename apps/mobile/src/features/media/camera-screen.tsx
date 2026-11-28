import { durationLimit } from "@tokuly/core/errors";
import { clock } from "@tokuly/core/shorts";
import { Camera as CameraIcon, RefreshCw, X, Zap, ZapOff } from "lucide-react-native";
import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { Linking, Pressable, Text, View } from "react-native";
import { SafeAreaView } from "react-native-safe-area-context";
import {
  Camera,
  type FlashMode,
  type Recorder,
  useCameraDevice,
  useCameraPermission,
  useMicrophonePermission,
  usePhotoOutput,
  useVideoOutput,
} from "react-native-vision-camera";
import { Button } from "#/components/button";
import i18n from "#/lib/i18n";
import type { LocalMedia } from "./posts";

const FLASH_ORDER: FlashMode[] = ["off", "auto", "on"];

/** Asks for the camera (and, to record with sound, the microphone), explaining why first. */
function PermissionCard({
  onAllow,
  onChooseImage,
  blocked,
  onClose,
}: {
  onAllow(): void;
  onChooseImage?: () => void;
  blocked: boolean;
  onClose(): void;
}) {
  const { t } = useTranslation("mobile");
  return (
    <SafeAreaView
      className="flex-1 items-center justify-center gap-4 bg-bg px-8"
      testID="camera-permission"
    >
      <Text className="text-5xl">📸</Text>
      <Text className="text-center text-xl font-black text-ink">
        {t(blocked ? "camera.blocked.title" : "camera.explain.title")}
      </Text>
      <Text className="text-center text-base text-ink-soft">
        {t(blocked ? "camera.blocked.body" : "camera.explain.body")}
      </Text>
      {onChooseImage && (
        <Button variant="secondary" onPress={onChooseImage}>
          {t("camera.choosePhoto")}
        </Button>
      )}
      {blocked ? (
        <Button onPress={() => void Linking.openSettings()}>{t("sharing.openSettings")}</Button>
      ) : (
        <Button testID="camera-allow" onPress={onAllow}>
          {t("camera.allow")}
        </Button>
      )}
      <Button variant="ghost" onPress={onClose}>
        {t("camera.close")}
      </Button>
    </SafeAreaView>
  );
}

/**
 * The camera for stories and shorts: a tap takes a photo (stories), holding
 * the shutter records (a short: a tap starts and stops), up to the
 * purpose's limit, which is shown before anything is recorded. Front or
 * back, flash, and the permission flow explained first.
 */
export function CameraScreen({
  limitSeconds,
  photos,
  photoOnly = false,
  initialPosition = "back",
  photoOnlyHint,
  onChooseImage,
  onCapture,
  onClose,
}: {
  /** The longest recording the server takes for this purpose. */
  limitSeconds: number;
  /** Whether a tap takes a photo (stories) or only videos are taken (shorts). */
  photos: boolean;
  /** Use the camera for one photo only, without asking for microphone access. */
  photoOnly?: boolean;
  /** Camera used when this screen opens. */
  initialPosition?: "back" | "front";
  /** Caller-provided guidance for the current photo in a one-shot flow. */
  photoOnlyHint?: string;
  /** Optional fallback for photo capture when the camera cannot be used. */
  onChooseImage?(): void;
  onCapture(media: LocalMedia): void;
  onClose(): void;
}) {
  const { t } = useTranslation("mobile");
  const camera = useCameraPermission();
  const microphone = useMicrophonePermission();
  const [position, setPosition] = useState<"back" | "front">(initialPosition);
  const [flash, setFlash] = useState<FlashMode>("off");
  const [recording, setRecording] = useState(false);
  const [elapsed, setElapsed] = useState(0);
  const [captureError, setCaptureError] = useState(false);
  const [asked, setAsked] = useState(false);
  const recorder = useRef<Recorder | null>(null);
  const device = useCameraDevice(position);
  const photoOutput = usePhotoOutput({ containerFormat: "jpeg", quality: 0.9 });
  const videoOutput = useVideoOutput({ enableAudio: microphone.hasPermission, fileType: "mp4" });

  useEffect(() => {
    if (!recording) return;
    const started = Date.now();
    const timer = setInterval(() => setElapsed((Date.now() - started) / 1000), 200);
    return () => clearInterval(timer);
  }, [recording]);

  if (!camera.hasPermission) {
    const ask = async () => {
      setAsked(true);
      if (await camera.requestPermission())
        if (!photoOnly && microphone.canRequestPermission) await microphone.requestPermission();
    };
    return (
      <PermissionCard
        blocked={asked && !camera.canRequestPermission}
        onAllow={() => void ask()}
        onChooseImage={onChooseImage}
        onClose={onClose}
      />
    );
  }

  const hasFlash = Boolean(device?.hasFlash || device?.hasTorch);

  const takePhoto = async () => {
    if (!device) return setCaptureError(true);
    try {
      const photo = await photoOutput.capturePhotoToFile(
        { flashMode: hasFlash ? flash : "off" },
        {},
      );
      setCaptureError(false);
      onCapture({
        uri: photo.filePath.startsWith("file://") ? photo.filePath : `file://${photo.filePath}`,
        kind: "image",
        name: "photo.jpg",
        contentType: "image/jpeg",
      });
    } catch {
      setCaptureError(true);
    }
  };

  const startRecording = async () => {
    if (recording) return;
    setElapsed(0);
    setRecording(true);
    try {
      const next = await videoOutput.createRecorder({ maxDuration: limitSeconds });
      recorder.current = next;
      await next.startRecording(
        (filePath) => {
          recorder.current = null;
          setRecording(false);
          onCapture({
            uri: filePath.startsWith("file://") ? filePath : `file://${filePath}`,
            kind: "video",
            name: "video.mp4",
            contentType: "video/mp4",
            duration: next.recordedDuration,
          });
        },
        () => {
          recorder.current = null;
          setRecording(false);
        },
      );
    } catch {
      recorder.current = null;
      setRecording(false);
    }
  };

  const stopRecording = () => void recorder.current?.stopRecording();

  const limit = durationLimit(i18n, limitSeconds);
  return (
    <View className="flex-1 bg-black" testID="camera-screen">
      {device && (
        <Camera
          style={{ position: "absolute", inset: 0 }}
          device={device}
          isActive
          outputs={[photoOutput, videoOutput]}
          torchMode={recording && flash === "on" && device.hasTorch ? "on" : "off"}
          enableNativeZoomGesture
          enableNativeTapToFocusGesture
        />
      )}
      <SafeAreaView className="flex-1 justify-between">
        <View className="flex-row items-center justify-between px-4 pt-2">
          <Pressable
            testID="camera-close"
            accessibilityRole="button"
            accessibilityLabel={t("camera.close")}
            onPress={onClose}
            className="rounded-full bg-black/40 p-2"
          >
            <X color="white" size={26} />
          </Pressable>
          {hasFlash && (
            <Pressable
              testID="camera-flash"
              accessibilityRole="button"
              accessibilityLabel={t(`camera.flash.${flash}`)}
              onPress={() =>
                setFlash(FLASH_ORDER[(FLASH_ORDER.indexOf(flash) + 1) % FLASH_ORDER.length]!)
              }
              className="flex-row items-center gap-1 rounded-full bg-black/40 px-3 py-2"
            >
              {flash === "off" ? (
                <ZapOff color="white" size={20} />
              ) : (
                <Zap color="white" size={20} />
              )}
              <Text className="text-sm font-bold text-white">{t(`camera.flash.${flash}`)}</Text>
            </Pressable>
          )}
        </View>
        <View className="items-center gap-3 px-4 pb-6">
          {!device || captureError ? (
            <View className="items-center gap-3 rounded-card bg-black/50 p-4">
              <Text className="text-center text-sm font-bold text-white">
                {t(captureError ? "camera.captureFailed" : "camera.noDevice")}
              </Text>
              {captureError && device && (
                <Button variant="secondary" onPress={() => setCaptureError(false)}>
                  {t("camera.tryAgain")}
                </Button>
              )}
              {photos && onChooseImage && (
                <Button variant="secondary" onPress={onChooseImage}>
                  {t("camera.choosePhoto")}
                </Button>
              )}
            </View>
          ) : (
            <>
              <Text
                testID="camera-limit"
                className="rounded-full bg-black/50 px-3 py-1 text-sm font-black text-white"
              >
                {recording
                  ? t("camera.recording", { time: clock(elapsed), limit: clock(limitSeconds) })
                  : t(
                      photoOnly
                        ? (photoOnlyHint ?? "camera.hintPhotoOnly")
                        : photos
                          ? "camera.hintStory"
                          : "camera.hintShort",
                      { limit },
                    )}
              </Text>
              <View className="w-full flex-row items-center justify-around">
                <View style={{ width: 48 }} />
                <Pressable
                  testID="camera-shutter"
                  accessibilityRole="button"
                  accessibilityLabel={t(
                    recording ? "camera.stop" : photos ? "camera.shoot" : "camera.record",
                  )}
                  onPress={() => {
                    if (photos && !recording) void takePhoto();
                    else if (!photos) void (recording ? stopRecording() : startRecording());
                  }}
                  onLongPress={photos && !photoOnly ? () => void startRecording() : undefined}
                  onPressOut={photos && !photoOnly && recording ? stopRecording : undefined}
                  delayLongPress={250}
                  className={`items-center justify-center rounded-full border-4 border-white ${recording ? "bg-live" : "bg-white/30"}`}
                  style={{ width: 84, height: 84 }}
                >
                  <CameraIcon color="white" size={30} />
                </Pressable>
                {photoOnly ? (
                  <View style={{ width: 48 }} />
                ) : (
                  <Pressable
                    testID="camera-flip"
                    accessibilityRole="button"
                    accessibilityLabel={t("camera.flip")}
                    disabled={recording}
                    onPress={() => setPosition(position === "back" ? "front" : "back")}
                    className="rounded-full bg-black/40 p-3"
                  >
                    <RefreshCw color="white" size={24} />
                  </Pressable>
                )}
              </View>
            </>
          )}
        </View>
      </SafeAreaView>
    </View>
  );
}
