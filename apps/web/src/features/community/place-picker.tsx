import { useState } from "react";
import { useTranslation } from "react-i18next";
import { Button } from "#/components/ui/button";
import { DEFAULT_CENTER, MapClick, MapFrame, Pin } from "#/features/map";
import { NS } from "./api";

export interface LatLng {
  lat: number;
  lng: number;
}

/**
 * A map to put an event's pin on: tap the map or drag the pin. It is loaded
 * only when someone opens it (the map library is large).
 */
export default function PlacePicker({
  value,
  onChange,
}: {
  value: LatLng | undefined;
  onChange(value: LatLng | undefined): void;
}) {
  const { t } = useTranslation(NS);
  const [center] = useState<LatLng>(value ?? { lat: DEFAULT_CENTER[1], lng: DEFAULT_CENTER[0] });
  return (
    <div className="flex flex-col gap-2">
      <MapFrame
        center={center}
        zoom={value ? 15 : 11}
        className="h-56 rounded-2xl border-2 border-line"
      >
        <MapClick onClick={onChange} />
        {value && (
          <Pin position={value} onDragEnd={onChange}>
            <span className="text-4xl drop-shadow" aria-hidden>
              📍
            </span>
          </Pin>
        )}
      </MapFrame>
      <div className="flex items-center gap-2">
        <p className="min-w-0 flex-1 text-xs text-ink-faint">{t("event.form.pinHint")}</p>
        {value && (
          <Button variant="ghost" size="sm" onClick={() => onChange(undefined)}>
            {t("event.form.pinRemove")}
          </Button>
        )}
      </div>
    </div>
  );
}
