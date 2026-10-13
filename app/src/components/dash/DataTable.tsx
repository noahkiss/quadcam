import { Fragment, type ReactNode } from "react";
import styles from "./DataTable.module.css";

export interface Column<T> {
  key: string;
  header: ReactNode;
  cell: (row: T) => ReactNode;
  /** A number: right-aligned, mono, tabular. */
  numeric?: boolean;
  /** The column that names the row (`<th scope="row">`). One per table. */
  rowHeader?: boolean;
  /** Hidden when the table is narrower than 720 px. */
  optional?: boolean;
  /** A CSS width, such as `6rem`; the other columns share the rest. */
  width?: string;
}

export interface Selection<T> {
  selected: (row: T) => boolean;
  /** A box that stands for other rows, some of them picked: shown as mixed. */
  mixed?: (row: T) => boolean;
  onChange: (row: T, on: boolean) => void;
  /** The checkbox's accessible name. */
  label: (row: T) => string;
}

export interface DataTableProps<T> {
  /** The table's accessible name. */
  label: string;
  columns: Column<T>[];
  rows: T[];
  rowKey: (row: T) => string;
  /** A checkbox column first, for rows a person picks. */
  selection?: Selection<T>;
  /** Group rows: a header row before each run of rows with a new group. */
  group?: (row: T) => string;
  /** Tighter rows, for long lists. */
  compact?: boolean;
  /** Shown in place of the table when there are no rows. */
  empty?: ReactNode;
  /** A CSS height, such as `min(60vh, 480px)`: the rows scroll inside a box that tall, under
   *  a header that stays. A focused or picked row scrolls into view. */
  maxHeight?: string;
}

/** Scrolls the row that holds `el` into the table's box, when the box scrolls. */
function reveal(el: Element) {
  el.closest("tr")?.scrollIntoView({ block: "nearest" });
}

/** A table of rows that share columns: aligned numbers, an optional checkbox column, optional
 *  group header rows, row actions in a last column. It scrolls sideways inside its own box
 *  when the window is too narrow, and drops `optional` columns first. With `maxHeight` it
 *  scrolls down inside that box too. */
export function DataTable<T>({ label, columns, rows, rowKey, selection, group, compact, empty, maxHeight }: DataTableProps<T>) {
  if (rows.length === 0 && empty) return <>{empty}</>;
  const span = columns.length + (selection ? 1 : 0);
  const cls = (c: Column<T>) => [c.numeric && styles.num, c.optional && styles.optional].filter(Boolean).join(" ") || undefined;
  let last: string | null = null;
  return (
    <div
      className={[styles.wrap, maxHeight && styles.scroll].filter(Boolean).join(" ")}
      data-component="data-table"
      style={maxHeight ? { maxHeight } : undefined}
      onFocus={maxHeight ? (e) => reveal(e.target) : undefined}
    >
      <table className={[styles.table, compact && styles.compact].filter(Boolean).join(" ")} aria-label={label}>
        <thead>
          <tr>
            {selection && (
              <th scope="col" className={styles.check}>
                <span className={styles.hidden}>Pick</span>
              </th>
            )}
            {columns.map((c) => (
              <th key={c.key} scope="col" className={cls(c)} style={c.width ? { width: c.width } : undefined}>
                {c.header}
              </th>
            ))}
          </tr>
        </thead>
        <tbody>
          {rows.map((r) => {
            const g = group?.(r) ?? null;
            const head = g !== null && g !== last;
            last = g;
            const on = selection?.selected(r) ?? false;
            const mixed = !on && (selection?.mixed?.(r) ?? false);
            return (
              <Fragment key={rowKey(r)}>
                {head && (
                  <tr className={styles.group}>
                    <th scope="colgroup" colSpan={span}>
                      {g}
                    </th>
                  </tr>
                )}
                <tr className={on ? styles.on : undefined}>
                  {selection && (
                    <td className={styles.check}>
                      <input
                        type="checkbox"
                        aria-label={selection.label(r)}
                        checked={on}
                        ref={(el) => {
                          if (el) el.indeterminate = mixed;
                        }}
                        onChange={(e) => {
                          selection.onChange(r, e.target.checked);
                          if (maxHeight) reveal(e.target);
                        }}
                      />
                    </td>
                  )}
                  {columns.map((c) =>
                    c.rowHeader ? (
                      <th key={c.key} scope="row" className={cls(c)}>
                        {c.cell(r)}
                      </th>
                    ) : (
                      <td key={c.key} className={cls(c)}>
                        {c.cell(r)}
                      </td>
                    ),
                  )}
                </tr>
              </Fragment>
            );
          })}
        </tbody>
      </table>
    </div>
  );
}
