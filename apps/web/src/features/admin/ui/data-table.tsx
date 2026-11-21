import type { MouseEvent, ReactNode } from "react";
import { Fragment } from "react";
import { cn } from "#/lib/cn";
import { useIsWide } from "./hooks";

export interface Column<T> {
  key: string;
  header: string;
  cell(row: T): ReactNode;
  /** Width and alignment of the column (table layout only). */
  className?: string;
  /**
   * Where the value goes in the card a row becomes on narrow screens:
   * `title` is the headline, `bare` joins the chips under it without a label,
   * `hidden` drops it. Anything else is a labelled line.
   */
  card?: "title" | "bare" | "hidden";
}

interface DataTableProps<T> {
  /** What the table lists, for assistive technology. */
  label: string;
  columns: Column<T>[];
  rows: T[];
  rowKey(row: T): string;
  /** Makes whole rows clickable (their title cell should still hold a real link). */
  onRowClick?(row: T): void;
  /** Details shown under a row; return nothing for rows that are collapsed. */
  expanded?(row: T): ReactNode;
}

/** Clicks on controls inside a row belong to the control, not to the row. */
function fromControl(event: MouseEvent): boolean {
  return Boolean((event.target as HTMLElement).closest("a, button, input, select, textarea, label, [role='switch']"));
}

/**
 * The console's list of records: a table with a sticky header on wide
 * screens, a stack of cards on narrow ones. Exactly one of the two is in the
 * document at a time.
 */
export function DataTable<T>(props: DataTableProps<T>) {
  return useIsWide() ? <WideTable {...props} /> : <CardList {...props} />;
}

function WideTable<T>({ label, columns, rows, rowKey, onRowClick, expanded }: DataTableProps<T>) {
  return (
    <div className="overflow-clip rounded-2xl border-2 border-line bg-surface">
      <table aria-label={label} className="w-full border-separate border-spacing-0 text-sm">
        <thead>
          <tr>
            {columns.map((column) => (
              <th key={column.key} scope="col" className={cn("sticky top-14 z-10 border-b-2 border-line bg-surface-2 px-3 py-2.5 text-left text-xs font-black whitespace-nowrap text-ink-soft", column.className)}>
                {column.header}
              </th>
            ))}
          </tr>
        </thead>
        <tbody>
          {rows.map((row) => {
            const details = expanded?.(row);
            return (
              <Fragment key={rowKey(row)}>
                <tr
                  onClick={onRowClick ? (event) => !fromControl(event) && onRowClick(row) : undefined}
                  className={cn("group/row", onRowClick && "cursor-pointer hover:bg-surface-2/60")}
                >
                  {columns.map((column) => (
                    <td key={column.key} className={cn("border-b border-line px-3 py-2.5 align-middle group-last/row:border-b-0", details && "border-b-0", column.className)}>
                      {column.cell(row)}
                    </td>
                  ))}
                </tr>
                {details && (
                  <tr>
                    <td colSpan={columns.length} className="border-b border-line bg-surface-2/50 px-3 py-3">
                      {details}
                    </td>
                  </tr>
                )}
              </Fragment>
            );
          })}
        </tbody>
      </table>
    </div>
  );
}

function CardList<T>({ label, columns, rows, rowKey, onRowClick, expanded }: DataTableProps<T>) {
  const title = columns.filter((column) => column.card === "title");
  const bare = columns.filter((column) => column.card === "bare");
  const labelled = columns.filter((column) => column.card === undefined);
  return (
    <ul aria-label={label} className="flex flex-col gap-2">
      {rows.map((row) => {
        const details = expanded?.(row);
        return (
          <li key={rowKey(row)} className="rounded-2xl border-2 border-line bg-surface text-sm">
            {/* The click is a convenience target; the title holds the real link. */}
            <div onClick={onRowClick ? (event) => !fromControl(event) && onRowClick(row) : undefined} className={cn("flex flex-col gap-2 p-3", onRowClick && "cursor-pointer")}>
              {title.map((column) => (
                <div key={column.key} className="min-w-0 font-bold">
                  {column.cell(row)}
                </div>
              ))}
              {bare.length > 0 && (
                <div className="flex flex-wrap items-center gap-1.5">
                  {bare.map((column) => (
                    <Fragment key={column.key}>{column.cell(row)}</Fragment>
                  ))}
                </div>
              )}
              {labelled.length > 0 && (
                <dl className="grid grid-cols-[auto_minmax(0,1fr)] items-baseline gap-x-3 gap-y-1">
                  {labelled.map((column) => (
                    <Fragment key={column.key}>
                      <dt className="text-xs font-bold text-ink-faint">{column.header}</dt>
                      <dd className="min-w-0">{column.cell(row)}</dd>
                    </Fragment>
                  ))}
                </dl>
              )}
            </div>
            {details && <div className="border-t-2 border-line bg-surface-2/50 p-3">{details}</div>}
          </li>
        );
      })}
    </ul>
  );
}
