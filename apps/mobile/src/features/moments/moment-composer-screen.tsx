import { useQuery, useQueryClient } from "@tanstack/react-query";
import { ConnectError, media, moments } from "@tokuly/api";
import { randomUUID } from "expo-crypto";
import { Image } from "expo-image";
import { Camera, MapPin, RotateCcw } from "lucide-react-native";
import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { Alert, Linking, Modal, Pressable, Switch, Text, TextInput, View } from "react-native";
import { Button } from "#/components/button";
import { EmptyState, ErrorNote, Page, Spinner } from "#/components/page";
import { CameraScreen } from "#/features/media/camera-screen";
import type { LocalMedia } from "#/features/media/posts";
import { type UploadState, uploadFile } from "#/features/media/upload";
import { AccountChanged, currentAccount, ensureActive } from "#/features/session/account";
import { errorMessage } from "#/lib/errors";
import { useTheme } from "#/lib/theme";
import { askForegroundAccess, currentPosition, readLocationAccess } from "#/platform/location";
import { forgetFiles, keepFile } from "#/platform/transfer";
import { momentClient, momentKeys } from "./data";

type Side = "back" | "front";

interface CapturedPhoto {
  media: LocalMedia;
  upload: UploadState;
  assetId?: string;
}

type Photos = Partial<Record<Side, CapturedPhoto>>;

const photoPurpose = media.AssetPurpose.MOMENT;

