import type { Subject } from "@tokuly/core/engagement";
import { lazy, Suspense } from "react";
import { create } from "zustand";

const CollectionChooser = lazy(() =>
  import("./collection-chooser").then((module) => ({ default: module.CollectionChooser })),
);

const useChooser = create<{ subject: Subject | null }>(() => ({ subject: null }));

/** Opens the collection chooser for a subject, from anywhere (a button, a toast, a list row). */
export function openCollectionChooser(subject: Subject): void {
  useChooser.setState({ subject });
}

/** The one chooser of the app; mounted with the engagement background so it outlives the control that opened it. */
export function CollectionChooserHost() {
  const subject = useChooser((state) => state.subject);
  if (!subject) return null;
  return (
    <Suspense fallback={null}>
      <CollectionChooser
        key={`${subject.type}:${subject.id}`}
        subject={subject}
        onClose={() => useChooser.setState({ subject: null })}
      />
    </Suspense>
  );
}
