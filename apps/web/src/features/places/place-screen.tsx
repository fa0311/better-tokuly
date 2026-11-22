import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { Link, useNavigate } from "@tanstack/react-router";
import { common, geo, toDate, toMillis } from "@tokuly/api";
import {
  Bookmark,
  BookmarkCheck,
  EllipsisVertical,
  MapPinCheck,
  Navigation,
  Pencil,
  Trash2,
} from "lucide-react";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { Avatar } from "#/components/ui/avatar";
import { Button } from "#/components/ui/button";
import { Chip } from "#/components/ui/card";
import { ConfirmDialog } from "#/components/ui/confirm";
import { Menu } from "#/components/ui/menu";
import { EmptyState, Page, Skeleton } from "#/components/ui/page";
import { CommentThread } from "#/features/comments";
import { EmojiText } from "#/features/emoji";
import {
  BookmarkButton,
  ReactionBar,
  ReportButton,
  ShareButton,
  useEngagement,
} from "#/features/engagement";
import { circlePolygon, GeoLayer, MapFrame, Pin, useIsDark, zoomForRadius } from "#/features/map";
import { api } from "#/lib/api";
import { errorMessage } from "#/lib/errors";
import { compactAge, compactDuration, distanceMeters } from "#/lib/format";
import { useMe } from "#/stores/session";
import {
  FRIENDS_KEY,
  loadFriends,
  localDate,
  locationClient,
  placeClient,
  placeKeys,
  toLatLng,
} from "./api";
import { CheckInDialog } from "./check-in-dialog";
import { GazetteerCredit } from "./credit";
import { labelMeta, visibilityKey } from "./labels";
import { PlaceBubble } from "./place-row";

/** A friend counts as "here now" while their pin is this fresh. */
const HERE_FRESHNESS_MS = 15 * 60_000;
const KNOWN_BADGES = ["GOOD_FOR_GROUPS", "BEST_FRIENDS_HANG_HERE", "LOYAL_REGULARS"];

