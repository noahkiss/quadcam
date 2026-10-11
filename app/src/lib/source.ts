// Where a dump view (Rates, OSD) was read from, as a person reads it: a backup by its time,
// a file by its name.
import { backupTime, fmtWhen } from "./backups";

/** "From backup · Oct 5, 2026, 10:00 AM", or the file names, then the firmware line. */
export function sourceText(source: string[], firmware?: string | null): string {
  const from = source.map((s) => {
    const t = backupTime(s);
    return t ? `From backup · ${fmtWhen(t)}` : s;
  });
  return [from.join(", "), firmware].filter(Boolean).join(" · ");
}
