import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useNavigate } from "@tanstack/react-router";
import { common, geo, media } from "@tokuly/api";
import { Crosshair, ImagePlus, LocateFixed, SmilePlus, Trash2 } from "lucide-react";
import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { Button } from "#/components/ui/button";
import { TextField } from "#/components/ui/field";
import { EmptyState, Page, Skeleton } from "#/components/ui/page";
import { SelectField } from "#/components/ui/select";
import { Slider } from "#/components/ui/slider";
import { EmojiPicker } from "#/features/engagement";
import { cn } from "#/lib/cn";
import { errorMessage } from "#/lib/errors";
import { uploadAndProcess } from "#/lib/upload";
import {
  currentPosition,
  DEFAULT_CENTER,
  type LatLng,
  placeClient,
  placeKeys,
  toLatLng,
} from "./api";
import { GazetteerCredit } from "./credit";
import {
  CATEGORIES,
  EMOJI_CHOICES,
  LABELS,
  labelMeta,
  VISIBILITIES,
  visibilityKey,
} from "./labels";
import {
  cameraMs,
  circlePolygon,
  GeoLayer,
  MapClick,
  MapFrame,
  Pin,
  useIsDark,
  useMap,
  zoomForRadius,
} from "./map-kit";

const DEFAULT_RADIUS = 100;
/** The kinds of place in the order they are offered. */
const LABEL_ORDER = [
  geo.PlaceLabel.HOME,
  geo.PlaceLabel.SCHOOL,
  geo.PlaceLabel.WORK,
  geo.PlaceLabel.CUSTOM,
] as const;
const NO_CATEGORY = "none";

/** What a new place starts from: a suggestion or a search hit fills some of it in. */
export interface PlaceDraft {
  lat?: number;
  lng?: number;
  name?: string;
  label?: geo.PlaceLabel;
}

interface Form {
  name: string;
  emoji: string;
  label: geo.PlaceLabel;
  center: LatLng;
  radius: number;
  visibility: common.Visibility;
  category: string;
  /** `undefined` leaves the photo as it is; `""` removes it; otherwise a new asset. */
  photoAssetId?: string;
  photoUrl?: string;
}

/** `/places/new`: a new place, pin first. */
export function NewPlaceScreen({ draft }: { draft: PlaceDraft }) {
  const { t } = useTranslation("places");
  const given =
    draft.lat !== undefined && draft.lng !== undefined ? { lat: draft.lat, lng: draft.lng } : null;
  // Without a spot to start from, the pin starts where the device is.
  const here = useQuery({
    queryKey: ["geo", "device-position"],
    queryFn: currentPosition,
    enabled: !given,
    staleTime: 30_000,
    retry: false,
  });
  if (!given && here.isPending) return <EditorSkeleton title={t("editor.newTitle")} />;
  const label = draft.label ?? geo.PlaceLabel.CUSTOM;
  return (
    <PlaceEditor
      title={t("editor.newTitle")}
      initial={{
        name:
          draft.name ??
          (label === geo.PlaceLabel.CUSTOM ? "" : t(`labels.${labelMeta(label).key}`)),
        emoji: labelMeta(label).emoji,
        label,
        center: given ?? here.data ?? DEFAULT_CENTER,
        radius: DEFAULT_RADIUS,
        visibility: common.Visibility.PRIVATE,
        category: "",
      }}
    />
  );
}

/** `/places/<id>/edit`: change a place of mine. */
export function EditPlaceScreen({ placeId }: { placeId: string }) {
  const { t } = useTranslation("places");
  const { data, isPending, isError } = useQuery({
    queryKey: placeKeys.place(placeId),
    queryFn: () => placeClient().getPlace({ placeId }),
    retry: false,
  });
  if (isPending) return <EditorSkeleton title={t("editor.editTitle")} />;
  const place = data?.place;
  if (isError || !place?.center) {
    return (
      <Page title={t("editor.editTitle")} back="/places">
        <EmptyState emoji="🧭" title={t("place.notFound")} />
      </Page>
    );
  }
  return (
    <PlaceEditor
      title={t("editor.editTitle")}
      placeId={place.id}
      initial={{
        name: place.name,
        emoji: place.emoji || labelMeta(place.label).emoji,
        label: place.label || geo.PlaceLabel.CUSTOM,
        center: toLatLng(place.center)!,
        radius: place.radiusMeters || DEFAULT_RADIUS,
        visibility: place.visibility || common.Visibility.PRIVATE,
        category: place.category,
        photoUrl: place.photo?.url || undefined,
      }}
    />
  );
}

