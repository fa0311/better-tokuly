import { useMutation, useQuery } from "@tanstack/react-query";
import { common, create, identity, media, social } from "@tokuly/api";
import {
  ArrowDown,
  ArrowUp,
  BadgeCheck,
  EyeOff,
  ImagePlus,
  Music,
  Plus,
  RefreshCw,
  Trash2,
} from "lucide-react";
import { type ReactNode, useState } from "react";
import { toast } from "sonner";
import { Button } from "#/components/ui/button";
import { Chip } from "#/components/ui/card";
import { TextArea, TextField } from "#/components/ui/field";
import { SelectField } from "#/components/ui/select";
import { SwitchRow } from "#/components/ui/switch";
import { accountClient, api, profileClient } from "#/lib/api";
import { errorMessage } from "#/lib/errors";
import { uploadAndProcess } from "#/lib/upload";
import { useTranslation } from "#/lib/use-translation";
import { useMe } from "#/stores/session";
import { adoptMe } from "../data";
import { Group, SettingsPage } from "../shell/settings-page";

type VisibilityName = "PUBLIC" | "FOLLOWERS" | "FRIENDS" | "CLOSE_FRIENDS" | "PRIVATE";
/** The audiences of the social graph, widest first (a locked account's "public" is its followers). */
const AUDIENCES: VisibilityName[] = ["PUBLIC", "FOLLOWERS", "FRIENDS", "CLOSE_FRIENDS", "PRIVATE"];
type Field = keyof Omit<identity.ProfileFieldVisibility, "$typeName" | "$unknown">;

const BLOOD_TYPES = ["UNSPECIFIED", "A", "B", "O", "AB"] as const;
const STYLES = ["UNSPECIFIED", "POP", "PASTEL", "NEON", "MONO", "NIGHT"] as const;
const CATEGORIES = [
  "MUSIC",
  "GAMES",
  "FOOD",
  "ANIME_MANGA",
  "SPORTS",
  "MOVIES_TV",
  "BOOKS",
  "FASHION",
  "OTHER",
] as const;
type Category = (typeof CATEGORIES)[number];

/** The name of a stored audience; anything not offered here reads as the narrowest. */
const visibilityName = (value: common.Visibility): VisibilityName => {
  const name = common.Visibility[value] as VisibilityName | undefined;
  return name && AUDIENCES.includes(name) ? name : "PRIVATE";
};

/** A card's audience as one select value: an audience name, or `circle:<id>`. */
const cardAudienceValue = (audience: common.AudienceRef | undefined): string =>
  audience?.visibility === common.Visibility.CIRCLE && audience.circleId
    ? `circle:${audience.circleId}`
    : visibilityName(audience?.visibility ?? common.Visibility.PUBLIC);

const cardAudienceRef = (value: string): common.AudienceRef =>
  value.startsWith("circle:")
    ? create(common.AudienceRefSchema, {
        visibility: common.Visibility.CIRCLE,
        circleId: value.slice(7),
      })
    : create(common.AudienceRefSchema, { visibility: common.Visibility[value as VisibilityName] });

const ns = { namespace: "profileBook" } as const;

/** Saves one or more groups of the プロフ帳 and puts the result into the session. */
function useSave() {
  const { t } = useTranslation("profileBook");
  const me = useMe();
  return useMutation({
    mutationFn: (patch: Partial<identity.UpdateProfileExtrasRequest>) =>
      accountClient().updateProfileExtras(create(identity.UpdateProfileExtrasRequestSchema, patch)),
    onSuccess: (response) => {
      if (me) adoptMe({ ...me, extras: response.extras });
      toast.success(t("saved"));
    },
    onError: (error) => toast.error(errorMessage(error, ns)),
  });
}

/** The audience picker shown beside a field. */
function AudienceSelect({
  value,
  onChange,
  label,
}: {
  value: common.Visibility;
  onChange(next: common.Visibility): void;
  label: string;
}) {
  const { t } = useTranslation("profileBook");
  return (
    <SelectField<VisibilityName>
      label={label}
      value={visibilityName(value)}
      onChange={(next) => onChange(common.Visibility[next])}
      options={AUDIENCES.map((name) => ({ value: name, label: t(`audience.${name}`) }))}
    />
  );
}

