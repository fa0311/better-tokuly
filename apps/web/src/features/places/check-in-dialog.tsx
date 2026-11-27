import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useNavigate } from "@tanstack/react-router";
import { toDate } from "@tokuly/api";
import { placeEmoji } from "@tokuly/core/places";
import { Check } from "lucide-react";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { Avatar } from "#/components/ui/avatar";
import { Button } from "#/components/ui/button";
import { Dialog } from "#/components/ui/dialog";
import { TextArea } from "#/components/ui/field";
import { Skeleton } from "#/components/ui/page";
import { cn } from "#/lib/cn";
import { errorMessage } from "#/lib/errors";
import { distanceMeters } from "#/lib/format";
import {
  currentPosition,
  FRIENDS_KEY,
  type LatLng,
  loadFriends,
  localDate,
  placeClient,
  placeKeys,
} from "./api";
import { GazetteerCredit } from "./credit";

const MAX_MESSAGE = 200;
const MAX_TAGGED = 20;
/** Places this close to the device are offered for a check-in "here". */
const NEARBY_METERS = 300;

/** Where a check-in goes when the caller already knows: a place, or a point with a name to show. */
export interface CheckInTarget {
  placeId?: string;
  position?: LatLng;
  name: string;
}

/**
 * "I'm here": checks in at `target`, or — without one — wherever the device
 * is, with the places around it to choose from. A message and tagged
 * friends are optional.
 */
