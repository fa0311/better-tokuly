import { useQueryClient } from "@tanstack/react-query";
import { ConnectError, media, moments } from "@tokuly/api";
import {
  CAPTION_MAX,
  cleanCaption,
  countdownText,
  momentKeys,
  momentPhase,
  rememberPostedMoment,
} from "@tokuly/core/moments";
import { randomUUID } from "expo-crypto";
import { Image } from "expo-image";
import * as ImagePicker from "expo-image-picker";
import { router } from "expo-router";
import { Camera, RotateCcw, Users } from "lucide-react-native";
import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { Alert, Linking, Modal, Pressable, Text, View } from "react-native";
import { Button } from "#/components/button";
import { SwitchRow } from "#/components/form-rows";
import { EmptyState, ErrorNote, Page, Spinner } from "#/components/page";
import { TextField } from "#/components/text-field";
import { CameraScreen } from "#/features/media/camera-screen";
import type { LocalMedia } from "#/features/media/posts";
import { type UploadState, uploadFile } from "#/features/media/upload";
import { AccountChanged, currentAccount, ensureActive } from "#/features/session/account";
import { mediaUrl } from "#/lib/config";
import { errorMessage } from "#/lib/errors";
import { useTheme } from "#/lib/theme";
import { askForegroundAccess, currentPosition, readLocationAccess } from "#/platform/location";
import { forgetFiles, keepFile } from "#/platform/transfer";
import { useMomentToday } from "./data";

type Side = "back" | "front";

interface CapturedPhoto {
  media: LocalMedia;
  upload: UploadState;
  assetId?: string;
}

type Photos = Partial<Record<Side, CapturedPhoto>>;

const photoPurpose = media.AssetPurpose.MOMENT;

function momentDate(date: string, language: string): string {
  const [year, month, day] = date.split("-").map(Number);
  return new Intl.DateTimeFormat(language, { dateStyle: "long", timeZone: "UTC" }).format(
    new Date(Date.UTC(year ?? 2000, (month ?? 1) - 1, day ?? 1)),
  );
}

