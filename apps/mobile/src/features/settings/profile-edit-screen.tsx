import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { errorReason, identity, media } from "@tokuly/api";
import { graphemes } from "@tokuly/core/format";
import { profileKeys } from "@tokuly/core/profile";
import { Image } from "expo-image";
import * as ImagePicker from "expo-image-picker";
import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { ActivityIndicator, Alert, Linking, Platform, View } from "react-native";
import { Avatar } from "#/components/avatar";
import { Button } from "#/components/button";
import { Page, Section } from "#/components/page";
import { TextField } from "#/components/text-field";
import { type Account, currentAccount, ensureActive, isActive } from "#/features/session/account";
import { useMe } from "#/features/session/store";
import { authClient, profileClient } from "#/lib/api";
import { SERVER_URL } from "#/lib/config";
import { errorMessage } from "#/lib/errors";
import { useTheme } from "#/lib/theme";
import { uploadAndProcess } from "#/lib/upload";

const MAX_NAME = 40;
const MAX_BIO = 300;
const MAX_SCHOOL = 60;

/** Puts the account the server answered with into the session, while it is still the active one. */
function adopt(account: Account, me: identity.Me | undefined) {
  if (me && isActive(account)) account.api.session.updateMe(me);
}

const absolute = (url: string) => (url.startsWith("/") ? `${SERVER_URL}${url}` : url);

/**
 * Everything other people see about the account, as on the web: the avatar
 * and banner (picked and cropped by the system picker), the name, handle,
 * bio and school. The status stays on the web.
 */
export function ProfileEditScreen() {
  const me = useMe();
  if (!me?.user) return null;
  return (
    <Page testID="profile-edit">
      <ProfileImages me={me} />
      <ProfileForm me={me} />
    </Page>
  );
}

type Slot = "avatar" | "banner";

function ProfileImages({ me }: { me: identity.Me }) {
  const { t } = useTranslation(["settings", "mobile", "common"]);
  const { colors } = useTheme();
  const query = useQueryClient();
  const change = useMutation({
    mutationFn: async ({ slot, remove }: { slot: Slot; remove?: boolean }) => {
      const account = currentAccount();
      if (!account) return null;
      let assetId = "";
      if (!remove) {
        const permission = await ImagePicker.requestMediaLibraryPermissionsAsync();
        if (!permission.granted) {
          Alert.alert(t("mobile:chat.photoDenied"), undefined, [
            { text: t("common:actions.cancel"), style: "cancel" },
            { text: t("mobile:sharing.openSettings"), onPress: () => void Linking.openSettings() },
          ]);
          return null;
        }
        const picked = await ImagePicker.launchImageLibraryAsync({
          mediaTypes: ["images"],
          // iOS crops to a square only: there the banner is shown as it is, centred.
          allowsEditing: slot === "avatar" || Platform.OS === "android",
          aspect: slot === "avatar" ? [1, 1] : [3, 1],
          quality: 0.9,
          exif: false,
          // An uncropped iPhone photo would be HEIC, which the server does not decode.
          preferredAssetRepresentationMode:
            ImagePicker.UIImagePickerPreferredAssetRepresentationMode.Compatible,
        });
        const image = picked.canceled ? undefined : picked.assets[0];
        if (!image) return null;
        ensureActive(account);
        const asset = await uploadAndProcess(
          { uri: image.uri, name: `${slot}.jpg`, type: image.mimeType ?? "image/jpeg" },
          {
            purpose: slot === "avatar" ? media.AssetPurpose.AVATAR : media.AssetPurpose.BANNER,
            api: account.api,
          },
        );
        assetId = asset.id;
      }
      const response = await account.api
        .service(identity.AccountService)
        .updateProfile(slot === "avatar" ? { avatarAssetId: assetId } : { bannerAssetId: assetId });
      adopt(account, response.me);
      return remove ? t(`settings:profile.${slot}.removed`) : t(`settings:profile.${slot}.changed`);
    },
    onSuccess: (done) => {
      if (!done) return;
      void query.invalidateQueries({ queryKey: profileKeys.profiles });
      Alert.alert(done);
    },
    onError: (error) => Alert.alert(errorMessage(error, { namespace: "settings" })),
  });
  const confirmRemove = (slot: Slot) =>
    Alert.alert(
      t(`settings:profile.${slot}.removeTitle`),
      t(`settings:profile.${slot}.removeBody`),
      [
        { text: t("common:actions.cancel"), style: "cancel" },
        {
          text: t("settings:profile.removeConfirm"),
          style: "destructive",
          onPress: () => change.mutate({ slot, remove: true }),
        },
      ],
    );
  const banner = me.banner?.url;
  return (
    <View className="overflow-hidden rounded-card bg-surface">
      <View className="aspect-[3/1] bg-brand-soft">
        {banner && (
          <Image
            testID="banner-preview"
            source={{ uri: absolute(banner) }}
            contentFit="cover"
            style={{ width: "100%", height: "100%" }}
          />
        )}
        {change.isPending && (
          <View className="absolute inset-0 items-center justify-center bg-black/30">
            <ActivityIndicator color={colors.surface} />
          </View>
        )}
      </View>
      <View className="-mt-8 px-4">
        <Avatar user={me.user} size="xl" outlined />
      </View>
      <View className="flex-row flex-wrap gap-2 p-4">
        <Button
          testID="profile-avatar-change"
          size="sm"
          variant="secondary"
          disabled={change.isPending}
          onPress={() => change.mutate({ slot: "avatar" })}
        >
          {t("settings:profile.avatar.change")}
        </Button>
        <Button
          testID="profile-banner-change"
          size="sm"
          variant="secondary"
          disabled={change.isPending}
          onPress={() => change.mutate({ slot: "banner" })}
        >
          {t("settings:profile.banner.change")}
        </Button>
        {me.user?.avatar?.url ? (
          <Button
            size="sm"
            variant="ghost"
            disabled={change.isPending}
            onPress={() => confirmRemove("avatar")}
          >
            {t("settings:profile.avatar.remove")}
          </Button>
        ) : null}
        {banner ? (
          <Button
            size="sm"
            variant="ghost"
            disabled={change.isPending}
            onPress={() => confirmRemove("banner")}
          >
            {t("settings:profile.banner.remove")}
          </Button>
        ) : null}
      </View>
    </View>
  );
}