export function CheckInDialog({
  open,
  onOpenChange,
  target,
}: {
  open: boolean;
  onOpenChange(open: boolean): void;
  target?: CheckInTarget;
}) {
  const { t, i18n } = useTranslation("places");
  const queryClient = useQueryClient();
  const navigate = useNavigate();
  const [message, setMessage] = useState("");
  const [tagged, setTagged] = useState<string[]>([]);
  const [pickedPlaceId, setPickedPlaceId] = useState<string | null>(null);

  const here = useQuery({
    queryKey: ["geo", "device-position"],
    queryFn: currentPosition,
    enabled: open && !target,
    staleTime: 30_000,
    retry: false,
  });
  const position = here.data;
  const area = useQuery({
    queryKey: position ? placeKeys.reverse(position, i18n.language) : ["geo", "reverse", "none"],
    queryFn: () => placeClient().reverseGeocode({ position: position!, language: i18n.language }),
    enabled: open && !target && Boolean(position),
    staleTime: 10 * 60_000,
    retry: false,
  });
  const around = useQuery({
    queryKey: placeKeys.search("", position ?? null),
    queryFn: () => placeClient().searchPlaces({ query: "", near: position!, limit: 10 }),
    enabled: open && !target && Boolean(position),
    staleTime: 60_000,
    retry: false,
  });
  const friends = useQuery({
    queryKey: FRIENDS_KEY,
    queryFn: loadFriends,
    enabled: open,
    staleTime: 60_000,
  });

  const nearby = (around.data?.results ?? []).flatMap((result) =>
    result.place?.center &&
    position &&
    distanceMeters(position, result.place.center) <=
      Math.max(NEARBY_METERS, result.place.radiusMeters)
      ? [result.place]
      : [],
  );

  const checkIn = useMutation({
    mutationFn: () => {
      const placeId = target?.placeId ?? pickedPlaceId;
      const where = placeId
        ? ({ case: "placeId", value: placeId } as const)
        : ({ case: "position", value: target?.position ?? position! } as const);
      return placeClient().checkIn({ where, message: message.trim(), withUserIds: tagged });
    },
    onSuccess: (response) => {
      void queryClient.invalidateQueries({ queryKey: ["geo"] });
      onOpenChange(false);
      setMessage("");
      setTagged([]);
      setPickedPlaceId(null);
      const date = localDate(toDate(response.visit?.arrivedAt) ?? new Date());
      toast.success(t("checkIn.done"), {
        action: {
          label: t("checkIn.seeDay"),
          onClick: () => void navigate({ to: "/history/$date", params: { date } }),
        },
      });
    },
    onError: (error) => toast.error(errorMessage(error, { namespace: "places" })),
  });

  const ready = Boolean(target) || Boolean(position);
  const toggle = (userId: string) =>
    setTagged((current) =>
      current.includes(userId)
        ? current.filter((id) => id !== userId)
        : current.length < MAX_TAGGED
          ? [...current, userId]
          : current,
    );

  return (
    <Dialog
      open={open}
      onOpenChange={onOpenChange}
      title={t("checkIn.title")}
      description={t("checkIn.lead")}
      footer={
        <Button
          variant="lime"
          disabled={!ready}
          loading={checkIn.isPending}
          onClick={() => checkIn.mutate()}
        >
          {t("checkIn.submit")}
        </Button>
      }
    >
      <div className="flex flex-col gap-5">
        {target ? (
          <p className="rounded-2xl bg-surface-2 px-4 py-3 font-black">📍 {target.name}</p>
        ) : here.isPending ? (
          <Skeleton className="h-12" />
        ) : here.isError ? (
          <p
            role="alert"
            className="rounded-2xl bg-surface-2 px-4 py-3 text-sm font-bold text-ink-soft"
          >
            {t("checkIn.noPosition")}
          </p>
        ) : (
          <fieldset>
            <legend className="px-1 text-sm font-bold text-ink-soft">{t("checkIn.where")}</legend>
            <div className="mt-1.5 flex flex-wrap gap-2">
              <Choice selected={pickedPlaceId === null} onSelect={() => setPickedPlaceId(null)}>
                📍{" "}
                {area.data?.result?.name
                  ? t("checkIn.hereNamed", { name: area.data.result.name })
                  : t("checkIn.here")}
              </Choice>
              {nearby.map((place) => (
                <Choice
                  key={place.id}
                  selected={pickedPlaceId === place.id}
                  onSelect={() => setPickedPlaceId(place.id)}
                >
                  {placeEmoji(place)} {place.name}
                </Choice>
              ))}
            </div>
            {area.data?.result?.name && <GazetteerCredit className="mt-1.5 px-1" />}
          </fieldset>
        )}

        <TextArea
          label={t("checkIn.message")}
          placeholder={t("checkIn.messagePlaceholder")}
          value={message}
          maxLength={MAX_MESSAGE}
          rows={2}
          onChange={(event) => setMessage(event.target.value)}
        />

        {(friends.data?.length ?? 0) > 0 && (
          <fieldset>
            <legend className="px-1 text-sm font-bold text-ink-soft">{t("checkIn.with")}</legend>
            <div className="scrollbar-none mt-1.5 flex gap-3 overflow-x-auto pb-1">
              {friends.data!.map((friend) => {
                const user = friend.user;
                if (!user) return null;
                const selected = tagged.includes(user.id);
                return (
                  <button
                    key={user.id}
                    type="button"
                    aria-pressed={selected}
                    onClick={() => toggle(user.id)}
                    className="flex w-16 shrink-0 flex-col items-center gap-1"
                  >
                    <span className="relative">
                      <Avatar
                        user={user}
                        size="lg"
                        outlined
                        className={cn(!selected && "opacity-60")}
                      />
                      {selected && (
                        <span className="sticker-sm absolute -top-1.5 -right-1.5 grid size-6 place-items-center rounded-full bg-lime text-[#17122b]">
                          <Check className="size-3.5" />
                        </span>
                      )}
                    </span>
                    <span className="w-full truncate text-center text-[11px] font-bold">
                      {friend.nickname || user.displayName}
                    </span>
                  </button>
                );
              })}
            </div>
          </fieldset>
        )}
        <p className="text-xs text-ink-faint">{t("checkIn.audience")}</p>
      </div>
    </Dialog>
  );
}

function Choice({
  selected,
  onSelect,
  children,
}: {
  selected: boolean;
  onSelect(): void;
  children: React.ReactNode;
}) {
  return (
    <button
      type="button"
      aria-pressed={selected}
      onClick={onSelect}
      className={cn(
        "pressable max-w-full truncate rounded-full border-2 px-4 py-2 text-sm font-black",
        selected ? "border-ink bg-lime text-[#17122b]" : "border-line bg-surface text-ink-soft",
      )}
    >
      {children}
    </button>
  );
}
