import { useQuery } from "@tanstack/react-query";
import { community, media } from "@tokuly/api";
import { CATEGORIES, imageUrl, parseTags, VISIBILITY_KEYS } from "@tokuly/core/community";
import { Check, Plus, Trash2 } from "lucide-react";
import { useEffect, useRef, useState } from "react";
import { toast } from "sonner";
import { Button } from "#/components/ui/button";
import { TextArea, TextField } from "#/components/ui/field";
import { SwitchRow } from "#/components/ui/switch";
import { cn } from "#/lib/cn";
import { errorMessage } from "#/lib/errors";
import { uploadAndProcess } from "#/lib/upload";
import { useTranslation } from "#/lib/use-translation";
import { communityClient, NS } from "./api";
import { SectionTitle } from "./parts";

const V = community.CommunityVisibility;
const C = community.CommunityCategory;

/** A picture being chosen: unchanged (`undefined`), replaced, or removed (`null`). */
export type ArtDraft = { assetId: string; preview: string } | null | undefined;

/** Everything the creation form and the settings page edit. */
export interface CommunityDraft {
  name: string;
  handle: string;
  description: string;
  category: community.CommunityCategory;
  visibility: community.CommunityVisibility;
  tags: string;
  schoolName: string;
  rules: { title: string; body: string }[];
  questions: string[];
  icon: ArtDraft;
  banner: ArtDraft;
  membersCanInvite: boolean;
  membersCanCreateEvents: boolean;
}

export function emptyDraft(): CommunityDraft {
  return {
    name: "",
    handle: "",
    description: "",
    category: C.CLUB,
    visibility: V.PUBLIC,
    tags: "",
    schoolName: "",
    rules: [],
    questions: [],
    icon: undefined,
    banner: undefined,
    membersCanInvite: true,
    membersCanCreateEvents: false,
  };
}

export function draftOf(c: community.Community): CommunityDraft {
  return {
    name: c.name,
    handle: c.handle,
    description: c.description,
    category: c.category,
    visibility: c.visibility,
    tags: c.tags.map((tag) => `#${tag}`).join(" "),
    schoolName: c.schoolName,
    rules: c.rules.map((rule) => ({ title: rule.title, body: rule.body })),
    questions: [...c.joinQuestions],
    icon: undefined,
    banner: undefined,
    membersCanInvite: c.settings?.membersCanInvite ?? false,
    membersCanCreateEvents: c.settings?.membersCanCreateEvents ?? false,
  };
}

/** The parts of a draft both CreateCommunity and UpdateCommunity take as they are. */
export function draftFields(draft: CommunityDraft) {
  return {
    name: draft.name.trim(),
    handle: draft.handle.trim().toLowerCase(),
    description: draft.description.trim(),
    category: draft.category,
    visibility: draft.visibility,
    schoolName: usesSchool(draft.category) ? draft.schoolName.trim() : "",
    settings: {
      membersCanInvite: draft.membersCanInvite,
      membersCanCreateEvents: draft.membersCanCreateEvents,
    },
  };
}
export const draftTags = (draft: CommunityDraft) => parseTags(draft.tags, 10);
export const draftRules = (draft: CommunityDraft) =>
  draft.rules
    .map((rule) => ({ title: rule.title.trim(), body: rule.body.trim() }))
    .filter((rule) => rule.title);
export const draftQuestions = (draft: CommunityDraft) =>
  draft.questions.map((question) => question.trim()).filter(Boolean);

const usesSchool = (category: community.CommunityCategory) =>
  category === C.SCHOOL || category === C.CLUB;

/** Whether a handle is free, asked a moment after typing stops. `current` is the handle the community has now. */
export function useHandleCheck(handle: string, current = "") {
  const [settled, setSettled] = useState(handle);
  useEffect(() => {
    const timer = window.setTimeout(() => setSettled(handle), 350);
    return () => window.clearTimeout(timer);
  }, [handle]);
  const normalized = settled.trim().toLowerCase();
  const query = useQuery({
    queryKey: ["community", "handle", normalized],
    queryFn: () => communityClient().checkHandle({ handle: normalized }),
    enabled: normalized.length > 0 && normalized !== current.toLowerCase(),
    staleTime: 10_000,
  });
  const unchanged = normalized === current.toLowerCase();
  return {
    /** The answer is for what is in the field right now. */
    settled: settled === handle && (unchanged || query.isSuccess),
    available: unchanged || (query.data?.available ?? false),
    reason: unchanged ? "" : (query.data?.reason ?? ""),
  };
}

