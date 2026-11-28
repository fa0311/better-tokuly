import { useNavigate } from "@tanstack/react-router";
import { useTranslation } from "react-i18next";
import { Avatar } from "#/components/ui/avatar";
import { MapPin } from "#/features/map";
import { imageUrl } from "#/features/notes";
import { useMapMoments } from "./api";

/**
 * Friends' moments of today on the live map, where they were taken: a
 * little polaroid of the back camera with the friend's face. Only moments
 * the caller may see the place of (unlocked, and the friend's precise
 * position visible to them) come with one.
 */
export function MomentsOnMap() {
  const { t } = useTranslation("moments");
  const navigate = useNavigate();
  const { data } = useMapMoments();
  return (
    <>
      {data?.moments.map((moment) => {
        const location = moment.location;
        if (!location) return null;
        const name = moment.author?.displayName ?? "";
        return (
          <MapPin key={moment.id} lng={location.lng} lat={location.lat} zIndex={15}>
            <button
              type="button"
              onClick={() =>
                void navigate({ to: "/moments/$momentId", params: { momentId: moment.id } })
              }
              aria-label={t("map.open", { name })}
              className="pressable sticker relative flex -rotate-3 flex-col rounded-xl bg-surface p-1 pb-1.5"
              data-testid="map-moment"
            >
              <span
                className="block h-16 w-12 overflow-hidden rounded-lg bg-surface-3"
                style={{ background: moment.back?.dominantColor || undefined }}
              >
                {moment.back?.url && (
                  <img src={imageUrl(moment.back, 48)} alt="" className="size-full object-cover" />
                )}
              </span>
              <span className="absolute -right-2 -bottom-2">
                <Avatar user={moment.author} size="xs" outlined />
              </span>
              <span className="absolute -top-2 -left-2 text-sm" aria-hidden>
                ⚡
              </span>
            </button>
          </MapPin>
        );
      })}
    </>
  );
}
