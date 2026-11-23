import type { ReactNode } from "react";

/** A titled card of a form: one step of posting or editing a short. */
export function FormSection({
  title,
  lead,
  children,
}: {
  title: string;
  lead?: string;
  children: ReactNode;
}) {
  return (
    <section className="sticker rounded-card bg-surface p-4">
      <h2 className="text-lg font-black">{title}</h2>
      {lead && <p className="mt-0.5 text-sm text-ink-soft">{lead}</p>}
      <div className="mt-3 flex flex-col gap-3">{children}</div>
    </section>
  );
}
