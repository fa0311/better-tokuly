import { Plus } from "lucide-react";
import { type ReactNode, useState } from "react";
import { useTranslation } from "react-i18next";
import { SelectField } from "#/components/ui/select";
import { useSubjects } from "./api";
import { SubjectsDialog } from "./subjects-dialog";

const NONE = "none";

/**
 * Chooses one of the caller's subjects, or none. A new subject can be added
 * on the spot (someone who only uses the timer never opens the timetable),
 * and is chosen once it is saved.
 */
export function SubjectSelect({ value, onChange, label, disabled }: { value: string; onChange(subjectId: string): void; label?: ReactNode; disabled?: boolean }) {
  const { t } = useTranslation("school");
  const subjects = useSubjects();
  const [adding, setAdding] = useState(false);
  const options = [
    { value: NONE, label: t("subjects.none") },
    ...(subjects.data ?? []).map((subject) => ({ value: subject.id, label: `${subject.emoji ? `${subject.emoji} ` : ""}${subject.name}` })),
  ];
  // A subject that is archived (or not loaded yet) is not among the choices.
  const known = options.some((option) => option.value === value);
  return (
    <div className="flex items-end gap-2">
      <SelectField className="min-w-0 flex-1" label={label ?? t("subjects.one")} value={known ? value : NONE} onChange={(next) => onChange(next === NONE ? "" : next)} options={options} disabled={disabled} />
      <button
        type="button"
        aria-label={t("subjects.add")}
        title={t("subjects.add")}
        disabled={disabled}
        onClick={() => setAdding(true)}
        className="grid size-12 shrink-0 place-items-center rounded-2xl border-2 border-dashed border-ink-faint text-ink-soft transition-colors hover:border-ink hover:text-ink disabled:opacity-50"
      >
        <Plus className="size-5" />
      </button>
      <SubjectsDialog open={adding} startNew onOpenChange={setAdding} onCreated={(subject) => onChange(subject.id)} />
    </div>
  );
}
