// The mock core's blackbox pulls: `gear_blackbox_pull`, `gear_blackbox` and
// `gear_blackbox_erase`, written by hand from `core/blackbox.rs`. One FC flash with a few
// logs; a pull stores it, and erases it only when the `gearEraseBlackbox` setting is on.
import type { BlackboxEntry, BlackboxErased, BlackboxPullResult, Connected, FcJob, Pull } from "../types";
import { FC } from "./gear";

const TOTAL = 16_777_216;
const NOTE = "Guess: the FC has no clock, so logs are paired with the radio's flights by order, newest with newest. Test arms under 32 KB are skipped. Check the sizes before you trust a pair.";

export interface MockBlackbox {
  pulls: BlackboxEntry[];
  /** Bytes used in the FC's flash, and how many logs they hold. */
  used: number;
  logs: number;
}

export const freshBlackbox = (): MockBlackbox => ({ pulls: [], used: 4_100_000, logs: 3 });

const stamp = (t: Date) => t.toISOString().slice(0, 19).replace(/[-:]/g, "").replace("T", "T").replace(/^(\d{4})(\d{2})(\d{2})/, "$1-$2-$3");

function fcOf(connected: Connected[]): Connected {
  const fc = connected.find((c) => c.kind === "fc" && c.link.kind === "serial");
  if (!fc) throw "No device: no FC found. Plug USB in before the battery.";
  return fc;
}

export function list(b: MockBlackbox, device: string | null): BlackboxEntry[] {
  return structuredClone([...b.pulls].reverse().filter((e) => !device || e.pull.device === device));
}

export function pull(b: MockBlackbox, connected: Connected[], values: Record<string, unknown>, keep: boolean, now: Date): FcJob<BlackboxPullResult> {
  const fc = fcOf(connected);
  const device = fc.id || FC.id;
  const result: BlackboxPullResult = { pull: null, new: false, method: null, read_bytes: 0, read_secs: 0, erase: "off", erase_note: null, erase_secs: null, notes: [] };
  if (b.used === 0) {
    result.notes.push("The blackbox flash is empty; nothing to pull.");
    return { result, notes: [] };
  }
  const t = now.toISOString();
  const size = Math.floor(b.used / b.logs);
  const p: Pull = {
    id: `${device}/${stamp(now)}`,
    device,
    aircraft: FC.aircraft,
    pulled_at: t,
    day: t.slice(0, 10),
    method: "msp",
    blob: { xxh64: "00000000000000a1", size: b.used },
    used: b.used,
    total: TOTAL,
    logs: Array.from({ length: b.logs }, (_, i) => ({ index: i + 1, offset: i * size, size, firmware: "Betaflight 4.5.1", craft: "Whoop", start: "0000-01-01T00:00:00.000+00:00", dated: false, looptime_us: 125, headers: 9 })),
    firmware: "Betaflight 4.5.1",
    craft: "Whoop",
    erased: false,
    erase_note: null,
  };
  result.pull = p;
  result.new = true;
  result.method = "msp";
  result.read_bytes = b.used;
  result.read_secs = b.used / 84_000;
  if (values.gearEraseBlackbox === true && !keep) {
    p.erased = true;
    b.used = 0;
    b.logs = 0;
    result.erase = "done";
    result.erase_secs = 12;
  }
  b.pulls.push({ pull: structuredClone(p), flights: { links: [], unpaired_logs: p.logs.length, unpaired_flights: 0, short_logs: 0, note: NOTE } });
  return { result, notes: [] };
}

export function erase(b: MockBlackbox, connected: Connected[], confirm: boolean): FcJob<BlackboxErased> {
  if (!confirm) throw "Refused: erasing the blackbox deletes the FC's logs for good. Pass confirm=true to go ahead.";
  fcOf(connected);
  const latest = b.pulls[b.pulls.length - 1];
  if (b.used === 0) throw "The blackbox flash is already empty.";
  if (!latest || latest.pull.used !== b.used) throw `Refused: the flash holds ${b.used} bytes and QuadCam has no stored pull of exactly that. Pull first.`;
  latest.pull.erased = true;
  b.used = 0;
  b.logs = 0;
  return { result: { pull: latest.pull.id, secs: 12 }, notes: [] };
}
