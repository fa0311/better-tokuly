import { Check, Plus } from "lucide-react-native";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { Modal, Pressable, ScrollView, Text, TextInput, View } from "react-native";
import { useSafeAreaInsets } from "react-native-safe-area-context";
import { Button } from "#/components/button";
import { Spinner } from "#/components/page";
import { useTheme } from "#/lib/theme";
import { useClipNote, useNewClip } from "./actions";
import { useClips } from "./data";

/** The server's longest clip name. */
const CLIP_NAME_MAX = 50;

/** "Add to clip": the viewer's clips with a tick on those holding the note, and a new clip made on the spot. */
export function ClipSheet({ noteId, onClose }: { noteId: string; onClose(): void }) {
  const { t } = useTranslation("clips");
  const { colors } = useTheme();
  const insets = useSafeAreaInsets();
  const clips = useClips(noteId);
  const toggle = useClipNote();
  const create = useNewClip();
  const [name, setName] = useState("");
  const [busy, setBusy] = useState(false);
  const list = clips.data?.pages.flatMap((page) => page.clips) ?? [];

  return (
    <Modal visible transparent animationType="slide" onRequestClose={onClose}>
      <Pressable accessibilityRole="none" className="flex-1 bg-black/40" onPress={onClose} />
      <View
        testID="clip-sheet"
        className="max-h-[70%] gap-3 rounded-t-3xl bg-surface px-4 pt-4"
        style={{ paddingBottom: insets.bottom + 12 }}
      >
        <Text className="text-center text-base font-black text-ink">{t("sheet.title")}</Text>
        <ScrollView contentContainerClassName="gap-1" keyboardShouldPersistTaps="handled">
          {clips.isPending ? (
            <Spinner />
          ) : list.length === 0 ? (
            <Text className="py-2 text-center text-sm text-ink-soft">{t("sheet.empty")}</Text>
          ) : (
            list.map((clip) => (
              <Pressable
                key={clip.id}
                testID={`clip-${clip.id}`}
                accessibilityRole="checkbox"
                accessibilityState={{ checked: clip.containsNote }}
                onPress={() => void toggle(clip, noteId)}
                className="flex-row items-center gap-3 rounded-2xl px-3 py-3 active:bg-surface-2"
              >
                <Text className="flex-1 font-black text-ink" numberOfLines={1}>
                  📎 {clip.name}
                </Text>
                <Text className="text-xs text-ink-faint">
                  {t("noteCount", { count: clip.noteCount })}
                </Text>
                {clip.containsNote && <Check color={colors.brand} size={20} strokeWidth={3} />}
              </Pressable>
            ))
          )}
        </ScrollView>
        <View className="flex-row items-center gap-2">
          <TextInput
            testID="clip-new-name"
            value={name}
            onChangeText={setName}
            maxLength={CLIP_NAME_MAX}
            placeholder={t("sheet.new")}
            placeholderTextColor={colors["ink-faint"]}
            className="h-11 flex-1 rounded-control bg-surface-2 px-4 text-base text-ink"
          />
          <Button
            testID="clip-create"
            size="sm"
            disabled={!name.trim()}
            loading={busy}
            icon={<Plus color={colors["on-brand"]} size={16} />}
            onPress={async () => {
              setBusy(true);
              if (await create(name.trim(), noteId)) setName("");
              setBusy(false);
            }}
          >
            {t("form.create")}
          </Button>
        </View>
      </View>
    </Modal>
  );
}