type HandleCheck =
  | { status: "unchanged" | "checking" | "available" }
  | { status: "unavailable"; reason: string };

/** Asks the server whether a new handle is free, a moment after typing stops. */
function useHandleCheck(handle: string, current: string): HandleCheck {
  const [state, setState] = useState<HandleCheck>({ status: "unchanged" });
  useEffect(() => {
    if (handle === current) {
      setState({ status: "unchanged" });
      return;
    }
    setState({ status: "checking" });
    let cancelled = false;
    const timer = setTimeout(async () => {
      try {
        const result = await authClient().checkHandle({ handle });
        if (!cancelled)
          setState(
            result.available
              ? { status: "available" }
              : { status: "unavailable", reason: result.reason },
          );
      } catch {
        // Not knowing is not a reason to block: saving checks again.
        if (!cancelled) setState({ status: "unchanged" });
      }
    }, 350);
    return () => {
      cancelled = true;
      clearTimeout(timer);
    };
  }, [handle, current]);
  return state;
}

/** Which field an API reason belongs under. */
const FIELD_OF_REASON: Record<string, "displayName" | "handle" | "bio" | "school"> = {
  DISPLAY_NAME_INVALID: "displayName",
  HANDLE_INVALID: "handle",
  HANDLE_RESERVED: "handle",
  HANDLE_TAKEN: "handle",
  BIO_TOO_LONG: "bio",
  SCHOOL_NAME_INVALID: "school",
};