/** `/places/<id>`: everything about one place, as far as the viewer may know it. */
export function PlaceScreen({ placeId }: { placeId: string }) {
  const { t, i18n } = useTranslation("places");
  const queryClient = useQueryClient();
  const navigate = useNavigate();
  const me = useMe();
  const [checkInOpen, setCheckInOpen] = useState(false);
  const [confirmDelete, setConfirmDelete] = useState(false);

  const { data, isPending, isError } = useQuery({
    queryKey: placeKeys.place(placeId),
    queryFn: () => placeClient().getPlace({ placeId }),
    retry: false,
  });
  const place = data?.place;
  const center = toLatLng(place?.center);
  const mine = Boolean(place && place.ownerId === me?.user?.id);
  const shared = Boolean(place && place.visibility !== common.Visibility.PRIVATE);

  const area = useQuery({
    queryKey: center ? placeKeys.reverse(center, i18n.language) : ["geo", "reverse", "none"],
    queryFn: () => placeClient().reverseGeocode({ position: center!, language: i18n.language }),
    enabled: Boolean(center),
    staleTime: 10 * 60_000,
    retry: false,
  });
  const visits = useQuery({
    queryKey: [...placeKeys.place(placeId), "visits"],
    queryFn: () =>
      api()
        .service(geo.HistoryService)
        .listVisits({ placeId, page: { limit: 5, cursor: "" } }),
    enabled: Boolean(place),
  });

  const setSaved = useMutation({
    mutationFn: async (saved: boolean) => {
      await (saved ? placeClient().savePlace({ placeId }) : placeClient().unsavePlace({ placeId }));
    },
    onMutate: (saved) => {
      // The button flips at once; the server's answer settles it.
      queryClient.setQueryData(
        placeKeys.place(placeId),
        (current: geo.GetPlaceResponse | undefined) =>
          current?.place ? { ...current, place: { ...current.place, saved } } : current,
      );
    },
    onSuccess: (_, saved) => toast.success(saved ? t("place.savedToast") : t("place.unsavedToast")),
    onError: (error) => toast.error(errorMessage(error, { namespace: "places" })),
    onSettled: () => queryClient.invalidateQueries({ queryKey: placeKeys.all }),
  });
  const remove = useMutation({
    mutationFn: () => placeClient().deletePlace({ placeId }),
    onSuccess: async () => {
      setConfirmDelete(false);
      queryClient.removeQueries({ queryKey: placeKeys.place(placeId) });
      await queryClient.invalidateQueries({ queryKey: placeKeys.all });
      toast.success(t("place.deleted"));
      await navigate({ to: "/places", replace: true });
    },
    onError: (error) => toast.error(errorMessage(error, { namespace: "places" })),
  });

  if (isPending) {
    return (
      <Page title={t("title")} back="/places" wide>
        <Skeleton className="h-64" />
        <Skeleton className="mt-4 h-32" />
      </Page>
    );
  }
  if (isError || !place || !center) {
    return (
      <Page title={t("title")} back="/places">
        <EmptyState emoji="🧭" title={t("place.notFound")} body={t("place.notFoundBody")} />
      </Page>
    );
  }

  const subject = { type: common.SubjectType.PLACE, id: place.id };
  const meta = labelMeta(place.label);
  const areaName = [area.data?.result?.name, area.data?.result?.region]
    .filter((part, index, all) => part && all.indexOf(part) === index)
    .join("・");
  const lastVisit = toMillis(place.myLastVisitAt);
  const badges = (data?.badges ?? []).filter((badge) => KNOWN_BADGES.includes(badge));

  return (
    <Page
      title={place.name}
      back="/places"
      wide
      actions={
        mine ? (
          <Menu
            trigger={
              <Button variant="ghost" size="icon-sm" aria-label={t("place.more")}>
                <EllipsisVertical className="size-5" />
              </Button>
            }
            actions={[
              {
                label: t("place.edit"),
                icon: <Pencil className="size-4" />,
                onSelect: () => void navigate({ to: "/places/$placeId/edit", params: { placeId } }),
              },
              {
                label: t("place.delete"),
                icon: <Trash2 className="size-4" />,
                danger: true,
                onSelect: () => setConfirmDelete(true),
              },
            ]}
          />
        ) : undefined
      }
    >
      <div className="grid gap-5 lg:grid-cols-[minmax(0,1fr)_26rem] lg:items-start">
        <div className="lg:sticky lg:top-20">
          <MapFrame
            center={center}
            zoom={zoomForRadius(place.radiusMeters || 100)}
            className="sticker h-64 rounded-card lg:h-[calc(100dvh-9rem)]"
          >
            <PlaceOnMap place={place} />
          </MapFrame>
        </div>

        <div className="flex flex-col gap-5">
          <section className="sticker rounded-card bg-surface p-5">
            <div className="flex items-start gap-4">
              {place.photo?.url ? (
                <img
                  src={place.photo.url}
                  alt=""
                  className="sticker-sm size-16 shrink-0 rounded-3xl object-cover"
                />
              ) : (
                <PlaceBubble place={place} size="lg" />
              )}
              <div className="min-w-0 flex-1">
                <h2 className="break-words text-2xl font-black leading-tight">{place.name}</h2>
                <div className="mt-2 flex flex-wrap gap-1.5">
                  {place.label !== geo.PlaceLabel.CUSTOM &&
                    place.label !== geo.PlaceLabel.UNSPECIFIED && (
                      <Chip>{t(`labels.${meta.key}`)}</Chip>
                    )}
                  {mine && <Chip>{t(`visibility.${visibilityKey(place.visibility)}`)}</Chip>}
                  {place.category && (
                    <Chip>
                      {t(`categories.${place.category}`, { defaultValue: place.category })}
                    </Chip>
                  )}
                  {place.autoDetected && <Chip>{t("place.autoDetected")}</Chip>}
                </div>
              </div>
            </div>
            {(place.address || areaName) && (
              <div className="mt-4">
                <p className="font-bold" data-testid="place-address">
                  📍 {place.address || areaName}
                </p>
                <GazetteerCredit />
              </div>
            )}
            <div className="mt-4 flex flex-wrap gap-2">
              <Button
                variant="lime"
                icon={<MapPinCheck className="size-4" />}
                onClick={() => setCheckInOpen(true)}
              >
                {t("actions.checkIn")}
              </Button>
              {!mine && (
                <Button
                  variant="secondary"
                  aria-pressed={place.saved}
                  icon={
                    place.saved ? (
                      <BookmarkCheck className="size-4" />
                    ) : (
                      <Bookmark className="size-4" />
                    )
                  }
                  onClick={() => setSaved.mutate(!place.saved)}
                >
                  {place.saved ? t("place.saved") : t("place.save")}
                </Button>
              )}
              <a
                href={`https://www.google.com/maps/dir/?api=1&destination=${center.lat},${center.lng}`}
                target="_blank"
                rel="noreferrer"
                className="pressable sticker inline-flex h-11 items-center gap-2 rounded-full bg-surface px-5 text-[15px] font-bold"
              >
                <Navigation className="size-4" />
                {t("place.directions")}
              </a>
              {mine && (
                <Link
                  to="/places/$placeId/edit"
                  params={{ placeId }}
                  className="pressable sticker inline-flex h-11 items-center gap-2 rounded-full bg-surface px-5 text-[15px] font-bold"
                >
                  <Pencil className="size-4" />
                  {t("place.edit")}
                </Link>
              )}
            </div>
            {badges.length > 0 && (
              <ul className="mt-4 flex flex-wrap gap-2">
                {badges.map((badge) => (
                  <li
                    key={badge}
                    className="sticker-sm rounded-full bg-sun px-3 py-1 text-xs font-black text-[#17122b]"
                  >
                    {t(`badges.${badge}`)}
                  </li>
                ))}
              </ul>
            )}
          </section>

          <FriendsHere place={place} />

          <section className="sticker rounded-card bg-surface p-5" aria-label={t("place.myVisits")}>
            <h3 className="font-black">{t("place.myVisits")}</h3>
            {place.myVisitCount === 0 ? (
              <p className="mt-2 text-sm text-ink-soft">{t("place.noVisits")}</p>
            ) : (
              <>
                <p className="mt-1 text-sm font-bold text-ink-soft">
                  {t("place.visitCount", { count: place.myVisitCount })}
                  {lastVisit > 0 && ` · ${t("row.lastVisit", { age: compactAge(lastVisit) })}`}
                </p>
                <ul className="mt-3 flex flex-col gap-1">
                  {(visits.data?.visits ?? []).map((visit) => {
                    const arrived = toDate(visit.arrivedAt);
                    if (!arrived) return null;
                    return (
                      <li key={visit.id}>
                        <Link
                          to="/history/$date"
                          params={{ date: localDate(arrived) }}
                          className="flex items-center gap-3 rounded-2xl px-3 py-2 transition-colors hover:bg-surface-2"
                        >
                          <span aria-hidden>{visit.checkIn ? "✅" : "👣"}</span>
                          <span className="min-w-0 flex-1 truncate font-bold">
                            {arrived.toLocaleDateString(i18n.language, {
                              month: "short",
                              day: "numeric",
                              weekday: "short",
                            })}{" "}
                            {arrived.toLocaleTimeString(i18n.language, {
                              hour: "2-digit",
                              minute: "2-digit",
                            })}
                          </span>
                          <span className="shrink-0 text-sm text-ink-soft">
                            {visit.departedAt
                              ? compactDuration(Number(visit.durationSeconds) * 1000)
                              : visit.checkIn && Number(visit.durationSeconds) === 0
                                ? t("place.checkedIn")
                                : t("place.now")}
                          </span>
                        </Link>
                      </li>
                    );
                  })}
                </ul>
              </>
            )}
          </section>

          {((data?.regulars.length ?? 0) > 0 || (data?.friendsVisited.length ?? 0) > 0) && (
            <section
              className="sticker rounded-card bg-surface p-5"
              aria-label={t("place.regulars")}
            >
              <h3 className="font-black">{t("place.regulars")}</h3>
              <p className="text-sm font-bold text-ink-soft">
                {t("place.totalVisits", { count: data?.totalVisitCount ?? 0 })}
              </p>
              <ol className="mt-3 flex flex-col gap-1">
                {data!.regulars.map((regular) =>
                  regular.user ? (
                    <li key={regular.user.id}>
                      <Link
                        to="/u/$handle"
                        params={{ handle: regular.user.handle }}
                        className="flex items-center gap-3 rounded-2xl px-3 py-2 transition-colors hover:bg-surface-2"
                      >
                        <span className="w-6 text-center font-black tabular-nums">
                          {regular.rank}
                        </span>
                        <Avatar user={regular.user} size="sm" />
                        <span className="min-w-0 flex-1 truncate font-bold">
                          <EmojiText text={regular.user.displayName} />
                        </span>
                        <span className="shrink-0 text-sm text-ink-soft">
                          {t("row.visits", { count: regular.visitCount })}
                        </span>
                      </Link>
                    </li>
                  ) : null,
                )}
              </ol>
              {(data?.friendsVisited.length ?? 0) > 0 && (
                <div className="mt-3 flex items-center gap-2 text-sm font-bold text-ink-soft">
                  <span className="flex -space-x-2">
                    {data!.friendsVisited.slice(0, 6).map((friend) => (
                      <Avatar key={friend.id} user={friend} size="xs" />
                    ))}
                  </span>
                  {t("place.friendsVisited", { count: data!.friendsVisited.length })}
                </div>
              )}
            </section>
          )}

          {shared && <PlaceEngagement subject={subject} name={place.name} mine={mine} />}
        </div>
      </div>

      <CheckInDialog
        open={checkInOpen}
        onOpenChange={setCheckInOpen}
        target={{ placeId: place.id, name: place.name }}
      />
      <ConfirmDialog
        open={confirmDelete}
        onOpenChange={setConfirmDelete}
        title={t("place.deleteTitle", { name: place.name })}
        description={t("place.deleteBody")}
        confirmLabel={t("place.delete")}
        danger
        loading={remove.isPending}
        onConfirm={() => remove.mutate()}
      />
    </Page>
  );
}