export function MomentComposerScreen() {
  const { t, i18n } = useTranslation("moments");
  const { colors } = useTheme();
  const queryClient = useQueryClient();
  const today = useQuery({
    queryKey: momentKeys.today,
    queryFn: () => momentClient().getToday({}),
  });
  const [photos, setPhotos] = useState<Photos>({});
  const [retakes, setRetakes] = useState(0);
  const [caption, setCaption] = useState("");
  const [locationDefault, setLocationDefault] = useState<boolean | undefined>();
  const [locationChoice, setLocationChoice] = useState<boolean | undefined>();
  const [cameraFor, setCameraFor] = useState<Side | null>(null);
  const [savingCapture, setSavingCapture] = useState(false);
  const [busy, setBusy] = useState(false);
  const [progress, setProgress] = useState(0);
  const [posted, setPosted] = useState(false);
  const controller = useRef<AbortController | null>(null);
  const posting = useRef(false);
  const draftId = useRef(randomUUID()).current;
  const includeLocation = locationChoice ?? locationDefault ?? false;
  const day = today.data?.day;

  useEffect(() => {
    if (today.data?.settings) setLocationDefault(today.data.settings.shareLocation);
  }, [today.data?.settings]);

  useEffect(
    () => () => {
      controller.current?.abort();
      forgetFiles(draftId);
    },
    [draftId],
  );

  const changeLocation = async (value: boolean) => {
    if (!value) {
      setLocationChoice(false);
      return;
    }
    const current = await readLocationAccess();
    const access = current.foreground === "granted" ? current : await askForegroundAccess();
    if (access.foreground !== "granted") {
      setLocationChoice(false);
      Alert.alert(t("permission.title"), t("permission.body"), [
        { text: t("permission.withoutLocation") },
        { text: t("permission.openSettings"), onPress: () => void Linking.openSettings() },
      ]);
      return;
    }
    setLocationChoice(true);
  };

  const capture = async (side: Side, file: LocalMedia) => {
    setCameraFor(null);
    setSavingCapture(true);
    try {
      const name = `${side}-${file.name}`;
      const kept = await keepFile(draftId, file.uri, name);
      if (photos[side]) setRetakes((count) => Math.min(count + 1, 99));
      setPhotos((current) => ({
        ...current,
        [side]: {
          media: { ...file, uri: kept.uri, name },
          upload: { uri: kept.uri, name, contentType: file.contentType, size: kept.size },
        },
      }));
    } catch {
      Alert.alert(t("cameraError"));
    } finally {
      setSavingCapture(false);
    }
  };

  const upload = async (
    side: Side,
    account: NonNullable<ReturnType<typeof currentAccount>>,
    signal: AbortSignal,
  ) => {
    const photo = photos[side];
    if (!photo) throw new Error("missing Moment photo");
    if (photo.assetId) return photo.assetId;
    const asset = await uploadFile(account, photoPurpose, photo.upload, {
      until: "ready",
      signal,
      save: (uploadState) =>
        setPhotos((current) => ({
          ...current,
          [side]: current[side] ? { ...current[side]!, upload: uploadState } : undefined,
        })),
      onProgress: (fraction) => setProgress((side === "back" ? 0 : 0.5) + fraction * 0.5),
    });
    ensureActive(account);
    setPhotos((current) => ({
      ...current,
      [side]: current[side] ? { ...current[side]!, assetId: asset.id } : undefined,
    }));
    return asset.id;
  };

  const post = async () => {
    if (posting.current || savingCapture || !photos.back || !photos.front) return;
    const account = currentAccount();
    if (!account) return;
    posting.current = true;
    const abort = new AbortController();
    controller.current = abort;
    setBusy(true);
    setProgress(0);
    try {
      const backAssetId = await upload("back", account, abort.signal);
      const frontAssetId = await upload("front", account, abort.signal);
      ensureActive(account);

      let location: { lat: number; lng: number } | undefined;
      if (includeLocation) {
        let access = await readLocationAccess();
        if (access.foreground !== "granted") access = await askForegroundAccess();
        ensureActive(account);
        if (access.foreground === "granted" && access.servicesEnabled) {
          const position = await currentPosition();
          ensureActive(account);
          if (position) location = { lat: position.lat, lng: position.lng };
        } else {
          setLocationChoice(false);
          Alert.alert(t("permission.title"), t("permission.body"));
        }
      }

      const response = await account.api.service(moments.MomentService).postMoment(
        {
          backAssetId,
          frontAssetId,
          caption: caption.trim(),
          location,
          retakes,
          audience: moments.MomentAudience.UNSPECIFIED,
        },
        { signal: abort.signal },
      );
      ensureActive(account);
      setPosted(true);
      forgetFiles(draftId);
      queryClient.setQueryData(momentKeys.today, (current: typeof today.data) =>
        current
          ? { ...current, mine: response.moment ?? current.mine, posted: true, unlocked: true }
          : current,
      );
      void queryClient.invalidateQueries({ queryKey: momentKeys.today });
    } catch (error) {
      if (error instanceof AccountChanged || abort.signal.aborted) return;
      if (
        error instanceof ConnectError &&
        error.metadata.get("x-tokuly-error") === "ALREADY_POSTED"
      ) {
        const latest = await today.refetch();
        if (latest.data?.posted) {
          setPosted(true);
          forgetFiles(draftId);
          return;
        }
      }
      Alert.alert(t("postError"), errorMessage(error));
    } finally {
      controller.current = null;
      posting.current = false;
      setBusy(false);
    }
  };

  const deadline = day?.onTimeUntil
    ? new Date(Number(day.onTimeUntil.seconds) * 1000 + day.onTimeUntil.nanos / 1_000_000)
    : null;
  const deadlineLabel = deadline
    ? t("windowUntil", {
        time: new Intl.DateTimeFormat(i18n.language, { hour: "numeric", minute: "2-digit" }).format(
          deadline,
        ),
      })
    : undefined;

  if (today.isPending) {
    return (
      <Page testID="moment-composer">
        <Spinner />
        <Text className="text-center text-sm text-ink-soft">{t("loading")}</Text>
      </Page>
    );
  }

  if (today.isError) {
    return (
      <Page testID="moment-composer">
        <ErrorNote>{errorMessage(today.error)}</ErrorNote>
        <Button variant="secondary" onPress={() => void today.refetch()}>
          {t("retry")}
        </Button>
      </Page>
    );
  }

  if (!day) {
    return (
      <Page testID="moment-composer">
        <EmptyState emoji="⏳" title={t("noPrompt.title")} body={t("noPrompt.body")} />
      </Page>
    );
  }

  if (posted || today.data.posted) {
    const mine = today.data.mine;
    return (
      <Page testID="moment-composer">
        <EmptyState emoji="✨" title={t("posted.title")} body={t("posted.body")} />
        {mine && (
          <View className="flex-row gap-3">
            {mine.back?.url ? (
              <Image
                source={{ uri: mine.back.url }}
                contentFit="cover"
                className="h-52 flex-1 rounded-card"
              />
            ) : null}
            {mine.front?.url ? (
              <Image
                source={{ uri: mine.front.url }}
                contentFit="cover"
                className="h-52 flex-1 rounded-card"
              />
            ) : null}
          </View>
        )}
      </Page>
    );
  }

  return (
    <>
      <Page testID="moment-composer">
        <View className="gap-1 rounded-card bg-surface p-4">
          <Text className="text-xl font-black text-ink">{day.date}</Text>
          {deadlineLabel && <Text className="text-sm text-ink-soft">{deadlineLabel}</Text>}
        </View>

        <Text className="px-1 text-sm text-ink-soft">{t("capture.lead")}</Text>
        <View className="flex-row gap-3">
          <PhotoSlot
            label={t("capture.back")}
            action={photos.back ? t("capture.retake") : t("capture.takeBack")}
            photo={photos.back}
            disabled={busy || savingCapture}
            onPress={() => setCameraFor("back")}
          />
          <PhotoSlot
            label={t("capture.front")}
            action={photos.front ? t("capture.retake") : t("capture.takeFront")}
            photo={photos.front}
            disabled={busy || savingCapture}
            onPress={() => setCameraFor("front")}
          />
        </View>

        <View className="gap-2">
          <Text className="px-1 text-xs font-black text-ink-soft">{t("caption")}</Text>
          <TextInput
            testID="moment-caption"
            value={caption}
            onChangeText={setCaption}
            editable={!busy}
            maxLength={100}
            returnKeyType="done"
            className="min-h-12 rounded-control bg-surface px-4 py-3 text-base text-ink"
            placeholder={t("caption")}
            placeholderTextColor={colors["ink-faint"]}
          />
          <Text className="self-end text-xs text-ink-faint">{caption.length}/100</Text>
        </View>

        <View className="flex-row items-center gap-3 rounded-card bg-surface p-4">
          <MapPin size={20} color={colors.ink} />
          <View className="flex-1 gap-1">
            <Text className="font-bold text-ink">{t("location.title")}</Text>
            <Text className="text-xs text-ink-soft">{t("location.hint")}</Text>
          </View>
          <Switch
            accessibilityLabel={t("location.title")}
            value={includeLocation}
            disabled={busy}
            onValueChange={(value) => void changeLocation(value)}
            trackColor={{ true: colors.brand }}
          />
        </View>

        {busy && (
          <View className="gap-2">
            <View className="h-2 overflow-hidden rounded-full bg-surface-2">
              <View
                className="h-full bg-brand"
                style={{ width: `${Math.round(progress * 100)}%` }}
              />
            </View>
            <Text className="text-center text-xs text-ink-soft">{t("posting")}</Text>
          </View>
        )}
        <Button
          testID="moment-post"
          block
          loading={busy}
          disabled={!photos.back || !photos.front || busy || savingCapture}
          onPress={() => void post()}
        >
          {t("post")}
        </Button>
      </Page>

      <Modal
        visible={cameraFor !== null}
        animationType="slide"
        onRequestClose={() => setCameraFor(null)}
      >
        {cameraFor && (
          <CameraScreen
            key={cameraFor}
            limitSeconds={60}
            photos
            photoOnly
            initialPosition={cameraFor}
            onCapture={(file) => void capture(cameraFor, file)}
            onClose={() => setCameraFor(null)}
          />
        )}
      </Modal>
    </>
  );
}

function PhotoSlot({
  label,
  action,
  photo,
  disabled = false,
  onPress,
}: {
  label: string;
  action: string;
  photo?: CapturedPhoto;
  disabled?: boolean;
  onPress(): void;
}) {
  const { colors } = useTheme();
  return (
    <View className="flex-1 gap-2">
      <Text className="px-1 text-xs font-black text-ink-soft">{label}</Text>
      <Pressable
        accessibilityRole="button"
        accessibilityLabel={action}
        disabled={disabled}
        onPress={onPress}
        className="aspect-[3/4] items-center justify-center overflow-hidden rounded-card bg-surface"
      >
        {photo ? (
          <Image source={{ uri: photo.media.uri }} contentFit="cover" className="h-full w-full" />
        ) : (
          <Camera size={30} color={colors["ink-faint"]} />
        )}
      </Pressable>
      <Button
        size="sm"
        variant={photo ? "secondary" : "primary"}
        icon={photo ? <RotateCcw size={16} color={colors.ink} /> : undefined}
        disabled={disabled}
        onPress={onPress}
      >
        {action}
      </Button>
    </View>
  );
}
