import { useQuery } from "@tanstack/react-query";
import type { shorts } from "@tokuly/api";
import { Music2, X } from "lucide-react";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { TextField } from "#/components/ui/field";
import { ListRow } from "#/components/ui/page";
import { EmojiText } from "#/features/emoji";
import { compactCount } from "#/features/engagement";
import { discoveryClient, shortKeys } from "../api";
import { soundTitle } from "../model";
import { ChoiceChips } from "../sort-chips";

/** What a new short is set to. */
export type SoundChoice =
  /** The video's own audio, which becomes an original sound; `title` names it (optional). */
  | { kind: "own"; title: string }
  /** A sound that already exists: another short's, or a named track. */
  | { kind: "existing"; sound: shorts.Sound }
  /** A track from outside Tokuly, by name. */
  | { kind: "external"; title: string; artist: string };

/** The choice as the API takes it. `undefined` leaves it to the server (an original sound of its own). */
export function soundInput(choice: SoundChoice):
  | { sound: { case: "soundId" | "originalTitle"; value: string } }
  | {
      sound: { case: "external"; value: { title: string; artist: string } };
    }
  | undefined {
  if (choice.kind === "existing") return { sound: { case: "soundId", value: choice.sound.id } };
  if (choice.kind === "external") {
    const title = choice.title.trim();
    return title
      ? { sound: { case: "external", value: { title, artist: choice.artist.trim() } } }
      : undefined;
  }
  const title = choice.title.trim();
  return title ? { sound: { case: "originalTitle", value: title } } : undefined;
}

/** Sound attribution of a new short: its own audio, a sound picked from the ones in use, or a track by name. */
export function SoundField({
  value,
  onChange,
}: {
  value: SoundChoice;
  onChange(value: SoundChoice): void;
}) {
  const { t, i18n } = useTranslation("shorts");
  const [query, setQuery] = useState("");
  const search = query.trim();
  const suggestions = useQuery({
    queryKey: shortKeys.soundSuggestions(search),
    queryFn: async () =>
      (await discoveryClient().suggestSounds({ query: search, limit: 8 })).sounds,
    enabled: value.kind === "existing",
    staleTime: 60_000,
  });
  const original = (name: string) => t("sound.original", { name });
  const picked = value.kind === "existing" && value.sound.id ? value.sound : undefined;

  return (
    <div className="flex flex-col gap-3">
      <ChoiceChips
        label={t("form.sound.label")}
        value={value.kind}
        onChange={(kind) =>
          onChange(
            kind === "own"
              ? { kind, title: "" }
              : kind === "external"
                ? { kind, title: "", artist: "" }
                : { kind: "existing", sound: { id: "" } as shorts.Sound },
          )
        }
        options={[
          { value: "own", label: t("form.sound.own") },
          { value: "existing", label: t("form.sound.existing") },
          { value: "external", label: t("form.sound.external") },
        ]}
      />
      {value.kind === "own" && (
        <TextField
          label={t("form.sound.ownTitle")}
          hint={t("form.sound.ownHint")}
          value={value.title}
          maxLength={100}
          onChange={(event) => onChange({ kind: "own", title: event.target.value })}
        />
      )}
      {value.kind === "external" && (
        <>
          <TextField
            label={t("form.sound.trackTitle")}
            value={value.title}
            maxLength={100}
            onChange={(event) => onChange({ ...value, title: event.target.value })}
          />
          <TextField
            label={t("form.sound.trackArtist")}
            value={value.artist}
            maxLength={100}
            onChange={(event) => onChange({ ...value, artist: event.target.value })}
          />
        </>
      )}
      {value.kind === "existing" &&
        (picked ? (
          <div
            className="flex items-center gap-3 rounded-2xl border-2 border-ink bg-brand-soft px-3 py-2.5"
            data-testid="picked-sound"
          >
            <Music2 className="size-5 shrink-0" aria-hidden />
            <span className="min-w-0 flex-1 truncate font-bold">
              <EmojiText text={soundTitle(picked, original)} />
            </span>
            <button
              type="button"
              aria-label={t("form.sound.change")}
              className="grid size-9 place-items-center rounded-full hover:bg-surface-2"
              onClick={() => onChange({ kind: "existing", sound: { id: "" } as shorts.Sound })}
            >
              <X className="size-5" />
            </button>
          </div>
        ) : (
          <>
            <TextField
              label={t("form.sound.search")}
              value={query}
              placeholder={t("form.sound.searchPlaceholder")}
              onChange={(event) => setQuery(event.target.value)}
            />
            <ul className="flex flex-col">
              {(suggestions.data ?? [])
                .filter((sound) => sound.reusable)
                .map((sound) => (
                  <li key={sound.id}>
                    <ListRow
                      onClick={() => onChange({ kind: "existing", sound })}
                      leading={
                        <span className="grid size-10 shrink-0 place-items-center rounded-2xl bg-pink text-white">
                          <Music2 className="size-5" />
                        </span>
                      }
                      title={<EmojiText text={soundTitle(sound, original)} />}
                      subtitle={t("sound.uses", {
                        count: compactCount(Number(sound.useCount), i18n.language),
                      })}
                    />
                  </li>
                ))}
              {suggestions.data?.length === 0 && (
                <li className="px-3 py-2 text-sm text-ink-soft">{t("form.sound.none")}</li>
              )}
            </ul>
          </>
        ))}
    </div>
  );
}
