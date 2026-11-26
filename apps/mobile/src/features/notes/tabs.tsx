import { NotebookPen } from "lucide-react-native";
import i18n from "#/lib/i18n";
import type { TabEntry } from "#/seams/tabs";

export default [
  { name: "notes", order: 25, title: () => i18n.t("mobile:tabs.notes"), icon: NotebookPen },
] satisfies TabEntry[];
