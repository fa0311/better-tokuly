import { useQuery, useQueryClient } from "@tanstack/react-query";
import { useTranslation } from "react-i18next";
import { Dialog } from "#/components/ui/dialog";
import { SelectField } from "#/components/ui/select";
import { PaneSpinner } from "#/components/ui/spinner";
import { chooseAudioDevice, listAudioDevices } from "./session";

const DEVICES = ["spaces", "devices"] as const;

/** Which microphone and which speaker the rooms use, now and from here on. */
export function DevicesDialog({
  open,
  onOpenChange,
}: {
  open: boolean;
  onOpenChange(open: boolean): void;
}) {
  const { t } = useTranslation("spaces");
  const queryClient = useQueryClient();
  const devices = useQuery({
    queryKey: DEVICES,
    queryFn: listAudioDevices,
    enabled: open,
    staleTime: 0,
    gcTime: 0,
  });
  const choose = async (kind: "audioinput" | "audiooutput", deviceId: string) => {
    await chooseAudioDevice(kind, deviceId);
    void queryClient.invalidateQueries({ queryKey: DEVICES });
  };
  const optionsOf = (list: MediaDeviceInfo[], fallback: string) =>
    list.map((device, index) => ({
      value: device.deviceId,
      label: device.label || `${fallback} ${index + 1}`,
    }));
  const data = devices.data;
  // A choice that is gone (an unplugged headset) falls back to the first one on offer.
  const current = (list: MediaDeviceInfo[], id: string) =>
    list.some((device) => device.deviceId === id) ? id : (list[0]?.deviceId ?? "");

  return (
    <Dialog
      open={open}
      onOpenChange={onOpenChange}
      title={t("devices.title")}
      description={t("devices.body")}
    >
      {!data ? (
        <PaneSpinner />
      ) : (
        <div className="flex flex-col gap-4">
          {data.microphones.length > 0 ? (
            <SelectField
              label={t("devices.microphone")}
              value={current(data.microphones, data.microphoneId)}
              onChange={(id) => void choose("audioinput", id)}
              options={optionsOf(data.microphones, t("devices.microphone"))}
            />
          ) : (
            <p className="text-sm font-bold text-ink-soft">{t("devices.noMicrophone")}</p>
          )}
          {data.speakers.length > 0 ? (
            <SelectField
              label={t("devices.speaker")}
              value={current(data.speakers, data.speakerId)}
              onChange={(id) => void choose("audiooutput", id)}
              options={optionsOf(data.speakers, t("devices.speaker"))}
            />
          ) : (
            // Some browsers (Safari, Firefox) play through the system's output and offer no choice.
            <p className="text-sm text-ink-soft">{t("devices.systemSpeaker")}</p>
          )}
        </div>
      )}
    </Dialog>
  );
}
