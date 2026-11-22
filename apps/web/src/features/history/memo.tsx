import { useMutation, useQueryClient } from "@tanstack/react-query";
import { type geo, media } from "@tokuly/api";
import { ImagePlus, X } from "lucide-react";
import { useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { Button } from "#/components/ui/button";
import { TextArea } from "#/components/ui/field";
import { errorMessage } from "#/lib/errors";
import { uploadAndProcess } from "#/lib/upload";
import { historyClient, historyKeys } from "./api";

const MAX_TEXT = 2000;
const MAX_PHOTOS = 10;

interface Photo {
  assetId: string;
  url: string;
}

/** The diary entry of a day: a few words and photos. Saving it empty removes it. */
export function DayMemoEditor({ date, memo }: { date: string; memo: geo.DayMemo | undefined }) {
  const { t } = useTranslation("history");
  const queryClient = useQueryClient();
  const savedText = memo?.text ?? "";
  const savedPhotos: Photo[] = (memo?.photos ?? []).map((photo) => ({
    assetId: photo.assetId,
    url: photo.variants[0]?.url || photo.url,
  }));
  const [text, setText] = useState(savedText);
  const [photos, setPhotos] = useState(savedPhotos);
  const [uploading, setUploading] = useState(false);
  const fileInput = useRef<HTMLInputElement>(null);
  const dirty =
    text !== savedText ||
    photos.map((photo) => photo.assetId).join() !==
      savedPhotos.map((photo) => photo.assetId).join();

  const save = useMutation({
    mutationFn: () =>
      historyClient().setDayMemo({
        date,
        text: text.trim(),
        photoAssetIds: photos.map((photo) => photo.assetId),
      }),
    onSuccess: async (response) => {
      setText(response.memo?.text ?? "");
      await queryClient.invalidateQueries({ queryKey: historyKeys.all });
      toast.success(
        response.memo?.text || response.memo?.photos.length ? t("memo.saved") : t("memo.removed"),
      );
    },
    onError: (error) => toast.error(errorMessage(error, { namespace: "history" })),
  });

  const addPhotos = async (files: FileList | null) => {
    const picked = [...(files ?? [])].slice(0, MAX_PHOTOS - photos.length);
    if (picked.length === 0) return;
    setUploading(true);
    try {
      for (const file of picked) {
        const asset = await uploadAndProcess(file, { purpose: media.AssetPurpose.DAY_MEMO });
        setPhotos((current) => [...current, { assetId: asset.id, url: URL.createObjectURL(file) }]);
      }
    } catch (error) {
      toast.error(errorMessage(error, { namespace: "history" }));
    } finally {
      setUploading(false);
      if (fileInput.current) fileInput.current.value = "";
    }
  };

  return (
    <section className="sticker rounded-card bg-surface p-5" aria-label={t("memo.title")}>
      <h2 className="font-black">📝 {t("memo.title")}</h2>
      <TextArea
        className="mt-3"
        aria-label={t("memo.title")}
        placeholder={t("memo.placeholder")}
        rows={3}
        maxLength={MAX_TEXT}
        value={text}
        onChange={(event) => setText(event.target.value)}
      />
      {photos.length > 0 && (
        <ul className="mt-3 flex flex-wrap gap-2">
          {photos.map((photo) => (
            <li key={photo.assetId} className="relative">
              <img src={photo.url} alt="" className="sticker-sm size-20 rounded-2xl object-cover" />
              <button
                type="button"
                aria-label={t("memo.removePhoto")}
                onClick={() =>
                  setPhotos((current) =>
                    current.filter((candidate) => candidate.assetId !== photo.assetId),
                  )
                }
                className="absolute -top-2 -right-2 grid size-6 place-items-center rounded-full border-2 border-ink bg-surface"
              >
                <X className="size-3.5" />
              </button>
            </li>
          ))}
        </ul>
      )}
      <div className="mt-3 flex items-center justify-between gap-2">
        <input
          ref={fileInput}
          type="file"
          accept="image/*"
          multiple
          className="sr-only"
          aria-label={t("memo.addPhoto")}
          onChange={(event) => void addPhotos(event.target.files)}
        />
        <Button
          variant="ghost"
          size="sm"
          icon={<ImagePlus className="size-4" />}
          loading={uploading}
          disabled={photos.length >= MAX_PHOTOS}
          onClick={() => fileInput.current?.click()}
        >
          {t("memo.addPhoto")}
        </Button>
        <Button
          size="sm"
          disabled={!dirty || uploading}
          loading={save.isPending}
          onClick={() => save.mutate()}
        >
          {t("memo.save")}
        </Button>
      </div>
    </section>
  );
}