function PlaceOnMap({ place }: { place: geo.Place }) {
  const dark = useIsDark();
  const center = toLatLng(place.center)!;
  const tint = dark ? "#9a82ff" : "#7c5cff";
  return (
    <>
      <GeoLayer
        id="place-radius"
        data={circlePolygon(center, place.radiusMeters || 100)}
        layers={[
          {
            id: "place-radius-fill",
            type: "fill",
            paint: { "fill-color": tint, "fill-opacity": 0.18 },
          },
          {
            id: "place-radius-edge",
            type: "line",
            paint: { "line-color": tint, "line-width": 2.5 },
          },
        ]}
      />
      <Pin position={center}>
        <div className="flex flex-col items-center">
          <PlaceBubble place={place} size="lg" />
          <span className="h-3 w-[3px] bg-ink" aria-hidden />
        </div>
      </Pin>
    </>
  );
}

/**
 * Friends whose live pin is inside the place right now. Built from the same
 * pins the map shows, so it says nothing the viewer could not already see:
 * approximate and stale pins do not count.
 */
function FriendsHere({ place }: { place: geo.Place }) {
  const { t } = useTranslation("places");
  const locations = useQuery({
    queryKey: ["geo", "friend-locations"],
    queryFn: () => locationClient().listFriendLocations({}),
    staleTime: 30_000,
    retry: false,
  });
  const friends = useQuery({ queryKey: FRIENDS_KEY, queryFn: loadFriends, staleTime: 60_000 });
  const center = toLatLng(place.center);
  if (!center) return null;
  const now = Date.now();
  const here = (locations.data?.locations ?? []).filter(
    (location) =>
      location.position &&
      location.precision === geo.GhostMode.PRECISE &&
      !location.sharingPaused &&
      now - toMillis(location.recordedAt) <= HERE_FRESHNESS_MS &&
      distanceMeters(center, location.position) <= (place.radiusMeters || 100),
  );
  const users = here.flatMap((location) => {
    const user = friends.data?.find((friend) => friend.user?.id === location.userId)?.user;
    return user ? [user] : [];
  });
  if (users.length === 0) return null;
  return (
    <section
      className="sticker animate-pop-in rounded-card bg-lime p-5 text-[#17122b]"
      aria-label={t("place.friendsHere")}
    >
      <h3 className="font-black">{t("place.friendsHereCount", { count: users.length })}</h3>
      <ul className="mt-3 flex flex-wrap gap-3">
        {users.map((user) => (
          <li key={user.id}>
            <Link
              to="/u/$handle"
              params={{ handle: user.handle }}
              className="flex w-16 flex-col items-center gap-1"
            >
              <Avatar user={user} size="lg" outlined />
              <span className="w-full truncate text-center text-[11px] font-black">
                <EmojiText text={user.displayName} />
              </span>
            </Link>
          </li>
        ))}
      </ul>
    </section>
  );
}

/** Reactions, saving to collections, sharing, reporting and the comments of a place others can see. */
function PlaceEngagement({
  subject,
  name,
  mine,
}: {
  subject: { type: common.SubjectType; id: string };
  name: string;
  mine: boolean;
}) {
  const { t } = useTranslation("places");
  const { data } = useEngagement(subject);
  // `null`: the engagement service does not show this place to the viewer.
  if (!data) return null;
  return (
    <section className="flex flex-col gap-4" aria-label={t("place.talk")}>
      <div className="flex flex-wrap items-center gap-2">
        <ReactionBar subject={subject} />
        <BookmarkButton subject={subject} />
        <ShareButton subject={subject} title={name} />
        {!mine && <ReportButton subject={subject} label={name} />}
      </div>
      <CommentThread subject={subject} />
    </section>
  );
}