/** Name, handle, bio and school: the text on the profile. */
function ProfileForm({ me }: { me: identity.Me }) {
  const { t, i18n } = useTranslation(["settings", "common"]);
  const { colors } = useTheme();
  const query = useQueryClient();
  const user = me.user;
  const currentHandle = user?.handle ?? "";

  // The school is not part of the account message; the profile carries it.
  const profile = useQuery({
    queryKey: profileKeys.profile(currentHandle),
    queryFn: () => profileClient().getProfile({ by: { case: "handle", value: currentHandle } }),
    enabled: currentHandle !== "",
  });
  const savedSchool = profile.data?.profile?.schoolName ?? "";

  const [displayName, setDisplayName] = useState(user?.displayName ?? "");
  const [handle, setHandle] = useState(currentHandle);
  const [bio, setBio] = useState(me.bio);
  // `null` until edited, because the saved value arrives after the form is shown.
  const [schoolDraft, setSchoolDraft] = useState<string | null>(null);
  const school = schoolDraft ?? savedSchool;
  const [fieldError, setFieldError] = useState<{ field: string; message: string } | null>(null);
  const handleCheck = useHandleCheck(handle, currentHandle);

  const changes = {
    displayName: displayName.trim() !== (user?.displayName ?? "") ? displayName.trim() : undefined,
    handle: handle !== currentHandle ? handle : undefined,
    bio: bio.trim() !== me.bio ? bio.trim() : undefined,
    schoolName:
      schoolDraft !== null && schoolDraft.trim() !== savedSchool ? schoolDraft.trim() : undefined,
  };
  const dirty = Object.values(changes).some((value) => value !== undefined);
  const lengths = {
    name: graphemes(displayName.trim()).length,
    bio: graphemes(bio.trim()).length,
    school: graphemes(school.trim()).length,
  };
  const invalid =
    lengths.name < 1 ||
    lengths.name > MAX_NAME ||
    lengths.bio > MAX_BIO ||
    lengths.school > MAX_SCHOOL ||
    handleCheck.status === "unavailable";

  const save = useMutation({
    mutationFn: async () => {
      const account = currentAccount();
      if (!account) return undefined;
      const response = await account.api.service(identity.AccountService).updateProfile(changes);
      adopt(account, response.me);
      return response.me;
    },
    onSuccess: (saved) => {
      if (!saved) return;
      // Show what the server stored (it trims), and stop treating the school as edited.
      setDisplayName(saved.user?.displayName ?? "");
      setHandle(saved.user?.handle ?? "");
      setBio(saved.bio);
      setSchoolDraft(null);
      setFieldError(null);
      void query.invalidateQueries({ queryKey: profileKeys.profiles });
      Alert.alert(t("settings:profile.saved"));
    },
    onError: (error) => {
      const field = FIELD_OF_REASON[errorReason(error) ?? ""];
      const message = errorMessage(error, { namespace: "settings" });
      if (field) setFieldError({ field, message });
      else Alert.alert(message);
    },
  });
  const errorFor = (field: string) =>
    fieldError?.field === field ? fieldError.message : undefined;

  const handleProblem =
    handleCheck.status === "unavailable"
      ? t(`settings:profile.handle.reasons.${handleCheck.reason}`, {
          defaultValue: t("settings:profile.handle.reasons.INVALID_FORMAT"),
        })
      : errorFor("handle");
  const birthdate = me.birthdate ? new Date(`${me.birthdate}T00:00:00`) : undefined;

  return (
    <Section title={t("settings:profile.about.title")}>
      <View className="gap-4 p-4">
        <TextField
          testID="profile-display-name"
          label={t("settings:profile.displayName.label")}
          hint={t("settings:profile.displayName.hint")}
          error={
            lengths.name > MAX_NAME
              ? t("settings:profile.tooLong", { max: MAX_NAME })
              : errorFor("displayName")
          }
          value={displayName}
          onChangeText={setDisplayName}
          autoComplete="nickname"
        />
        <TextField
          testID="profile-handle"
          label={t("settings:profile.handle.label")}
          hint={
            handleCheck.status === "available"
              ? t("settings:profile.handle.available")
              : t("settings:profile.handle.hint")
          }
          error={handleProblem}
          value={handle}
          onChangeText={(value) => setHandle(value.toLowerCase().replace(/[^a-z0-9_-]/g, ""))}
          autoCapitalize="none"
          autoCorrect={false}
          maxLength={24}
        />
        <TextField
          testID="profile-bio"
          label={t("settings:profile.bio.label")}
          placeholder={t("settings:profile.bio.placeholder")}
          hint={`${lengths.bio} / ${MAX_BIO}`}
          error={
            lengths.bio > MAX_BIO
              ? t("settings:profile.tooLong", { max: MAX_BIO })
              : errorFor("bio")
          }
          value={bio}
          onChangeText={setBio}
          multiline
          style={{ height: 112, paddingTop: 12, textAlignVertical: "top" }}
        />
        <TextField
          testID="profile-school"
          label={t("settings:profile.school.label")}
          placeholder={t("settings:profile.school.placeholder")}
          hint={t("settings:profile.school.hint")}
          error={
            lengths.school > MAX_SCHOOL
              ? t("settings:profile.tooLong", { max: MAX_SCHOOL })
              : errorFor("school")
          }
          value={school}
          editable={!profile.isPending}
          onChangeText={setSchoolDraft}
        />
        {birthdate && (
          <TextField
            label={t("settings:profile.birthdate.label")}
            hint={t("settings:profile.birthdate.hint")}
            value={birthdate.toLocaleDateString(i18n.language, { dateStyle: "long" })}
            editable={false}
            style={{ color: colors["ink-soft"] }}
          />
        )}
        <Button
          testID="profile-save"
          loading={save.isPending}
          disabled={!dirty || invalid || handleCheck.status === "checking"}
          onPress={() => {
            setFieldError(null);
            save.mutate();
          }}
        >
          {t("common:actions.save")}
        </Button>
      </View>
    </Section>
  );
}
