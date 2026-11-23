import { ChevronRight } from "lucide-react";
import { Fragment, type ReactNode, useState } from "react";
import { useTranslation } from "react-i18next";
import { cn } from "#/lib/cn";
import {
  fieldsOf,
  nestedRef,
  resolve,
  type SchemaDocument,
  type SchemaNode,
  shortName,
  typeLabel,
} from "./schema";

/** A proto comment as text: paragraphs, with `code` spans. */
export function Prose({ text, className }: { text: string; className?: string }) {
  if (!text.trim()) return null;
  return (
    <div className={cn("flex flex-col gap-2 leading-relaxed", className)}>
      {text.split(/\n\s*\n/).map((paragraph, index) => (
        <p key={index}>{inlineCode(paragraph.replace(/\s*\n\s*/g, " "))}</p>
      ))}
    </div>
  );
}

function inlineCode(text: string): ReactNode[] {
  return text.split(/(`[^`]+`)/).map((part, index) =>
    part.startsWith("`") && part.endsWith("`") && part.length > 2 ? (
      <code key={index} className="rounded-md bg-surface-2 px-1 py-0.5 font-mono text-[0.86em]">
        {part.slice(1, -1)}
      </code>
    ) : (
      <Fragment key={index}>{part}</Fragment>
    ),
  );
}

/** The fields of a request or response, with nested messages and enums opening in place. */
export function SchemaView({ document }: { document: SchemaDocument }) {
  const { t } = useTranslation("dev");
  const root = resolve(document, document);
  if (fieldsOf(root).length === 0)
    return <p className="px-1 text-sm text-ink-soft">{t("reference.noFields")}</p>;
  return <Definition document={document} definition={root} />;
}

function Definition({
  document,
  definition,
}: {
  document: SchemaDocument;
  definition: SchemaNode;
}) {
  const { t } = useTranslation("dev");
  if (definition.enum) {
    const notes = definition["x-enum-descriptions"] ?? {};
    return (
      <ul className="flex flex-col gap-1">
        {definition.enum.map((value) => (
          <li key={value} className="text-sm">
            <code className="font-mono text-xs font-bold">{value}</code>
            {notes[value] && <span className="ml-2 text-ink-soft">{notes[value]}</span>}
          </li>
        ))}
      </ul>
    );
  }
  return (
    <ul className="divide-y-2 divide-line rounded-2xl border-2 border-line">
      {fieldsOf(definition).map((field) => (
        <li key={field.name} className="px-3 py-2.5">
          <FieldRow document={document} name={field.name} node={field.node}>
            {field.oneof && (
              <span className="rounded-full bg-surface-2 px-2 py-0.5 text-[11px] font-bold text-ink-soft">
                {t("reference.oneof", { name: field.oneof })}
              </span>
            )}
          </FieldRow>
        </li>
      ))}
    </ul>
  );
}

function FieldRow({
  document,
  name,
  node,
  children,
}: {
  document: SchemaDocument;
  name: string;
  node: SchemaNode;
  children?: ReactNode;
}) {
  const [open, setOpen] = useState(false);
  const nested = nestedRef(node);
  const definition = nested ? document.$defs?.[nested] : undefined;
  return (
    <>
      <div className="flex flex-wrap items-center gap-x-2.5 gap-y-1">
        <code className="font-mono text-sm font-bold">{name}</code>
        {definition ? (
          <button
            type="button"
            aria-expanded={open}
            onClick={() => setOpen((shown) => !shown)}
            className="inline-flex items-center gap-0.5 rounded-full bg-brand-soft px-2 py-0.5 font-mono text-xs font-bold text-brand-strong"
          >
            <ChevronRight className={cn("size-3.5 transition-transform", open && "rotate-90")} />
            {typeLabel(node)}
          </button>
        ) : (
          <code className="font-mono text-xs text-ink-soft">{typeLabel(node)}</code>
        )}
        {children}
      </div>
      {node.description && <Prose text={node.description} className="mt-1 text-sm text-ink-soft" />}
      {/* Rendered only when opened: a message may contain itself. */}
      {open && definition && nested && (
        <div className="mt-2 border-l-4 border-brand-soft pl-3">
          <p className="mb-1.5 font-mono text-xs font-bold text-ink-soft">{shortName(nested)}</p>
          {definition.description && (
            <Prose text={definition.description} className="mb-2 text-sm text-ink-soft" />
          )}
          <Definition document={document} definition={definition} />
        </div>
      )}
    </>
  );
}
