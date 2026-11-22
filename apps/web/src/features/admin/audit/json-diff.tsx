import { useTranslation } from "react-i18next";
import { cn } from "#/lib/cn";

type Snapshot = Record<string, unknown>;

/** A JSON snapshot as an object, `{}` when empty, `null` when it is not an object. */
function parseSnapshot(json: string): Snapshot | null {
  if (!json.trim()) return {};
  try {
    const value: unknown = JSON.parse(json);
    return value !== null && typeof value === "object" && !Array.isArray(value)
      ? (value as Snapshot)
      : null;
  } catch {
    return null;
  }
}

const show = (value: unknown) =>
  value === undefined ? "" : typeof value === "string" ? value : JSON.stringify(value, null, 2);

/**
 * The before / after snapshots of an audit entry, field by field, with the
 * fields that changed marked. Snapshots that are not JSON objects are shown
 * as they are.
 */
export function JsonDiff({ beforeJson, afterJson }: { beforeJson: string; afterJson: string }) {
  const { t } = useTranslation("admin");
  const before = parseSnapshot(beforeJson);
  const after = parseSnapshot(afterJson);

  if (!before || !after) {
    return (
      <div className="grid gap-3 md:grid-cols-2">
        {[
          [t("audit.before"), beforeJson],
          [t("audit.after"), afterJson],
        ].map(([label, json]) => (
          <div key={label}>
            <p className="mb-1 text-xs font-black text-ink-soft">{label}</p>
            <pre className="overflow-x-auto rounded-xl bg-surface p-3 font-mono text-xs whitespace-pre-wrap break-all">
              {json || "—"}
            </pre>
          </div>
        ))}
      </div>
    );
  }

  const keys = [...new Set([...Object.keys(before), ...Object.keys(after)])];
  return (
    <table
      className="w-full table-fixed border-separate border-spacing-0 overflow-hidden rounded-xl bg-surface text-xs"
      aria-label={t("audit.details")}
    >
      <thead>
        <tr className="text-left text-ink-soft">
          <th scope="col" className="w-1/4 border-b border-line px-3 py-2 font-black">
            {t("audit.field")}
          </th>
          <th scope="col" className="border-b border-line px-3 py-2 font-black">
            {t("audit.before")}
          </th>
          <th scope="col" className="border-b border-line px-3 py-2 font-black">
            {t("audit.after")}
          </th>
        </tr>
      </thead>
      <tbody>
        {keys.map((key) => {
          const changed = JSON.stringify(before[key]) !== JSON.stringify(after[key]);
          return (
            <tr key={key} className={cn(changed && "bg-sun/15")}>
              <th
                scope="row"
                className="border-b border-line px-3 py-1.5 text-left align-top font-mono font-bold break-all"
              >
                {key}
                {changed && (
                  <span className="ml-1.5 rounded bg-sun px-1 font-sans text-[10px] font-black text-[#17122b]">
                    {t("audit.changed")}
                  </span>
                )}
              </th>
              <td className="border-b border-line px-3 py-1.5 align-top font-mono whitespace-pre-wrap break-all text-ink-soft">
                {show(before[key]) || "—"}
              </td>
              <td className="border-b border-line px-3 py-1.5 align-top font-mono whitespace-pre-wrap break-all">
                {show(after[key]) || "—"}
              </td>
            </tr>
          );
        })}
      </tbody>
    </table>
  );
}