function EditorSkeleton({ title }: { title: string }) {
  return (
    <Page title={title} back="/places" wide>
      <Skeleton className="h-[20rem]" />
      <Skeleton className="mt-4 h-40" />
    </Page>
  );
}

function PlaceEditor({
  title,
  placeId,
  initial,
}: {
  title: string;
  placeId?: string;
  initial: Form;
}) {
  const { t, i18n } = useTranslation("places");
  const queryClient = useQueryClient();
  const navigate = useNavigate();
  const [form, setForm] = useState(initial);
  const [uploading, setUploading] = useState(false);
  const fileInput = useRef<HTMLInputElement>(null);
  const set = (change: Partial<Form>) => setForm((current) => ({ ...current, ...change }));
  // Home, work and school are only ever a badge for friends: they stay private.
  const fixedPrivate = form.label !== geo.PlaceLabel.CUSTOM;

  const area = useQuery({
    queryKey: placeKeys.reverse(form.center, i18n.language),
    queryFn: () => placeClient().reverseGeocode({ position: form.center, language: i18n.language }),
    staleTime: 10 * 60_000,
    retry: false,
  });
  const areaName = [area.data?.result?.name, area.data?.result?.region]
    .filter((part, index, all) => part && all.indexOf(part) === index)
    .join("・");

  const save = useMutation({
    mutationFn: async () => {
      const visibility = fixedPrivate ? common.Visibility.PRIVATE : form.visibility;
      const shared = {
        name: form.name.trim(),
        emoji: form.emoji,
        label: form.label,
        center: form.center,
        radiusMeters: form.radius,
        visibility,
        category: form.category,
      };
      if (placeId)
        return (
          await placeClient().updatePlace({ placeId, ...shared, photoAssetId: form.photoAssetId })
        ).place;
      return (await placeClient().createPlace({ ...shared, photoAssetId: form.photoAssetId ?? "" }))
        .place;
    },
    onSuccess: async (place) => {
      await queryClient.invalidateQueries({ queryKey: placeKeys.all });
      toast.success(placeId ? t("editor.updated") : t("editor.created"));
      if (place)
        await navigate({ to: "/places/$placeId", params: { placeId: place.id }, replace: true });
    },
    onError: (error) => toast.error(errorMessage(error, { namespace: "places" })),
  });

  const pickPhoto = async (file: File | undefined) => {
    if (!file) return;
    setUploading(true);
    try {
      const asset = await uploadAndProcess(file, { purpose: media.AssetPurpose.PLACE_PHOTO });
      set({ photoAssetId: asset.id, photoUrl: URL.createObjectURL(file) });
    } catch (error) {
      toast.error(errorMessage(error, { namespace: "places" }));
    } finally {
      setUploading(false);
    }
  };

  return (
    <Page title={title} back={placeId ? `/places/${placeId}` : "/places"} wide>
      <form
        className="grid gap-5 lg:grid-cols-[minmax(0,1fr)_24rem] lg:items-start"
        onSubmit={(event) => {
          event.preventDefault();
          save.mutate();
        }}
      >
        <div className="lg:sticky lg:top-20">
          <MapFrame
            center={form.center}
            zoom={zoomForRadius(form.radius)}
            className="sticker h-[20rem] rounded-card lg:h-[calc(100dvh-10rem)]"
          >
            <EditorMap form={form} onMove={(center) => set({ center })} />
          </MapFrame>
          <p className="mt-2 px-1 text-sm font-bold text-ink-soft">{t("editor.pinHint")}</p>
          {areaName && (
            <div className="mt-1 px-1">
              <p className="text-sm font-bold" data-testid="place-area">
                📍 {areaName}
              </p>
              <GazetteerCredit />
            </div>
          )}
        </div>

        <div className="flex flex-col gap-5">
          <fieldset>
            <legend className="px-1 text-sm font-bold text-ink-soft">{t("editor.kind")}</legend>
            <div className="mt-1.5 grid grid-cols-4 gap-2">
              {LABEL_ORDER.map((label) => {
                const meta = LABELS[label];
                const selected = form.label === label;
                return (
                  <button
                    key={meta.key}
                    type="button"
                    aria-pressed={selected}
                    onClick={() =>
                      set({
                        label,
                        emoji: EMOJI_DEFAULTS.has(form.emoji) ? meta.emoji : form.emoji,
                      })
                    }
                    className={cn(
                      "pressable flex flex-col items-center gap-1 rounded-2xl border-2 px-1 py-2.5 text-xs font-black",
                      selected
                        ? cn("border-ink text-[#17122b]", meta.tint)
                        : "border-line bg-surface text-ink-soft",
                    )}
                  >
                    <span className="text-2xl" aria-hidden>
                      {meta.emoji}
                    </span>
                    {t(`labels.${meta.key}`)}
                  </button>
                );
              })}
            </div>
          </fieldset>

          <TextField
            label={t("editor.name")}
            placeholder={t("editor.namePlaceholder")}
            value={form.name}
            maxLength={60}
            required
            onChange={(event) => set({ name: event.target.value })}
          />

          <fieldset>
            <legend className="px-1 text-sm font-bold text-ink-soft">{t("editor.icon")}</legend>
            <div className="mt-1.5 flex flex-wrap gap-1.5">
              {EMOJI_CHOICES.map((emoji) => (
                <button
                  key={emoji}
                  type="button"
                  aria-pressed={form.emoji === emoji}
                  aria-label={emoji}
                  onClick={() => set({ emoji })}
                  className={cn(
                    "grid size-10 place-items-center rounded-xl border-2 text-xl",
                    form.emoji === emoji ? "border-ink bg-lime" : "border-transparent bg-surface-2",
                  )}
                >
                  {emoji}
                </button>
              ))}
              {!EMOJI_CHOICES.includes(form.emoji) && form.emoji && (
                <span
                  className="grid size-10 place-items-center rounded-xl border-2 border-ink bg-lime text-xl"
                  role="img"
                  aria-label={form.emoji}
                >
                  {form.emoji}
                </span>
              )}
              <EmojiPicker
                onPick={(emoji) => set({ emoji })}
                trigger={
                  <button
                    type="button"
                    aria-label={t("editor.moreEmoji")}
                    title={t("editor.moreEmoji")}
                    className="grid size-10 place-items-center rounded-xl border-2 border-dashed border-line text-ink-soft"
                  >
                    <SmilePlus className="size-5" />
                  </button>
                }
              />
            </div>
          </fieldset>

          <div>
            <div className="flex items-baseline justify-between px-1">
              <span className="text-sm font-bold text-ink-soft">{t("editor.radius")}</span>
              <span className="font-black tabular-nums">{form.radius} m</span>
            </div>
            <Slider
              value={form.radius}
              min={25}
              max={1000}
              step={25}
              label={t("editor.radius")}
              valueText={`${form.radius} m`}
              onChange={(radius) => set({ radius })}
            />
            <p className="px-1 text-xs text-ink-faint">{t("editor.radiusHint")}</p>
          </div>

          <SelectField
            label={t("editor.visibility")}
            hint={fixedPrivate ? t("editor.visibilityFixed") : undefined}
            value={fixedPrivate ? "private" : visibilityKey(form.visibility)}
            disabled={fixedPrivate}
            onChange={(key) =>
              set({
                visibility:
                  VISIBILITIES.find((entry) => entry.key === key)?.value ??
                  common.Visibility.PRIVATE,
              })
            }
            options={VISIBILITIES.map((entry) => ({
              value: entry.key,
              label: t(`visibility.${entry.key}`),
              hint: t(`visibility.${entry.key}Hint`),
            }))}
          />
          {fixedPrivate && (
            <p className="-mt-3 px-1 text-xs text-ink-faint">{t("editor.visibilityFixed")}</p>
          )}

          <SelectField
            label={t("editor.category")}
            value={form.category || NO_CATEGORY}
            onChange={(category) => set({ category: category === NO_CATEGORY ? "" : category })}
            options={[
              { value: NO_CATEGORY, label: t("categories.none") },
              ...CATEGORIES.map((category) => ({
                value: category as string,
                label: t(`categories.${category}`),
              })),
              // A category set elsewhere (another client, an older version) stays selectable.
              ...(form.category &&
              !CATEGORIES.includes(form.category as (typeof CATEGORIES)[number])
                ? [{ value: form.category, label: form.category }]
                : []),
            ]}
          />

          <div>
            <p className="px-1 text-sm font-bold text-ink-soft">{t("editor.photo")}</p>
            <div className="mt-1.5 flex items-center gap-3">
              {form.photoUrl ? (
                <img
                  src={form.photoUrl}
                  alt=""
                  className="sticker-sm size-20 rounded-2xl object-cover"
                />
              ) : (
                <span className="grid size-20 place-items-center rounded-2xl border-2 border-dashed border-line text-ink-faint">
                  <ImagePlus className="size-6" />
                </span>
              )}
              <div className="flex flex-col items-start gap-1">
                <input
                  ref={fileInput}
                  type="file"
                  accept="image/*"
                  className="sr-only"
                  aria-label={t("editor.photo")}
                  onChange={(event) => void pickPhoto(event.target.files?.[0])}
                />
                <Button
                  variant="secondary"
                  size="sm"
                  loading={uploading}
                  onClick={() => fileInput.current?.click()}
                >
                  {form.photoUrl ? t("editor.photoChange") : t("editor.photoAdd")}
                </Button>
                {form.photoUrl && (
                  <Button
                    variant="ghost"
                    size="sm"
                    icon={<Trash2 className="size-4" />}
                    onClick={() => set({ photoAssetId: "", photoUrl: undefined })}
                  >
                    {t("editor.photoRemove")}
                  </Button>
                )}
              </div>
            </div>
          </div>

          <Button
            type="submit"
            size="lg"
            block
            loading={save.isPending}
            disabled={!form.name.trim() || uploading}
          >
            {placeId ? t("editor.save") : t("editor.create")}
          </Button>
        </div>
      </form>
    </Page>
  );
}

