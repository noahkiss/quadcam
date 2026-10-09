// The mock core's firmware (`Core::gear_firmware`, `gear_splash`, `gear_flash_plan` and
// `gear_flash`): the check reads "the network" only when asked and remembers its answer,
// the flash plan runs the core's guards on the mock's devices, and a flash keeps the
// radio's old firmware as a backup. Written by hand from `core/firmware.rs`.
import type { ApplyPlan, ApplyReport, Check, Device, FirmwareRead, FirmwareStatus, FirmwareView, FlashParams, SplashParams, SplashPreview } from "../types";

export interface MockFirmware {
  /** What the releases say now. */
  releases: { edgetx: string | null; betaflight: string | null; elrs: string | null };
  /** The saved answer; null before the first check. */
  seen: FirmwareView["latest"];
  /** The setting. */
  mode: "manual" | "daily";
  /** STM32 DFU devices plugged in. */
  dfu: number;
  /** Sources that fail the next check. */
  failing: string[];
  /** The EdgeTX version the radio's image names (`gear_firmware_read`). */
  readVersion: string;
  /** Reads made, for specs. */
  reads: number;
  /** The next flash fails its read back. */
  failNext: boolean;
  /** Network checks and flashes made, for specs. */
  checks: number;
  flashed: string[];
}

export const freshFirmware = (): MockFirmware => ({
  releases: { edgetx: "2.12.4", betaflight: "2026.6.0", elrs: "3.5.3" },
  seen: {},
  mode: "manual",
  dfu: 1,
  readVersion: "2.12.4",
  reads: 0,
  failing: [],
  failNext: false,
  checks: 0,
  flashed: [],
});

const idOf = (d: Device) => d.identity ?? {};
const ver = (v: string) => v.replace(/^v/i, "").split(/[-+]/)[0].split(".").map((n) => Number(n) || 0);
function cmp(a: string, b: string) {
  const [x, y] = [ver(a), ver(b)];
  for (let i = 0; i < Math.max(x.length, y.length); i++) if ((x[i] ?? 0) !== (y[i] ?? 0)) return (x[i] ?? 0) < (y[i] ?? 0) ? -1 : 1;
  return 0;
}

/** `compat::PROVEN` for EdgeTX: the Pocket on 2.12; the splash on 2.12.4. */
const prefix = (v: string, p: string) => v === p || v.startsWith(`${p}.`) || v.startsWith(`${p}-`);
function compat(product: "edgetx" | "splash", board: string | null, version: string | null): { code: string; reason: string } | null {
  const label = product === "edgetx" ? "EdgeTX" : "The splash of EdgeTX";
  if (!version) return { code: "unknown_version", reason: `${label} did not report its version; QuadCam reads it but does not write it.` };
  if (board?.toLowerCase() !== "pocket") return { code: "unknown_board", reason: `Board ${board ?? "(none)"} is not proven.` };
  const proven = product === "edgetx" ? prefix(version, "2.12") : prefix(version, "2.12.4");
  return proven ? null : { code: "unknown_version", reason: `${label} ${version} on board ${board} is not proven; QuadCam reads it but does not write it.` };
}

/** `gear_firmware`: `check` true reads the releases, false the saved answer, null follows the setting. */
export function view(st: MockFirmware, devices: Device[], check: boolean | null, now: string): FirmwareView {
  const due = st.mode === "daily" && !st.seen.checked_at;
  if (check ?? due) {
    st.checks += 1;
    const errors = st.failing.map((f) => `${f}: The download failed: no fixture`);
    const keep = (name: string, v: string | null) => (st.failing.includes(name) ? st.seen[name === "EdgeTX" ? "edgetx" : name === "Betaflight" ? "betaflight" : "elrs"] ?? null : v);
    st.seen = { edgetx: keep("EdgeTX", st.releases.edgetx), betaflight: keep("Betaflight", st.releases.betaflight), elrs: keep("ExpressLRS", st.releases.elrs), checked_at: now, errors };
  }
  const latest = st.seen;
  const rows: FirmwareStatus[] = [];
  for (const d of devices) {
    const product = d.kind === "fc" ? "Betaflight" : d.kind === "radio" ? "EdgeTX" : d.kind === "elrs_tx" || d.kind === "elrs_rx" ? "ExpressLRS" : null;
    if (!product) continue;
    const newest = (d.kind === "fc" ? latest.betaflight : d.kind === "radio" ? latest.edgetx : latest.elrs) ?? null;
    const installed = idOf(d).version ?? null;
    let state: FirmwareStatus["state"] = "unknown";
    let note: string | null = null;
    if (!installed) note = "The device reported no version.";
    else if (newest) {
      const c = cmp(installed, newest);
      state = c < 0 ? "update" : "up_to_date";
      if (c > 0) note = "This version is newer than the newest release.";
    }
    let flashable = false;
    if (d.kind === "radio" && newest) {
      const r = compat("edgetx", idOf(d).board ?? null, newest);
      if (!r) flashable = true;
      else if (state === "update") note = `QuadCam will not flash it: ${r.reason}`;
    } else if (state === "update") note = `QuadCam checks ${product} versions. It does not flash them.`;
    rows.push({ device: d.id, kind: d.kind, name: d.name || `Unnamed ${d.kind === "radio" ? "Radio" : d.kind === "fc" ? "FC" : "ELRS"}`, product, board: idOf(d).board ?? null, installed, latest: newest, state, flashable, note });
  }
  return { devices: rows, latest: structuredClone(latest), mode: st.mode };
}