/**
 * The プロフ帳 editor: MBTI, blood type, pronouns, the long bio, theme,
 * 推し, favourite things, links, the pinned song and question cards, each
 * with who sees it.
 */
export function ProfileBookSection() {
  const { t } = useTranslation("profileBook");
  const me = useMe();
  const extras = me?.extras ?? create(identity.ProfileExtrasSchema);
  if (!me) return null;
  return (
    <SettingsPage title={t("title")}>
      <p className="mb-6 px-1 text-sm leading-relaxed text-ink-soft">{t("lead")}</p>
      <AboutGroup key={`about-${extras.mbti}-${extras.pronouns}`} extras={extras} />
      <CardsGroup extras={extras} />
      <OshiGroup extras={extras} />
      <TagsGroup extras={extras} />
      <LinksGroup extras={extras} />
      <SongGroup extras={extras} />
    </SettingsPage>
  );
}

function SaveRow({ onSave, busy }: { onSave(): void; busy: boolean }) {
  const { t } = useTranslation("profileBook");
  return (
    <div className="mt-4 flex justify-end">
      <Button onClick={onSave} loading={busy}>
        {t("save")}
      </Button>
    </div>
  );
}

function AboutGroup({ extras }: { extras: identity.ProfileExtras }) {
  const { t } = useTranslation("profileBook");
  const save = useSave();
  const [mbti, setMbti] = useState(extras.mbti);
  const [blood, setBlood] = useState(identity.BloodType[extras.bloodType] ?? "UNSPECIFIED");
  const [pronouns, setPronouns] = useState(extras.pronouns);
  const [longBio, setLongBio] = useState(extras.longBio);
  const [color, setColor] = useState(extras.theme?.color ?? "");
  const [style, setStyle] = useState(
    identity.ProfileThemeStyle[extras.theme?.style ?? 0] ?? "UNSPECIFIED",
  );
  const [visibility, setVisibility] = useState<Audiences>(audiencesOf(extras.visibility));
  const audience = (field: Field) => (
    <AudienceSelect
      label={t("whoSees")}
      value={visibility[field]}
      onChange={(next) => setVisibility({ ...visibility, [field]: next })}
    />
  );
  const row = (field: Field, control: ReactNode) => (
    <div className="grid gap-2 sm:grid-cols-[1fr_11rem] sm:items-end">
      {control}
      {audience(field)}
    </div>
  );
  return (
    <Group title={t("about.title")} lead={t("about.lead")}>
      <div className="flex flex-col gap-4">
        {row(
          "mbti",
          <TextField
            label={t("about.mbti")}
            hint={t("about.mbtiHint")}
            value={mbti}
            onChange={(e) => setMbti(e.target.value)}
            maxLength={6}
            autoCapitalize="characters"
          />,
        )}
        {row(
          "bloodType",
          <SelectField<string>
            label={t("about.bloodType")}
            value={blood}
            onChange={setBlood}
            options={BLOOD_TYPES.map((b) => ({ value: b, label: t(`blood.${b}`) }))}
          />,
        )}
        {row(
          "pronouns",
          <TextField
            label={t("about.pronouns")}
            hint={t("about.pronounsHint")}
            value={pronouns}
            onChange={(e) => setPronouns(e.target.value)}
            maxLength={24}
          />,
        )}
        {row(
          "zodiac",
          <p className="px-1 pb-3 text-sm font-bold">
            {t("about.zodiac")}{" "}
            <span className="text-ink-soft">
              {extras.zodiac
                ? t(`zodiac.${identity.Zodiac[extras.zodiac]}`)
                : t("about.zodiacNone")}
            </span>
          </p>,
        )}
        {row(
          "longBio",
          <TextArea
            label={t("about.longBio")}
            hint={t("about.longBioHint")}
            value={longBio}
            onChange={(e) => setLongBio(e.target.value)}
            rows={5}
            maxLength={2000}
          />,
        )}
        <div className="grid gap-2 sm:grid-cols-2">
          <TextField
            label={t("about.color")}
            type="color"
            value={color || "#7c5cff"}
            onChange={(e) => setColor(e.target.value)}
          />
          <SelectField<string>
            label={t("about.style")}
            value={style}
            onChange={setStyle}
            options={STYLES.map((s) => ({ value: s, label: t(`style.${s}`) }))}
          />
        </div>
        <p className="px-1 text-xs text-ink-faint">{t("about.audienceNote")}</p>
      </div>
      <SaveRow
        busy={save.isPending}
        onSave={() =>
          save.mutate({
            mbti,
            bloodType: identity.BloodType[blood as keyof typeof identity.BloodType],
            pronouns,
            longBio,
            theme: create(identity.ProfileThemeSchema, {
              color,
              style: identity.ProfileThemeStyle[style as keyof typeof identity.ProfileThemeStyle],
            }),
            visibility: create(identity.ProfileFieldVisibilitySchema, visibility),
          })
        }
      />
    </Group>
  );
}