/** Emoji that only stand for a kind of place: changing the kind swaps them, a hand-picked one stays. */
const EMOJI_DEFAULTS: Set<string> = new Set(Object.values(LABELS).map((meta) => meta.emoji));

/** The pin to drag, the circle that shows the radius, and a button that brings the pin to the device. */
function EditorMap({ form, onMove }: { form: Form; onMove(center: LatLng): void }) {
  const { t } = useTranslation("places");
  const map = useMap();
  const dark = useIsDark();
  const [locating, setLocating] = useState(false);

  // Keep the whole circle in view when the radius grows.
  useEffect(() => {
    const zoom = zoomForRadius(form.radius);
    if (map.getZoom() > zoom) map.easeTo({ zoom, duration: cameraMs(300) });
  }, [map, form.radius]);

  const toDevice = async () => {
    setLocating(true);
    try {
      const position = await currentPosition();
      onMove(position);
      map.easeTo({ center: [position.lng, position.lat], duration: cameraMs(600) });
    } catch {
      toast.error(t("editor.noPosition"));
    } finally {
      setLocating(false);
    }
  };

  const tint = dark ? "#9a82ff" : "#7c5cff";
  return (
    <>
      <GeoLayer
        id="place-radius"
        data={circlePolygon(form.center, form.radius)}
        layers={[
          {
            id: "place-radius-fill",
            type: "fill",
            paint: { "fill-color": tint, "fill-opacity": 0.18 },
          },
          {
            id: "place-radius-edge",
            type: "line",
            paint: { "line-color": tint, "line-width": 2.5, "line-dasharray": [2, 1.5] },
          },
        ]}
      />
      <MapClick onClick={onMove} />
      <Pin position={form.center} zIndex={5} onDragEnd={onMove}>
        <div
          className="flex cursor-grab flex-col items-center active:cursor-grabbing"
          data-testid="place-pin"
          role="img"
          aria-label={t("editor.pin")}
        >
          <span
            className={cn(
              "sticker grid size-14 place-items-center rounded-[1.25rem] text-3xl",
              labelMeta(form.label).tint,
            )}
          >
            {form.emoji || labelMeta(form.label).emoji}
          </span>
          <span className="h-3 w-[3px] bg-ink" aria-hidden />
          <span className="size-2 rounded-full bg-ink" aria-hidden />
        </div>
      </Pin>
      <button
        type="button"
        aria-label={t("editor.toCentre")}
        title={t("editor.toCentre")}
        onClick={() => onMove({ lat: map.getCenter().lat, lng: map.getCenter().lng })}
        className="glass sticker-sm pressable absolute top-[4.25rem] right-3 z-10 grid size-11 place-items-center rounded-full"
      >
        <Crosshair className="size-5" />
      </button>
      <button
        type="button"
        aria-label={t("editor.toDevice")}
        title={t("editor.toDevice")}
        disabled={locating}
        onClick={() => void toDevice()}
        className="glass sticker-sm pressable absolute top-3 right-3 z-10 grid size-11 place-items-center rounded-full disabled:opacity-50"
      >
        <LocateFixed className="size-5" />
      </button>
    </>
  );
}