// A 1 x 1 white PNG: the mock does not draw pictures.
const PIXEL = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg==";
const MONO = ["pocket", "boxer", "zorro", "tx12", "mt12", "t12", "t14", "t8", "tlite", "tpro", "x7", "xlite"];

/** `gear_splash`. */
export function splash(p: SplashParams): SplashPreview {
  if (!p.image.toLowerCase().endsWith(".png")) throw `Not a PNG image: ${p.image}`;
  const threshold = p.threshold ?? 128;
  const board = p.board?.trim() || null;
  const supported = !board || MONO.includes(board.toLowerCase());
  const dark = Math.round(((p.invert ? threshold : 255 - threshold) / 255) * 8192);
  return {
    width: 128,
    height: 64,
    threshold,
    invert: !!p.invert,
    board: p.board ?? null,
    dark,
    png_base64: PIXEL,
    supported,
    reason: supported ? null : `This radio's splash format is not supported yet (board ${board}).`,
    hash: `h${threshold}${p.invert ? "i" : "n"}`,
  };
}

const ok = (name: string): Check => ({ name, ok: true });
const fail = (name: string, code: string, reason: string): Check => ({ name, ok: false, refusal: { code: code as never, reason } });
const refused = (code: string, reason: string) => `Refused (${code}): ${reason}`;

/** `gear_firmware_read`: the read-only trial. `st.readVersion` is the version the radio's image names. */
export function read(st: MockFirmware, devices: Device[], device: string | null): FirmwareRead {
  if (st.dfu === 0) throw refused("no_device", "No radio is in DFU mode. Turn the radio off, then plug in the USB cable (do not hold the trim buttons, which start the EdgeTX bootloader instead). Wait a few seconds and read again.");
  if (st.dfu > 1) throw refused("several_devices", `${st.dfu} radios are in DFU mode; unplug all but one.`);
  const d = device ? devices.find((x) => x.id === device) : undefined;
  if (device && !d) throw `No saved device ${device}.`;
  const known = d ? idOf(d).version ?? null : null;
  const matches = known ? known.replace(/^v/i, "") === st.readVersion : null;
  const id = `${d?.id ?? "dfu-0001"}/2026-10-09T120000-read`;
  const message = matches === null ? `The firmware names EdgeTX ${st.readVersion}. QuadCam has no version to compare it with. The copy is saved.` : matches ? `The radio's firmware is EdgeTX ${st.readVersion}, the version QuadCam knows for it. The copy is saved.` : `The firmware names EdgeTX ${st.readVersion}, but QuadCam knows the radio as ${known}. Read the version in the radio's own About screen. The copy is saved.`;
  st.reads += 1;
  return {
    copy: { id, device: d?.id ?? "dfu-0001", taken_at: "2026-10-09T12:00:00Z", kind: "read", size: 519580, sha256: "7d1c0f5e".repeat(8), image_board: "pocket", image_version: st.readVersion },
    device: d?.id ?? null,
    known_version: known,
    flash_bytes: 1048576,
    matches,
    message,
    steps: [
      { name: "Open the DFU device", state: "done", detail: "1024 KB of flash" },
      { name: "Read the flash twice", state: "done", detail: "both reads equal" },
      { name: "Save the copy", state: "done", detail: `507 KB, ${id}` },
    ],
  };
}