type Audiences = Record<Field, common.Visibility>;

/** The audiences of a stored visibility (or the defaults), as a plain object. */
function audiencesOf(v: identity.ProfileFieldVisibility | undefined): Audiences {
  const d = everyonePublic();
  if (!v) return d;
  const pick = (field: Field) => v[field] || d[field];
  return {
    mbti: pick("mbti"),
    bloodType: pick("bloodType"),
    pronouns: pick("pronouns"),
    zodiac: pick("zodiac"),
    oshi: pick("oshi"),
    tags: pick("tags"),
    links: pick("links"),
    longBio: pick("longBio"),
    pinnedSong: pick("pinnedSong"),
  };
}

function everyonePublic(): Audiences {
  const p = common.Visibility.PUBLIC;
  return {
    mbti: p,
    bloodType: p,
    pronouns: p,
    zodiac: common.Visibility.PRIVATE,
    oshi: p,
    tags: p,
    links: p,
    longBio: p,
    pinnedSong: p,
  };
}

/** Saves a list group together with its audience. */
function useListAudience(extras: identity.ProfileExtras, field: Field) {
  const [audience, setAudience] = useState(extras.visibility?.[field] ?? common.Visibility.PUBLIC);
  const visibility = () =>
    create(identity.ProfileFieldVisibilitySchema, {
      ...audiencesOf(extras.visibility),
      [field]: audience,
    });
  return { audience, setAudience, visibility };
}

interface CardDraft {
  id: string;
  promptId: string;
  question: string;
  answer: string;
  /** An audience name or `circle:<id>`. */
  audience: string;
  hidden: boolean;
}