export function MomentComposerScreen() {
  const { t, i18n } = useTranslation("moments");
  const { colors } = useTheme();
  const queryClient = useQueryClient();
  const accountAtCapture = currentAccount();
  const today = useMomentToday();
  const [photos, setPhotos] = useState<Photos>({});
  const [retakes, setRetakes] = useState(0);
  const [caption, setCaption] = useState("");
  const [locationChoice, setLocationChoice] = useState<boolean | undefined>();
  const [cameraFor, setCameraFor] = useState<Side | null>(null);
  const [savingCapture, setSavingCapture] = useState(false);
  const [busy, setBusy] = useState(false);
  const [progress, setProgress] = useState(0);
  const [posted, setPosted] = useState(false);
  const [now, setNow] = useState(Date.now());
  const controller = useRef<AbortController | null>(null);
  const posting = useRef(false);
  const draftId = useRef(randomUUID()).current;
  const includeLocation = locationChoice ?? today.data?.settings?.shareLocation ?? false;
  const day = today.data?.day;
  const phase = momentPhase(today.data, now);
  const cleanedCaption = cleanCaption(caption);

  useEffect(() => {
    if (!day || today.data?.posted) return;
    const timer = setInterval(() => setNow(Date.now()), 1_000);
    return () => clearInterval(timer);
  }, [day, today.data?.posted]);

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
      Alert.alert(t("camera.locationFailed"), undefined, [
        { text: t("composer.continueWithoutLocation") },
        { text: t("composer.openSettings"), onPress: () => void Linking.openSettings() },
      ]);
      return;
    }
    setLocationChoice(true);
  };

  const capture = async (side: Side, file: LocalMedia, account = currentAccount()) => {
    if (!account) return;
    setCameraFor(null);
    setSavingCapture(true);
    let saved = false;
    try {
      const name = `${side}-${file.name}`;
      const kept = await keepFile(draftId, file.uri, name);
      ensureActive(account);
      if (photos[side]) setRetakes((count) => Math.min(count + 1, 99));
      setPhotos((current) => ({
        ...current,
        [side]: {
          media: { ...file, uri: kept.uri, name },
          upload: { uri: kept.uri, name, contentType: file.contentType, size: kept.size },
        },
      }));
      saved = true;
    } catch (error) {
      if (error instanceof AccountChanged) {
        forgetFiles(draftId);
        return;
      }
      Alert.alert(t("composer.captureFailed"));
    } finally {
      setSavingCapture(false);
      if (saved && side === "back" && !photos.front) setCameraFor("front");
    }
  };

  const pickPhoto = async (side: Side, account = currentAccount()) => {
    if (!account) return;
    try {
      const permission = await ImagePicker.requestMediaLibraryPermissionsAsync();
      ensureActive(account);
      if (!permission.granted) {
        Alert.alert(t("composer.photoPermission"));
        return;
      }
      const result = await ImagePicker.launchImageLibraryAsync({
        mediaTypes: ["images"],
        quality: 0.9,
        exif: false,
        preferredAssetRepresentationMode:
          ImagePicker.UIImagePickerPreferredAssetRepresentationMode.Compatible,
      });
      ensureActive(account);
      if (result.canceled || !result.assets[0]) return;
      const asset = result.assets[0];
      const contentType = asset.mimeType ?? "image/jpeg";
      const extension = contentType === "image/png" ? "png" : "jpg";
      await capture(
        side,
        {
          uri: asset.uri,
          kind: "image",
          name: asset.fileName ?? `moment-${side}.${extension}`,
          contentType,
        },
        account,
      );
    } catch (error) {
      if (error instanceof AccountChanged) return;
      Alert.alert(t("composer.captureFailed"));
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
    if (
      posting.current ||
      savingCapture ||
      !photos.back ||
      !photos.front ||
      cleanedCaption === null
    )
      return;
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
          let position: Awaited<ReturnType<typeof currentPosition>> = null;
          try {
            position = await currentPosition();
          } catch {
            // A location failure should not block a photo-only Moment.
          }
          ensureActive(account);
          if (position) location = { lat: position.lat, lng: position.lng };
          else {
            setLocationChoice(false);
            Alert.alert(t("camera.locationFailed"));
          }
        } else {
          setLocationChoice(false);
          Alert.alert(t("camera.locationFailed"));
        }
      }

      const response = await account.api.service(moments.MomentService).postMoment(
        {
          backAssetId,
          frontAssetId,
          caption: cleanedCaption,
          location,
          retakes,
          audience: today.data?.settings?.audience ?? moments.MomentAudience.UNSPECIFIED,
        },
        { signal: abort.signal },
      );
      ensureActive(account);
      setPosted(true);
      forgetFiles(draftId);
      if (response.moment) rememberPostedMoment(queryClient, response.moment);
      else void queryClient.invalidateQueries({ queryKey: momentKeys.all });
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
      Alert.alert(t("composer.postFailed"), errorMessage(error));
    } finally {
      controller.current = null;
      posting.current = false;
      setBusy(false);
    }
  };

  const deadlineLabel =
    phase.kind === "open"
      ? `⚡ ${t("camera.open", { time: countdownText(phase.leftMs) })}`
      : phase.kind === "late"
        ? t("camera.late")
        : undefined;

  if (today.isPending) {
    return (
      <Page testID="moment-composer">
        <Spinner />
        <Text className="text-center text-sm text-ink-soft">{t("composer.loading")}</Text>
      </Page>
    );
  }

  if (today.isError) {
    return (
      <Page testID="moment-composer">
        <ErrorNote>{errorMessage(today.error)}</ErrorNote>
        <Button variant="secondary" onPress={() => void today.refetch()}>
          {t("composer.retry")}
        </Button>
      </Page>
    );
  }

  if (!day) {
    return (
      <Page testID="moment-composer">
        <EmptyState emoji="⏳" title={t("today.waitingTitle")} body={t("today.waitingBody")} />
        <Button variant="secondary" onPress={() => router.push("/moments/friends")}>
          {t("friends.open")}
        </Button>
      </Page>
    );
  }

  if (posted || today.data.posted) {
    const mine = today.data.mine;
    return (
      <Page testID="moment-composer">
        <EmptyState emoji="✨" title={t("today.postedTitle")} body={t("composer.postedBody")} />
        {mine && (
          <View className="flex-row gap-3">
            {mine.back?.url ? (
              <Image
                source={{ uri: mediaUrl(mine.back.url) ?? mine.back.url }}
                contentFit="cover"
                style={{ height: 208, flex: 1, borderRadius: 16 }}
              />
            ) : null}
            {mine.front?.url ? (
              <Image
                source={{ uri: mediaUrl(mine.front.url) ?? mine.front.url }}
                contentFit="cover"
                style={{ height: 208, flex: 1, borderRadius: 16 }}
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
          <Text className="text-xl font-black text-ink">{momentDate(day.date, i18n.language)}</Text>
          {deadlineLabel && <Text className="text-sm text-ink-soft">{deadlineLabel}</Text>}
        </View>

        <Text className="px-1 text-sm text-ink-soft">
          {photos.back ? t("camera.shootFront") : t("camera.shootBack")}
        </Text>
        <View className="flex-row gap-3">
          <PhotoSlot
            label={t("card.back")}
            action={photos.back ? t("camera.retake") : t("camera.shoot")}
            photo={photos.back}
            disabled={busy || savingCapture}
            onPress={() => setCameraFor("back")}
          />
          <PhotoSlot
            label={t("card.front")}
            action={photos.front ? t("camera.retake") : t("camera.shoot")}
            photo={photos.front}
            disabled={busy || savingCapture || (!photos.back && !photos.front)}
            onPress={() => setCameraFor("front")}
          />
        </View>

        <TextField
          testID="moment-caption"
          label={t("camera.caption")}
          hint={`${[...caption].length}/${CAPTION_MAX}`}
          error={cleanedCaption === null ? t("composer.captionTooLong") : undefined}
          value={caption}
          onChangeText={setCaption}
          editable={!busy}
          returnKeyType="done"
        />

        <SwitchRow
          label={t("camera.location")}
          hint={t("camera.locationHint")}
          value={includeLocation}
          disabled={busy}
          onChange={(value) => void changeLocation(value)}
        />

        {busy && (
          <View className="gap-2">
            <View className="h-2 overflow-hidden rounded-full bg-surface-2">
              <View
                className="h-full bg-brand"
                style={{ width: `${Math.round(progress * 100)}%` }}
              />
            </View>
            <Text className="text-center text-xs text-ink-soft">{t("camera.posting")}</Text>
          </View>
        )}
        <Button
          testID="moment-post"
          block
          loading={busy}
          disabled={
            !photos.back || !photos.front || busy || savingCapture || cleanedCaption === null
          }
          onPress={() => void post()}
        >
          {t("camera.post")}
        </Button>
        <Button
          variant="secondary"
          icon={<Users size={16} color={colors.ink} />}
          onPress={() => router.push("/moments/friends")}
        >
          {t("friends.open")}
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
            photoOnlyHint={t(cameraFor === "back" ? "camera.shootBack" : "camera.shootFront")}
            onCapture={(file) => void capture(cameraFor, file, accountAtCapture)}
            onChooseImage={() => {
              const side = cameraFor;
              if (!side) return;
              setCameraFor(null);
              void pickPhoto(side, accountAtCapture);
            }}
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
          <Image
            source={{ uri: photo.media.uri }}
            contentFit="cover"
            style={{ width: "100%", height: "100%" }}
          />
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