/** The fields of a community: creation and settings share them. */
export function CommunityFields({
  draft,
  onChange,
  current,
}: {
  draft: CommunityDraft;
  onChange(next: CommunityDraft): void;
  /** The community being edited; none while creating. */
  current?: community.Community;
}) {
  const { t } = useTranslation(NS);
  const set = <K extends keyof CommunityDraft>(key: K, value: CommunityDraft[K]) =>
    onChange({ ...draft, [key]: value });
  const check = useHandleCheck(draft.handle, current?.handle);
  const handleError =
    draft.handle && check.settled && !check.available
      ? t(`errors.${check.reason || "HANDLE_INVALID"}`)
      : undefined;

  return (
    <div className="flex flex-col gap-4">
      <TextField
        label={t("form.name")}
        value={draft.name}
        maxLength={50}
        placeholder={t("form.namePlaceholder")}
        onChange={(event) => set("name", event.target.value)}
      />
      <TextField
        label={t("form.handle")}
        value={draft.handle}
        maxLength={24}
        autoCapitalize="off"
        autoComplete="off"
        spellCheck={false}
        placeholder="tennis_bu"
        hint={
          draft.handle && check.settled && check.available && draft.handle !== current?.handle
            ? t("form.handleFree")
            : t("form.handleHint")
        }
        error={handleError}
        trailing={
          draft.handle && check.settled && check.available ? (
            <Check className="mr-2.5 size-5 text-success" aria-hidden />
          ) : undefined
        }
        onChange={(event) => set("handle", event.target.value.replace(/\s/g, ""))}
      />
      <TextArea
        label={t("form.description")}
        value={draft.description}
        maxLength={1000}
        placeholder={t("form.descriptionPlaceholder")}
        onChange={(event) => set("description", event.target.value)}
      />

      <fieldset>
        <legend className="px-1 pb-1.5 text-sm font-bold text-ink-soft">
          {t("form.category")}
        </legend>
        <div className="grid grid-cols-2 gap-2 sm:grid-cols-4">
          {CATEGORIES.map((category) => (
            <button
              key={category.value}
              type="button"
              aria-pressed={draft.category === category.value}
              onClick={() => set("category", category.value)}
              className={cn(
                "flex h-12 items-center gap-2 rounded-2xl border-2 px-3 text-sm font-bold transition-colors",
                draft.category === category.value
                  ? "border-ink bg-brand-soft"
                  : "border-line hover:border-ink-faint",
              )}
            >
              <span aria-hidden className="text-lg">
                {category.emoji}
              </span>
              {t(`category.${category.key}`)}
            </button>
          ))}
        </div>
      </fieldset>
      {usesSchool(draft.category) && (
        <TextField
          label={t("form.school")}
          hint={t("form.schoolHint")}
          value={draft.schoolName}
          maxLength={100}
          onChange={(event) => set("schoolName", event.target.value)}
        />
      )}

      <fieldset>
        <legend className="px-1 pb-1.5 text-sm font-bold text-ink-soft">
          {t("form.visibility")}
        </legend>
        <div className="flex flex-col gap-2">
          {[V.PUBLIC, V.RESTRICTED, V.HIDDEN].map((value) => (
            <button
              key={value}
              type="button"
              aria-pressed={draft.visibility === value}
              onClick={() => set("visibility", value)}
              className={cn(
                "rounded-2xl border-2 px-4 py-3 text-left transition-colors",
                draft.visibility === value
                  ? "border-ink bg-brand-soft"
                  : "border-line hover:border-ink-faint",
              )}
            >
              <span className="block font-bold">{t(`visibility.${VISIBILITY_KEYS[value]}`)}</span>
              <span className="block text-sm text-ink-soft">
                {t(`visibility.${VISIBILITY_KEYS[value]}Hint`)}
              </span>
            </button>
          ))}
        </div>
      </fieldset>

      <TextField
        label={t("form.tags")}
        hint={t("form.tagsHint")}
        value={draft.tags}
        placeholder="#テニス #初心者歓迎"
        onChange={(event) => set("tags", event.target.value)}
      />

      <div className="flex flex-col gap-4 sm:flex-row">
        <ArtField
          label={t("form.icon")}
          shape="icon"
          purpose={media.AssetPurpose.COMMUNITY_ICON}
          draft={draft.icon}
          currentUrl={imageUrl(current?.icon, 96)}
          onChange={(icon) => set("icon", icon)}
        />
        <ArtField
          label={t("form.banner")}
          shape="banner"
          purpose={media.AssetPurpose.COMMUNITY_BANNER}
          draft={draft.banner}
          currentUrl={imageUrl(current?.banner, 320)}
          onChange={(banner) => set("banner", banner)}
        />
      </div>

      <SectionTitle>{t("form.rules")}</SectionTitle>
      <p className="-mt-2 px-1 text-sm text-ink-soft">{t("form.rulesHint")}</p>
      {draft.rules.map((rule, index) => (
        <div key={index} className="flex gap-2 rounded-2xl border-2 border-line p-3">
          <div className="flex min-w-0 flex-1 flex-col gap-2">
            <TextField
              aria-label={t("form.ruleTitle", { index: index + 1 })}
              placeholder={t("form.ruleTitle", { index: index + 1 })}
              value={rule.title}
              maxLength={80}
              onChange={(event) =>
                set(
                  "rules",
                  draft.rules.map((each, at) =>
                    at === index ? { ...each, title: event.target.value } : each,
                  ),
                )
              }
            />
            <TextArea
              aria-label={t("form.ruleBody", { index: index + 1 })}
              placeholder={t("form.ruleBodyPlaceholder")}
              value={rule.body}
              maxLength={500}
              onChange={(event) =>
                set(
                  "rules",
                  draft.rules.map((each, at) =>
                    at === index ? { ...each, body: event.target.value } : each,
                  ),
                )
              }
            />
          </div>
          <Button
            variant="ghost"
            size="icon-sm"
            aria-label={t("form.removeRule", { index: index + 1 })}
            onClick={() =>
              set(
                "rules",
                draft.rules.filter((_, at) => at !== index),
              )
            }
          >
            <Trash2 className="size-4" />
          </Button>
        </div>
      ))}
      {draft.rules.length < 20 && (
        <Button
          variant="secondary"
          size="sm"
          className="self-start"
          icon={<Plus className="size-4" />}
          onClick={() => set("rules", [...draft.rules, { title: "", body: "" }])}
        >
          {t("form.addRule")}
        </Button>
      )}

      {draft.visibility === V.RESTRICTED && (
        <>
          <SectionTitle>{t("form.questions")}</SectionTitle>
          <p className="-mt-2 px-1 text-sm text-ink-soft">{t("form.questionsHint")}</p>
          {draft.questions.map((question, index) => (
            <div key={index} className="flex items-center gap-2">
              <TextField
                className="min-w-0 flex-1"
                aria-label={t("form.question", { index: index + 1 })}
                placeholder={t("form.questionPlaceholder")}
                value={question}
                maxLength={200}
                onChange={(event) =>
                  set(
                    "questions",
                    draft.questions.map((each, at) => (at === index ? event.target.value : each)),
                  )
                }
              />
              <Button
                variant="ghost"
                size="icon-sm"
                aria-label={t("form.removeQuestion", { index: index + 1 })}
                onClick={() =>
                  set(
                    "questions",
                    draft.questions.filter((_, at) => at !== index),
                  )
                }
              >
                <Trash2 className="size-4" />
              </Button>
            </div>
          ))}
          {draft.questions.length < 5 && (
            <Button
              variant="secondary"
              size="sm"
              className="self-start"
              icon={<Plus className="size-4" />}
              onClick={() => set("questions", [...draft.questions, ""])}
            >
              {t("form.addQuestion")}
            </Button>
          )}
        </>
      )}

      <SectionTitle>{t("form.permissions")}</SectionTitle>
      <div className="rounded-card bg-surface p-1.5">
        <SwitchRow
          label={t("form.membersCanInvite")}
          hint={t("form.membersCanInviteHint")}
          checked={draft.membersCanInvite}
          onChange={(value) => set("membersCanInvite", value)}
        />
        <SwitchRow
          label={t("form.membersCanCreateEvents")}
          hint={t("form.membersCanCreateEventsHint")}
          checked={draft.membersCanCreateEvents}
          onChange={(value) => set("membersCanCreateEvents", value)}
        />
      </div>
    </div>
  );
}