function CardsGroup({ extras }: { extras: identity.ProfileExtras }) {
  const { t, i18n } = useTranslation("profileBook");
  const save = useSave();
  const { data: prompts } = useQuery({
    queryKey: ["identity", "profile-prompts"],
    queryFn: () => profileClient().listProfilePrompts({}),
    staleTime: Number.POSITIVE_INFINITY,
  });
  const [cards, setCards] = useState<CardDraft[]>(
    extras.qaCards.map((c) => ({
      id: c.id,
      promptId: c.promptId,
      question: c.promptId ? "" : (c.question?.ja ?? ""),
      answer: c.answer,
      audience: cardAudienceValue(c.audience),
      hidden: c.hidden,
    })),
  );
  const [picking, setPicking] = useState("");
  // The owner's own circles, for cards only some of them see (shared with the friends screens).
  const { data: circles } = useQuery({
    queryKey: ["social", "circles", "list"],
    queryFn: () => api().service(social.CircleService).listCircles({}),
  });
  const promptText = (id: string) => {
    const prompt = prompts?.prompts.find((p) => p.id === id);
    if (!prompt) return id;
    return `${prompt.emoji} ${i18n.language.startsWith("en") ? prompt.question?.en : prompt.question?.ja}`;
  };
  const update = (index: number, patch: Partial<CardDraft>) =>
    setCards(cards.map((card, i) => (i === index ? { ...card, ...patch } : card)));
  const move = (index: number, by: number) => {
    const next = [...cards];
    const [card] = next.splice(index, 1);
    if (card) next.splice(index + by, 0, card);
    setCards(next);
  };
  const used = new Set(cards.map((c) => c.promptId));
  const available = (prompts?.prompts ?? []).filter((p) => !used.has(p.id));

  return (
    <Group title={t("cards.title")} lead={t("cards.lead")}>
      <ul className="flex flex-col gap-4">
        {cards.map((card, index) => (
          <li
            key={card.id || `new-${index}`}
            className="rounded-2xl border-2 border-line p-3"
            data-testid="card-editor"
          >
            <div className="flex items-center gap-2">
              <p className="min-w-0 flex-1 truncate font-black">
                {card.promptId ? promptText(card.promptId) : t("cards.free")}
              </p>
              <Button
                size="icon-sm"
                variant="ghost"
                aria-label={t("cards.up")}
                disabled={index === 0}
                onClick={() => move(index, -1)}
              >
                <ArrowUp className="size-4" />
              </Button>
              <Button
                size="icon-sm"
                variant="ghost"
                aria-label={t("cards.down")}
                disabled={index === cards.length - 1}
                onClick={() => move(index, 1)}
              >
                <ArrowDown className="size-4" />
              </Button>
              <Button
                size="icon-sm"
                variant="ghost"
                aria-label={t("cards.delete")}
                onClick={() => setCards(cards.filter((_, i) => i !== index))}
              >
                <Trash2 className="size-4" />
              </Button>
            </div>
            {!card.promptId && (
              <TextField
                className="mt-2"
                label={t("cards.question")}
                value={card.question}
                maxLength={60}
                onChange={(e) => update(index, { question: e.target.value })}
              />
            )}
            <TextArea
              className="mt-2"
              label={t("cards.answer")}
              value={card.answer}
              rows={2}
              maxLength={200}
              onChange={(e) => update(index, { answer: e.target.value })}
            />
            <div className="mt-2 grid gap-2 sm:grid-cols-2 sm:items-center">
              <SelectField<string>
                label={t("whoSees")}
                value={card.audience}
                onChange={(audience) => update(index, { audience })}
                options={[
                  ...AUDIENCES.map((name) => ({ value: name, label: t(`audience.${name}`) })),
                  ...(circles?.circles ?? [])
                    .filter((circle) => circle.kind !== social.CircleKind.CLOSE_FRIENDS)
                    .map((circle) => ({
                      value: `circle:${circle.id}`,
                      label: t("audience.circle", {
                        name: `${circle.emoji} ${circle.name}`.trim(),
                      }),
                    })),
                ]}
              />
              <SwitchRow
                label={t("cards.hidden")}
                checked={card.hidden}
                onChange={(hidden) => update(index, { hidden })}
              />
            </div>
          </li>
        ))}
      </ul>
      <div className="mt-4 flex flex-wrap items-end gap-2">
        <SelectField<string>
          className="min-w-56 flex-1"
          label={t("cards.pick")}
          value={picking}
          onChange={setPicking}
          options={[
            { value: "", label: t("cards.freeOption") },
            ...available.map((p) => ({ value: p.id, label: promptText(p.id) })),
          ]}
        />
        <Button
          variant="secondary"
          icon={<Plus className="size-4" />}
          disabled={cards.length >= 30}
          onClick={() => {
            setCards([
              ...cards,
              {
                id: "",
                promptId: picking,
                question: "",
                answer: "",
                audience: "PUBLIC",
                hidden: false,
              },
            ]);
            setPicking("");
          }}
        >
          {t("cards.add")}
        </Button>
      </div>
      <SaveRow
        busy={save.isPending}
        onSave={() =>
          save.mutate(
            {
              qaCards: create(identity.ProfileQaCardListSchema, {
                items: cards.map((c) =>
                  create(identity.ProfileQaCardInputSchema, {
                    id: c.id,
                    promptId: c.promptId,
                    question: c.question,
                    answer: c.answer,
                    hidden: c.hidden,
                    audience: cardAudienceRef(c.audience),
                  }),
                ),
              }),
            },
            {
              onSuccess: (response) =>
                setCards(
                  (response.extras?.qaCards ?? []).map((c) => ({
                    id: c.id,
                    promptId: c.promptId,
                    question: c.promptId ? "" : (c.question?.ja ?? ""),
                    answer: c.answer,
                    audience: cardAudienceValue(c.audience),
                    hidden: c.hidden,
                  })),
                ),
            },
          )
        }
      />
    </Group>
  );
}

interface OshiDraft {
  id: string;
  name: string;
  url: string;
  emoji: string;
  imageUrl?: string;
  /** Set when a new picture was chosen ("" removes it). */
  imageAssetId?: string;
}

