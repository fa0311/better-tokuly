import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import type { identity } from "@tokuly/api";
import { errorReason } from "@tokuly/api";
import { profileKeys } from "@tokuly/core/profile";
import { Check, X } from "lucide-react";
import { type FormEvent, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { Button } from "#/components/ui/button";
import { TextArea, TextField } from "#/components/ui/field";
import { Skeleton } from "#/components/ui/page";
import { Spinner } from "#/components/ui/spinner";
import { accountClient, authClient, profileClient } from "#/lib/api";
import { errorMessage } from "#/lib/errors";
import { adoptMe, keys } from "../data";
import { Group } from "../shell/settings-page";
import { graphemes } from "./graphemes";

const MAX_NAME = 40;
const MAX_BIO = 300;
const MAX_SCHOOL = 60;

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

/** Which form field an API reason belongs under. */
const FIELD_OF_REASON: Record<string, "displayName" | "handle" | "bio" | "school"> = {
  DISPLAY_NAME_INVALID: "displayName",
  HANDLE_INVALID: "handle",
  HANDLE_RESERVED: "handle",
  HANDLE_TAKEN: "handle",
  BIO_TOO_LONG: "bio",
  SCHOOL_NAME_INVALID: "school",
};

/** Name, handle, bio and school: the text on the profile. */
export function ProfileForm({ me }: { me: identity.Me }) {
  const { t, i18n } = useTranslation(["settings", "common"]);
  const queryClient = useQueryClient();
  const user = me.user;
  const currentHandle = user?.handle ?? "";

  // The school is not part of the account message; the profile carries it.
  const profile = useQuery({
    queryKey: keys.profile(currentHandle),
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
    mutationFn: () => accountClient().updateProfile(changes),
    onSuccess: (response) => {
      const saved = response.me;
      adoptMe(saved);
      // Show what the server stored (it trims), and stop treating the school as edited.
      setDisplayName(saved?.user?.displayName ?? "");
      setHandle(saved?.user?.handle ?? "");
      setBio(saved?.bio ?? "");
      const newHandle = saved?.user?.handle ?? currentHandle;
      const previous = profile.data;
      if (previous?.profile) {
        queryClient.setQueryData(keys.profile(newHandle), {
          ...previous,
          profile: {
            ...previous.profile,
            user: saved?.user,
            bio: saved?.bio ?? "",
            schoolName: changes.schoolName ?? savedSchool,
          },
        });
      }
      setSchoolDraft(null);
      setFieldError(null);
      void queryClient.invalidateQueries({ queryKey: profileKeys.profiles });
      toast.success(t("settings:profile.saved"));
    },
    onError: (error) => {
      const field = FIELD_OF_REASON[errorReason(error) ?? ""];
      if (field) setFieldError({ field, message: errorMessage(error) });
      else toast.error(errorMessage(error));
    },
  });

  const submit = (event: FormEvent) => {
    event.preventDefault();
    setFieldError(null);
    if (dirty && !invalid) save.mutate();
  };
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
    <Group title={t("settings:profile.about.title")}>
      <form onSubmit={submit} className="flex flex-col gap-4">
        <TextField
          label={t("settings:profile.displayName.label")}
          hint={t("settings:profile.displayName.hint")}
          value={displayName}
          onChange={(event) => setDisplayName(event.target.value)}
          error={
            lengths.name > MAX_NAME
              ? t("settings:profile.tooLong", { max: MAX_NAME })
              : errorFor("displayName")
          }
          autoComplete="nickname"
          required
        />
        <TextField
          label={t("settings:profile.handle.label")}
          value={handle}
          onChange={(event) =>
            setHandle(event.target.value.toLowerCase().replace(/[^a-z0-9_-]/g, ""))
          }
          autoComplete="off"
          autoCapitalize="none"
          spellCheck={false}
          maxLength={24}
          required
          hint={
            handleCheck.status === "available" ? (
              <span className="font-bold text-success">
                {t("settings:profile.handle.available")}
              </span>
            ) : (
              t("settings:profile.handle.hint")
            )
          }
          error={handleProblem}
          trailing={
            handleCheck.status === "checking" ? (
              <Spinner className="mr-2 size-5 text-ink-faint" />
            ) : handleCheck.status === "available" ? (
              <Check className="mr-2 size-5 text-success" aria-hidden />
            ) : handleCheck.status === "unavailable" ? (
              <X className="mr-2 size-5 text-danger" aria-hidden />
            ) : null
          }
        />
        <TextArea
          label={t("settings:profile.bio.label")}
          placeholder={t("settings:profile.bio.placeholder")}
          value={bio}
          onChange={(event) => setBio(event.target.value)}
          rows={4}
          hint={`${lengths.bio} / ${MAX_BIO}`}
          error={
            lengths.bio > MAX_BIO
              ? `${lengths.bio} / ${MAX_BIO} — ${t("settings:profile.tooLong", { max: MAX_BIO })}`
              : errorFor("bio")
          }
        />
        {profile.isPending ? (
          <Skeleton className="h-20" />
        ) : (
          <TextField
            label={t("settings:profile.school.label")}
            hint={t("settings:profile.school.hint")}
            placeholder={t("settings:profile.school.placeholder")}
            value={school}
            onChange={(event) => setSchoolDraft(event.target.value)}
            error={
              lengths.school > MAX_SCHOOL
                ? t("settings:profile.tooLong", { max: MAX_SCHOOL })
                : errorFor("school")
            }
            autoComplete="organization"
          />
        )}
        {birthdate && (
          <TextField
            label={t("settings:profile.birthdate.label")}
            hint={t("settings:profile.birthdate.hint")}
            value={birthdate.toLocaleDateString(i18n.language, { dateStyle: "long" })}
            readOnly
            disabled
          />
        )}
        <Button
          type="submit"
          className="self-end"
          loading={save.isPending}
          disabled={!dirty || invalid || handleCheck.status === "checking"}
        >
          {t("common:actions.save")}
        </Button>
      </form>
    </Group>
  );
}