/** Choosing an icon or a banner: a preview, "choose" and "remove". */
function ArtField({
  label,
  shape,
  purpose,
  draft,
  currentUrl,
  onChange,
}: {
  label: string;
  shape: "icon" | "banner";
  purpose: media.AssetPurpose;
  draft: ArtDraft;
  currentUrl?: string;
  onChange(next: ArtDraft): void;
}) {
  const { t } = useTranslation(NS);
  const input = useRef<HTMLInputElement>(null);
  const [uploading, setUploading] = useState(false);
  const url = draft === null ? undefined : (draft?.preview ?? currentUrl);
  const pick = async (file: File) => {
    setUploading(true);
    try {
      const asset = await uploadAndProcess(file, { purpose });
      onChange({ assetId: asset.id, preview: URL.createObjectURL(file) });
    } catch (error) {
      toast.error(errorMessage(error, { namespace: NS }));
    } finally {
      setUploading(false);
    }
  };
  return (
    <div className={cn("flex flex-col gap-1.5", shape === "banner" && "min-w-0 flex-1")}>
      <p className="px-1 text-sm font-bold text-ink-soft">{label}</p>
      <div
        className={cn(
          "grid place-items-center overflow-hidden border-2 border-line bg-surface-2 text-xs text-ink-faint",
          shape === "icon" ? "size-24 rounded-[1.4rem]" : "h-24 w-full rounded-2xl",
        )}
      >
        {url ? <img src={url} alt="" className="size-full object-cover" /> : t("art.none")}
      </div>
      <input
        ref={input}
        type="file"
        accept="image/*"
        hidden
        aria-label={label}
        onChange={(event) => {
          const file = event.target.files?.[0];
          if (file) void pick(file);
          event.target.value = "";
        }}
      />
      <div className="flex gap-1.5">
        <Button
          variant="secondary"
          size="sm"
          loading={uploading}
          onClick={() => input.current?.click()}
        >
          {url ? t("art.change") : t("art.choose")}
        </Button>
        {url && (
          <Button variant="ghost" size="sm" onClick={() => onChange(null)}>
            {t("art.remove")}
          </Button>
        )}
      </div>
    </div>
  );
}