function OshiGroup({ extras }: { extras: identity.ProfileExtras }) {
  const { t } = useTranslation("profileBook");
  const save = useSave();
  const list = useListAudience(extras, "oshi");
  const [oshi, setOshi] = useState<OshiDraft[]>(
    extras.oshi.map((o) => ({
      id: o.id,
      name: o.name,
      url: o.url,
      emoji: o.emoji,
      imageUrl: o.image?.url,
    })),
  );
  const [uploading, setUploading] = useState<number | null>(null);
  const update = (index: number, patch: Partial<OshiDraft>) =>
    setOshi(oshi.map((o, i) => (i === index ? { ...o, ...patch } : o)));
  const pick = async (index: number, file: File | undefined) => {
    if (!file) return;
    setUploading(index);
    try {
      const asset = await uploadAndProcess(file, { purpose: media.AssetPurpose.AVATAR });
      update(index, { imageAssetId: asset.id, imageUrl: asset.url });
    } catch (error) {
      toast.error(errorMessage(error, ns));
    } finally {
      setUploading(null);
    }
  };
  return (
    <Group title={t("oshi.title")} lead={t("oshi.lead")}>
      <ul className="flex flex-col gap-3">
        {oshi.map((o, index) => (
          <li
            key={o.id || `new-${index}`}
            className="flex flex-wrap items-end gap-2 rounded-2xl border-2 border-line p-3"
          >
            <label className="pressable grid size-14 shrink-0 cursor-pointer place-items-center overflow-hidden rounded-xl border-2 border-ink bg-surface-2">
              {o.imageUrl ? (
                <img src={o.imageUrl} alt="" className="size-full object-cover" />
              ) : uploading === index ? (
                <RefreshCw className="size-5 animate-spin" />
              ) : (
                <ImagePlus className="size-5" aria-label={t("oshi.image")} />
              )}
              <input
                type="file"
                accept="image/*"
                className="sr-only"
                onChange={(e) => void pick(index, e.target.files?.[0])}
              />
            </label>
            <TextField
              className="min-w-40 flex-1"
              label={t("oshi.name")}
              value={o.name}
              maxLength={40}
              onChange={(e) => update(index, { name: e.target.value })}
            />
            <TextField
              className="w-20"
              label={t("oshi.emoji")}
              value={o.emoji}
              maxLength={8}
              onChange={(e) => update(index, { emoji: e.target.value })}
            />
            <TextField
              className="min-w-48 flex-1"
              label={t("oshi.url")}
              value={o.url}
              type="url"
              placeholder="https://"
              onChange={(e) => update(index, { url: e.target.value })}
            />
            <Button
              size="icon-sm"
              variant="ghost"
              aria-label={t("oshi.delete")}
              onClick={() => setOshi(oshi.filter((_, i) => i !== index))}
            >
              <Trash2 className="size-4" />
            </Button>
          </li>
        ))}
      </ul>
      <div className="mt-4 flex flex-wrap items-end gap-2">
        <Button
          variant="secondary"
          icon={<Plus className="size-4" />}
          disabled={oshi.length >= 10}
          onClick={() => setOshi([...oshi, { id: "", name: "", url: "", emoji: "" }])}
        >
          {t("oshi.add")}
        </Button>
        <div className="ml-auto w-44">
          <AudienceSelect label={t("whoSees")} value={list.audience} onChange={list.setAudience} />
        </div>
      </div>
      <SaveRow
        busy={save.isPending}
        onSave={() =>
          save.mutate({
            visibility: list.visibility(),
            oshi: create(identity.OshiListSchema, {
              items: oshi.map((o) =>
                create(identity.OshiInputSchema, {
                  id: o.id,
                  name: o.name,
                  url: o.url,
                  emoji: o.emoji,
                  imageAssetId: o.imageAssetId,
                }),
              ),
            }),
          })
        }
      />
    </Group>
  );
}

