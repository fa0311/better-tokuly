import { useQuery } from "@tanstack/react-query";
import { media } from "@tokuly/api";
import { emojiKeys } from "@tokuly/core/emoji";
import { Check, ImagePlus, X } from "lucide-react";
import { type KeyboardEvent, useEffect, useId, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { Chip } from "#/components/ui/card";
import { TextArea, TextField } from "#/components/ui/field";
import { SelectField } from "#/components/ui/select";
import { Spinner } from "#/components/ui/spinner";
import { SwitchRow } from "#/components/ui/switch";
import { cn } from "#/lib/cn";
import { errorMessage } from "#/lib/errors";
import { uploadAndProcess } from "#/lib/upload";
import { submissionClient } from "./api";
import { useCatalog } from "./catalog";
import { ThemePreview, useDebounced } from "./parts";

/** The longest licence note the server keeps. */
export const LICENSE_MAX = 1024;
/** Limits that apply when the server has not said otherwise yet. */
export const DEFAULT_MAX_ALIASES = 16;
const DEFAULT_MAX_BYTES = 1024 * 1024;
const NAME_PATTERN = /^[a-z0-9_]{2,32}$/;
/** What the server accepts as an emoji picture: stills and animations. */
const ACCEPTED_TYPES = ["image/png", "image/webp", "image/jpeg", "image/gif", "image/apng"];
const ACCEPT = ACCEPTED_TYPES.join(",");

/** What an emoji form edits. */
export interface EmojiDraft {
  name: string;
  /** The upload the picture comes from; unset while it is the emoji's current picture. */
  assetId?: string;
  /** What to show as the picture: a local file's address, or the emoji's current picture. */
  pictureUrl?: string;
  category: string;
  aliases: string[];
  license: string;
  sensitive: boolean;
  localOnly: boolean;
}

export const EMPTY_DRAFT: EmojiDraft = {
  name: "",
  category: "",
  aliases: [],
  license: "",
  sensitive: false,
  localOnly: false,
};

/** Whether a draft can be sent: a valid name and a picture that finished uploading. */
export function draftReady(draft: EmojiDraft, uploading: boolean): boolean {
  return NAME_PATTERN.test(draft.name) && Boolean(draft.pictureUrl) && !uploading;
}

export interface EmojiFormProps {
  draft: EmojiDraft;
  onChange(patch: Partial<EmojiDraft>): void;
  /** Told while a picture is on its way to the server: the form cannot be sent meanwhile. */
  onUploading(uploading: boolean): void;
  /** The name the emoji has now (an edit): it is not checked against itself. */
  currentName?: string;
  /** Largest picture accepted, in bytes. */
  maxImageBytes?: number;
  maxAliases?: number;
  /** A moderator's form: reserved names are allowed and the "this server only" flag is offered. */
  staff?: boolean;
}

/**
 * The fields of an emoji: picture (uploaded as soon as it is chosen, with a
 * live preview at text size on both themes), name with an availability
 * check while typing, category, aliases, licence note and flags. Used by the
 * submission form and by the moderators' add and edit dialogs.
 */
export function EmojiForm({
  draft,
  onChange,
  onUploading,
  currentName,
  maxImageBytes = DEFAULT_MAX_BYTES,
  maxAliases = DEFAULT_MAX_ALIASES,
  staff,
}: EmojiFormProps) {
  const { t } = useTranslation("emoji");
  const catalog = useCatalog();
  const fileInput = useRef<HTMLInputElement>(null);
  const [progress, setProgress] = useState<number | null>(null);
  const [pictureError, setPictureError] = useState("");
  const [aliasText, setAliasText] = useState("");
  const aliasId = useId();

  // ── Picture ──
  const choose = async (file: File | undefined) => {
    if (!file) return;
    // The file chooser's filter is a suggestion ("all files", drag and drop): check before spending an upload.
    if (!ACCEPTED_TYPES.includes(file.type)) return setPictureError(t("errors.ASSET_NOT_IMAGE"));
    if (file.size > maxImageBytes)
      return setPictureError(t("form.pictureTooLarge", { size: Math.round(maxImageBytes / 1024) }));
    setPictureError("");
    const local = URL.createObjectURL(file);
    onChange({ pictureUrl: local, assetId: undefined });
    setProgress(0);
    onUploading(true);
    try {
      const asset = await uploadAndProcess(file, {
        purpose: media.AssetPurpose.CUSTOM_EMOJI,
        onProgress: setProgress,
      });
      onChange({ assetId: asset.id });
    } catch (error) {
      onChange({ pictureUrl: undefined, assetId: undefined });
      const message = errorMessage(error, { namespace: "emoji" });
      setPictureError(message);
      toast.error(message);
    } finally {
      setProgress(null);
      onUploading(false);
    }
  };

  // ── Name ──
  const name = draft.name;
  const wellFormed = NAME_PATTERN.test(name) && /[a-z0-9]/.test(name);
  const settledName = useDebounced(name, 350);
  const checkable = wellFormed && settledName === name && name !== currentName;
  const check = useQuery({
    queryKey: emojiKeys.nameCheck(name),
    queryFn: () => submissionClient().checkName({ name }),
    enabled: checkable,
    staleTime: 10_000,
  });
  // A moderator may give a reserved name; everything else the check says holds for them too.
  const refused =
    check.data && !check.data.available && !(staff && check.data.reason === "NAME_RESERVED")
      ? check.data.reason
      : "";
  const nameError =
    name && !wellFormed
      ? t("form.nameInvalid")
      : checkable && refused
        ? t(`errors.${refused}`, { defaultValue: t("errors.NAME_TAKEN") })
        : undefined;
  const nameOk = wellFormed && (name === currentName || (checkable && check.data && !refused));

  // ── Aliases ──
  const addAliases = (text: string) => {
    const words = text
      .split(/[\s,、，]+/)
      .map((word) => word.trim().toLowerCase().slice(0, 32))
      .filter(Boolean);
    if (words.length === 0) return;
    onChange({ aliases: [...new Set([...draft.aliases, ...words])].slice(0, maxAliases) });
    setAliasText("");
  };
  const onAliasKey = (event: KeyboardEvent<HTMLInputElement>) => {
    if (event.nativeEvent.isComposing) return;
    if (event.key === "Enter" || event.key === "," || event.key === " ") {
      if (event.key === "Enter" || aliasText.trim()) event.preventDefault();
      addAliases(aliasText);
    } else if (event.key === "Backspace" && !aliasText && draft.aliases.length > 0) {
      onChange({ aliases: draft.aliases.slice(0, -1) });
    }
  };

  // A category the emoji has but the list lost (renamed meanwhile) stays selectable.
  const categoryNames = catalog.categories.map((entry) => entry.name);
  const categories =
    draft.category && !categoryNames.includes(draft.category)
      ? [...categoryNames, draft.category]
      : categoryNames;

  // The local address of a chosen file is released when the form goes away.
  const picture = draft.pictureUrl;
  useEffect(
    () => () => void (picture?.startsWith("blob:") && URL.revokeObjectURL(picture)),
    [picture],
  );

  return (
    <div className="flex flex-col gap-5">
      <div>
        <p className="px-1 text-sm font-bold text-ink-soft">{t("fields.picture")}</p>
        <div className="mt-1.5 flex items-stretch gap-3">
          <button
            type="button"
            disabled={progress !== null}
            onClick={() => fileInput.current?.click()}
            aria-label={t(picture ? "form.changePicture" : "form.choosePicture")}
            className={cn(
              "pressable relative grid size-28 shrink-0 place-items-center overflow-hidden rounded-card border-2 border-dashed border-ink-faint bg-surface-2",
              picture && "border-solid border-ink bg-surface",
            )}
          >
            {picture ? (
              <img src={picture} alt="" className="size-20 object-contain" draggable={false} />
            ) : (
              <span className="flex flex-col items-center gap-1 text-ink-soft">
                <ImagePlus className="size-7" />
                <span className="text-xs font-bold">{t("form.choosePicture")}</span>
              </span>
            )}
            {progress !== null && (
              <span
                className="absolute inset-0 grid place-items-center bg-overlay text-white"
                role="status"
                aria-label={t("form.uploading")}
              >
                <Spinner className="size-7" />
              </span>
            )}
          </button>
          <div className="min-w-0 flex-1 self-center text-sm text-ink-soft">
            <p>{t("form.pictureHint", { size: Math.round(maxImageBytes / 1024) })}</p>
            {progress !== null && (
              <p className="mt-1 font-bold text-brand">
                {t(progress < 1 ? "form.uploadingPercent" : "form.processing", {
                  percent: Math.round(progress * 100),
                })}
              </p>
            )}
            {pictureError && (
              <p role="alert" className="mt-1 font-bold text-danger">
                {pictureError}
              </p>
            )}
          </div>
        </div>
        <input
          ref={fileInput}
          type="file"
          accept={ACCEPT}
          className="sr-only"
          tabIndex={-1}
          aria-label={t("fields.picture")}
          data-testid="emoji-file"
          onChange={(event) => {
            void choose(event.target.files?.[0]);
            event.target.value = "";
          }}
        />
        {picture && (
          <ThemePreview className="mt-3">
            <p data-testid="emoji-preview-line">
              {t("form.previewBefore")}
              <img
                src={picture}
                alt={name ? `:${name}:` : ""}
                className="inline-block h-[1.35em] w-auto align-[-0.3em]"
                draggable={false}
              />
              {t("form.previewAfter")}
            </p>
            <p className="mt-1.5 inline-flex h-7 items-center gap-1 rounded-full border-2 border-line px-2 text-xs font-bold">
              <img src={picture} alt="" className="size-4 object-contain" draggable={false} />3
            </p>
          </ThemePreview>
        )}
      </div>

      <TextField
        label={t("fields.name")}
        value={name}
        maxLength={32}
        autoCapitalize="none"
        autoCorrect="off"
        spellCheck={false}
        placeholder="neko_love"
        onChange={(event) =>
          onChange({ name: event.target.value.toLowerCase().replace(/[\s-]+/g, "_") })
        }
        error={nameError}
        hint={name && nameOk ? t("form.nameOk", { code: `:${name}:` }) : t("form.nameHint")}
        trailing={
          checkable && check.isFetching ? (
            <Spinner className="mr-2 size-4 text-ink-faint" />
          ) : nameOk ? (
            <Check className="mr-2 size-5 text-success" aria-label={t("form.nameAvailable")} />
          ) : undefined
        }
      />

      <SelectField
        label={t("fields.category")}
        value={draft.category}
        onChange={(category) => onChange({ category })}
        options={[
          { value: "", label: t("form.noCategory") },
          ...categories.map((category) => ({ value: category, label: category })),
        ]}
      />

      <div className="flex flex-col gap-1.5">
        <label htmlFor={aliasId} className="px-1 text-sm font-bold text-ink-soft">
          {t("fields.aliases")}
        </label>
        <div className="flex min-h-12 flex-wrap items-center gap-1.5 rounded-control border-2 border-line bg-surface px-3 py-2 focus-within:border-brand">
          {draft.aliases.map((alias) => (
            <Chip key={alias} className="h-7 gap-0.5 pr-1">
              {alias}
              <button
                type="button"
                aria-label={t("form.removeAlias", { alias })}
                onClick={() =>
                  onChange({ aliases: draft.aliases.filter((item) => item !== alias) })
                }
                className="grid size-5 place-items-center rounded-full hover:bg-surface-3"
              >
                <X className="size-3" />
              </button>
            </Chip>
          ))}
          <input
            id={aliasId}
            value={aliasText}
            maxLength={64}
            disabled={draft.aliases.length >= maxAliases}
            onChange={(event) => setAliasText(event.target.value)}
            onKeyDown={onAliasKey}
            onBlur={() => addAliases(aliasText)}
            placeholder={draft.aliases.length === 0 ? t("form.aliasPlaceholder") : ""}
            className="h-7 min-w-24 flex-1 bg-transparent text-[15px] outline-none placeholder:text-ink-faint"
          />
        </div>
        <p className="px-1 text-xs text-ink-faint">
          {t("form.aliasHint", { count: draft.aliases.length, max: maxAliases })}
        </p>
      </div>

      <TextArea
        label={t("fields.license")}
        value={draft.license}
        maxLength={LICENSE_MAX}
        placeholder={t("form.licensePlaceholder")}
        onChange={(event) => onChange({ license: event.target.value })}
        hint={t("form.licenseHint")}
      />

      <div className="rounded-2xl bg-surface-2">
        <SwitchRow
          label={t("fields.sensitive")}
          hint={t("form.sensitiveHint")}
          checked={draft.sensitive}
          onChange={(sensitive) => onChange({ sensitive })}
        />
        {staff && (
          <SwitchRow
            label={t("fields.localOnly")}
            hint={t("form.localOnlyHint")}
            checked={draft.localOnly}
            onChange={(localOnly) => onChange({ localOnly })}
          />
        )}
      </div>
    </div>
  );
}