/** `gear_flash_plan`. */
export function plan(st: MockFirmware, devices: Device[], p: FlashParams): ApplyPlan {
  const d = devices.find((x) => x.id === p.device);
  if (!d) throw `No saved device ${p.device}.`;
  if (d.kind !== "radio") throw refused("incompatible", "QuadCam flashes the firmware of EdgeTX radios only.");
  const target = (p.version ?? idOf(d).version ?? "").replace(/^v/i, "");
  if (!target) throw "The radio reports no version; name the version to flash.";
  const board = idOf(d).board ?? null;
  const checks: Check[] = [];
  const known = compat("edgetx", board, target);
  checks.push(known ? fail("Known board and version", known.code, known.reason) : ok("Known board and version"));
  const diff: ApplyPlan["diff"] = [{ kind: "version", label: "EdgeTX firmware", before: idOf(d).version ?? null, after: target }];
  let splashOk = true;
  if (p.splash) {
    const sp = splash(p.splash);
    const r = !sp.supported ? { code: "unknown_board", reason: sp.reason ?? "" } : compat("splash", board, target);
    checks.push(r ? fail("Splash layout", r.code, r.reason) : ok("Splash layout"));
    splashOk = !r;
  }
  if (!known && splashOk) {
    checks.push(ok("Firmware image"));
    const put = [`pocket-def35ad.bin (507 KB)`, "SHA-256 7d1c0f5e…"];
    if (p.splash) {
      checks.push(ok("Splash markers"));
      put.push(`Splash: ${splash(p.splash).dark} dark pixels of 8192`);
    }
    diff.push({ kind: "files", label: "Firmware image", put, delete: [] });
  }
  checks.push(d.last_backup ? ok("Card backup") : fail("Card backup", "no_backup", "Back up this radio first: connect it in USB Storage mode and back it up. A flash can change how the radio reads its card."));
  checks.push(st.dfu === 1 ? ok("One radio in DFU mode") : st.dfu === 0 ? fail("One radio in DFU mode", "no_device", "No radio is in DFU mode. Turn the radio off, then plug in the USB cable (do not hold the trim buttons, which start the EdgeTX bootloader instead). Wait a few seconds.") : fail("One radio in DFU mode", "several_devices", `${st.dfu} radios are in DFU mode; unplug all but one.`));
  const ready = checks.every((c) => c.ok);
  return {
    change: "flash",
    device: idOf(d),
    checks,
    diff,
    digest: ready ? `flash-${d.id}-${target}-${p.splash ? splash(p.splash).hash : "none"}` : "",
    warnings: ["A radio in DFU mode shows no name. QuadCam writes the one radio in DFU mode; check it is the radio you picked.", "QuadCam reads the radio's current firmware over DFU twice first and keeps it as a copy; it erases nothing until both reads agree and the saved file reads back. If a flash fails, the radio's ROM bootloader still answers over USB (turn it off, plug in USB) and you can flash again. Flashing has not been tried on a real radio yet."],
  };
}

/** `gear_flash` and `gear_flash_click`. */
export function flash(st: MockFirmware, devices: Device[], p: FlashParams, digest: string, confirm: boolean): ApplyReport {
  if (!confirm) throw "Refused: a flash needs the plan's digest and confirm=true.";
  const pl = plan(st, devices, p);
  const bad = pl.checks.find((c) => !c.ok);
  if (bad?.refusal) throw refused(bad.refusal.code, bad.refusal.reason);
  if (pl.digest !== digest) throw refused("before_mismatch", "The firmware or the radio changed since the plan; plan again.");
  const backup = `${p.device}/2026-10-09T120000-before-flash`;
  const base = { change: "flash", device: p.device, backup, after_backup: null, sent: [], failed_line: null, verify: [], files: [], notes: pl.warnings ?? [], at: "2026-10-09T12:00:00Z" };
  const first = { name: "Copy the current firmware", state: "done" as const, detail: `600 KB, read twice, saved as ${backup}` };
  if (st.failNext) {
    st.failNext = false;
    return {
      ...base,
      status: "failed",
      saved: false,
      steps: [first, { name: "Erase", state: "done", detail: "8 sectors" }, { name: "Write", state: "failed", detail: "The device holds different bytes than were written (first difference at 0x08001800, found while writing)." }, { name: "Read back", state: "skipped" }, { name: "Leave DFU", state: "skipped" }],
      message: `The flash failed: The device holds different bytes than were written. The radio stays in DFU mode. Flash again, or unplug it, turn it off, plug it in again and retry. The radio's ROM bootloader always answers over USB. The firmware it ran before is kept (${backup}).`,
    };
  }
  st.flashed.push(p.device);
  return {
    ...base,
    status: "verified",
    saved: true,
    steps: [first, { name: "Erase", state: "done", detail: "8 sectors" }, { name: "Write", state: "done", detail: "600 KB" }, { name: "Read back", state: "done", detail: "600 KB, equal" }, { name: "Leave DFU", state: "done" }],
    message: "Verified: the radio holds the new firmware byte for byte and restarted. Connect it in USB Storage mode to check the version.",
  };
}