function TagsGroup({ extras }: { extras: identity.ProfileExtras }) {
  const { t } = useTranslation("profileBook");
  const save = useSave();
  const list = useListAudience(extras, "tags");
  const [tags, setTags] = useState(
    extras.tags.map((tag) => ({ category: tag.category, name: tag.name })),
  );
  const [category, setCategory] = useState<Category>("MUSIC");
  const [name, setName] = useState("");
  const categoryValue = identity.ProfileTagCategory[category];
  const { data: suggestions } = useQuery({
    queryKey: ["identity", "tag-suggestions", category, name.trim()],
    queryFn: () =>
      profileClient().suggestProfileTags({ category: categoryValue, query: name.trim() }),
    staleTime: 60_000,
  });
  const add = (value: string) => {
    const trimmed = value.trim();
    if (!trimmed || tags.some((tag) => tag.category === categoryValue && tag.name === trimmed))
      return;
    setTags([...tags, { category: categoryValue, name: trimmed }]);
    setName("");
  };
  return (
    <Group title={t("tags.title")} lead={t("tags.lead")}>
      <ul className="flex flex-wrap gap-2" aria-label={t("tags.title")}>
        {tags.map((tag, index) => (
          <li key={`${tag.category}-${tag.name}`}>
            <Chip className="gap-1">
              <span className="text-ink-soft">
                {t(`category.${identity.ProfileTagCategory[tag.category]}`)}
              </span>
              {tag.name}
              <button
                type="button"
                aria-label={t("tags.remove", { name: tag.name })}
                onClick={() => setTags(tags.filter((_, i) => i !== index))}
                className="ml-1 text-ink-faint hover:text-danger"
              >
                ×
              </button>
            </Chip>
          </li>
        ))}
      </ul>
      <form
        className="mt-4 flex flex-wrap items-end gap-2"
        onSubmit={(e) => {
          e.preventDefault();
          add(name);
        }}
      >
        <SelectField<Category>
          className="w-40"
          label={t("tags.category")}
          value={category}
          onChange={setCategory}
          options={CATEGORIES.map((c) => ({ value: c, label: t(`category.${c}`) }))}
        />
        <TextField
          className="min-w-40 flex-1"
          label={t("tags.name")}
          value={name}
          maxLength={24}
          onChange={(e) => setName(e.target.value)}
        />
        <Button type="submit" variant="secondary" disabled={tags.length >= 30}>
          {t("tags.add")}
        </Button>
      </form>
      {(suggestions?.suggestions.length ?? 0) > 0 && (
        <div className="mt-3">
          <p className="px-1 text-xs font-bold text-ink-faint">{t("tags.suggestions")}</p>
          <div className="mt-1 flex flex-wrap gap-1.5">
            {suggestions?.suggestions.slice(0, 10).map((s) => (
              <button
                key={s.name}
                type="button"
                onClick={() => add(s.name)}
                className="pressable rounded-full border-2 border-line px-3 py-1 text-sm font-bold"
              >
                {s.name} <span className="text-ink-faint">{s.count}</span>
              </button>
            ))}
          </div>
        </div>
      )}
      <div className="mt-4 flex items-end justify-between gap-2">
        <div className="w-44">
          <AudienceSelect label={t("whoSees")} value={list.audience} onChange={list.setAudience} />
        </div>
        <Button
          loading={save.isPending}
          onClick={() =>
            save.mutate({
              visibility: list.visibility(),
              tags: create(identity.ProfileTagListSchema, {
                items: tags.map((tag) => create(identity.ProfileTagSchema, tag)),
              }),
            })
          }
        >
          {t("save")}
        </Button>
      </div>
    </Group>
  );
}

