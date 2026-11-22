import type { chat } from "@tokuly/api";
import { ExternalLink, LocateFixed, Send } from "lucide-react";
import { lazy, Suspense, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { Button } from "#/components/ui/button";
import { Dialog } from "#/components/ui/dialog";
import { TextField } from "#/components/ui/field";
import { PaneSpinner } from "#/components/ui/spinner";
import { useLocations } from "#/stores/locations";

const LocationMap = lazy(() => import("./location-map"));

/** Shibuya crossing: where the map opens when the device will not say where it is. */
const FALLBACK = { lat: 35.6595, lng: 139.7005 };

interface Position {
  lat: number;
  lng: number;
}

/** The device's position, asked once (the app's own tracker may know it already). */
function useDevicePosition(enabled: boolean): { position: Position | null; denied: boolean } {
  const tracked = useLocations((state) => state.own);
  const [asked, setAsked] = useState<Position | null>(null);
  const [denied, setDenied] = useState(false);
  const known = Boolean(tracked);
  useEffect(() => {
    if (!enabled || known || !navigator.geolocation) return;
    let cancelled = false;
    navigator.geolocation.getCurrentPosition(
      (result) => !cancelled && setAsked({ lat: result.coords.latitude, lng: result.coords.longitude }),
      () => !cancelled && setDenied(true),
      { enableHighAccuracy: true, timeout: 10_000, maximumAge: 60_000 },
    );
    return () => {
      cancelled = true;
    };
  }, [enabled, known]);
  return { position: tracked ? { lat: tracked.lat, lng: tracked.lng } : asked, denied: denied && !tracked };
}

/**
 * "Share a place": the map opens on where the person is; dragging it moves
 * the pin somewhere else (a meeting point), and a name can be added.
 */
export function LocationShareDialog({ open, onOpenChange, onShare }: { open: boolean; onOpenChange(open: boolean): void; onShare(location: { lat: number; lng: number; name: string; address: string }): void }) {
  const { t, i18n } = useTranslation("chat");
  const device = useDevicePosition(open);
  const [chosen, setChosen] = useState<Position | null>(null);
  const [name, setName] = useState("");
  // Each opening starts from the device's position again.
  useEffect(() => {
    if (!open) {
      setChosen(null);
      setName("");
    }
  }, [open]);
  const start = device.position ?? FALLBACK;
  const position = chosen ?? start;

  return (
    <Dialog
      open={open}
      onOpenChange={onOpenChange}
      title={t("location.shareTitle")}
      description={device.denied ? t("location.denied") : t("location.shareLead")}
      footer={
        <Button
          icon={<Send className="size-4" />}
          onClick={() => {
            onShare({ lat: position.lat, lng: position.lng, name: name.trim(), address: "" });
            onOpenChange(false);
          }}
        >
          {t("location.send")}
        </Button>
      }
    >
      <div className="flex flex-col gap-3">
        <div className="relative h-64 overflow-hidden rounded-card border-2 border-line bg-surface-2">
          {open && (
            <Suspense fallback={<PaneSpinner />}>
              <LocationMap lat={chosen ? chosen.lat : start.lat} lng={chosen ? chosen.lng : start.lng} onMove={setChosen} language={i18n.language} />
            </Suspense>
          )}
          {device.position && chosen && (
            <button
              type="button"
              aria-label={t("location.here")}
              title={t("location.here")}
              onClick={() => setChosen(null)}
              className="sticker-sm absolute right-3 bottom-3 grid size-10 place-items-center rounded-full bg-surface"
            >
              <LocateFixed className="size-5" />
            </button>
          )}
        </div>
        <TextField label={t("location.name")} placeholder={t("location.namePlaceholder")} value={name} maxLength={100} onChange={(event) => setName(event.target.value)} />
      </div>
    </Dialog>
  );
}

/** A shared place on the map, with a way out to a maps app for directions. */
export function LocationViewDialog({ location, onClose }: { location: chat.LocationShare | null; onClose(): void }) {
  const { t, i18n } = useTranslation("chat");
  // The last place stays while the dialog animates out.
  const [shown, setShown] = useState(location);
  if (location && location !== shown) setShown(location);
  const position = shown?.position;
  if (!shown || !position) return null;
  return (
    <Dialog
      open={location !== null}
      onOpenChange={(open) => !open && onClose()}
      title={shown.name || t("location.shared")}
      description={shown.address || undefined}
      footer={
        <a
          href={`https://www.google.com/maps/search/?api=1&query=${position.lat},${position.lng}`}
          target="_blank"
          rel="noreferrer"
          className="pressable sticker inline-flex h-11 items-center gap-2 rounded-full bg-surface px-5 font-bold"
        >
          <ExternalLink className="size-4" />
          {t("location.directions")}
        </a>
      }
    >
      <div className="h-72 overflow-hidden rounded-card border-2 border-line bg-surface-2">
        {location !== null && (
          <Suspense fallback={<PaneSpinner />}>
            <LocationMap lat={position.lat} lng={position.lng} language={i18n.language} />
          </Suspense>
        )}
      </div>
    </Dialog>
  );
}