function LinksGroup({ extras }: { extras: identity.ProfileExtras }) {
  const { t } = useTranslation("profileBook");
  const save = useSave();
  const list = useListAudience(extras, "links");
  const me = useMe();
  const [links, setLinks] = useState(extras.links.map((l) => ({ title: l.title, url: l.url })));
  const recheck = useMutation({
    mutationFn: () => accountClient().recheckProfileLinks({}),
    onSuccess: () => toast.success(t("links.rechecking")),
    onError: (error) => toast.error(errorMessage(error, ns)),
  });
  const verified = (url: string) => extras.links.find((l) => l.url === url)?.verified ?? false;
  const profileUrl = `${window.location.origin}/u/${me?.user?.handle ?? ""}`;
  return (
    <Group title={t("links.title")} lead={t("links.lead", { url: profileUrl })}>
      <ul className="flex flex-col gap-3">
        {links.map((link, index) => (
          <li key={`link-${index}`} className="flex flex-wrap items-end gap-2">
            <TextField
              className="w-40"
              label={t("links.linkTitle")}
              value={link.title}
              maxLength={40}
              onChange={(e) =>
                setLinks(links.map((l, i) => (i === index ? { ...l, title: e.target.value } : l)))
              }
            />
            <TextField
              className="min-w-48 flex-1"
              label={t("links.url")}
              type="url"
              placeholder="https://"
              value={link.url}
              onChange={(e) =>
                setLinks(links.map((l, i) => (i === index ? { ...l, url: e.target.value } : l)))
              }
            />
            {verified(link.url) && (
              <BadgeCheck className="mb-3 size-5 text-success" aria-label={t("links.verified")} />
            )}
            <Button
              size="icon-sm"
              variant="ghost"
              aria-label={t("links.delete")}
              onClick={() => setLinks(links.filter((_, i) => i !== index))}
            >
              <Trash2 className="size-4" />
            </Button>
          </li>
        ))}
      </ul>
      <div className="mt-4 flex flex-wrap items-end gap-2">
        <Button
          variant="secondary"
          icon={<Plus className="size-4" />}
          disabled={links.length >= 8}
          onClick={() => setLinks([...links, { title: "", url: "" }])}
        >
          {t("links.add")}
        </Button>
        <Button
          variant="ghost"
          icon={<RefreshCw className="size-4" />}
          loading={recheck.isPending}
          onClick={() => recheck.mutate()}
        >
          {t("links.recheck")}
        </Button>
        <div className="ml-auto w-44">
          <AudienceSelect label={t("whoSees")} value={list.audience} onChange={list.setAudience} />
        </div>
      </div>
      <SaveRow
        busy={save.isPending}
        onSave={() =>
          save.mutate({
            visibility: list.visibility(),
            links: create(identity.ProfileLinkListSchema, {
              items: links
                .filter((l) => l.url.trim())
                .map((l) => create(identity.ProfileLinkInputSchema, l)),
            }),
          })
        }
      />
    </Group>
  );
}

function SongGroup({ extras }: { extras: identity.ProfileExtras }) {
  const { t } = useTranslation("profileBook");
  const me = useMe();
  const [track, setTrack] = useState("");
  const pin = useMutation({
    mutationFn: (request: { track?: string; clear?: boolean }) => accountClient().pinSong(request),
    onSuccess: async () => {
      setTrack("");
      const fresh = await accountClient().getMe({});
      if (fresh.me) adoptMe(fresh.me);
      toast.success(t("saved"));
    },
    onError: (error) => toast.error(errorMessage(error, ns)),
  });
  const song = extras.pinnedSong;
  return (
    <Group title={t("song.title")} lead={t("song.lead")}>
      {song?.title ? (
        <div className="flex items-center gap-3">
          {song.artworkUrl ? (
            <img src={song.artworkUrl} alt="" className="size-14 rounded-xl object-cover" />
          ) : (
            <span className="grid size-14 place-items-center rounded-xl bg-mint text-[#17122b]">
              <Music className="size-6" />
            </span>
          )}
          <span className="min-w-0 flex-1">
            <span className="block truncate font-black">{song.title}</span>
            <span className="block truncate text-sm text-ink-soft">{song.artist}</span>
          </span>
          <Button
            variant="ghost"
            icon={<EyeOff className="size-4" />}
            onClick={() => pin.mutate({ clear: true })}
          >
            {t("song.clear")}
          </Button>
        </div>
      ) : (
        <p className="text-sm text-ink-soft">{t("song.none")}</p>
      )}
      <form
        className="mt-4 flex flex-wrap items-end gap-2"
        onSubmit={(e) => {
          e.preventDefault();
          pin.mutate({ track });
        }}
      >
        <TextField
          className="min-w-56 flex-1"
          label={t("song.track")}
          placeholder="https://open.spotify.com/track/…"
          value={track}
          onChange={(e) => setTrack(e.target.value)}
        />
        <Button type="submit" loading={pin.isPending} disabled={!me}>
          {track.trim() ? t("song.pin") : t("song.pinNow")}
        </Button>
      </form>
    </Group>
  );
}
