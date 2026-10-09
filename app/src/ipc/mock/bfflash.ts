// The mock core's Betaflight flash (preview), written by hand from `core/bf_flash.rs`: the
// Firmware page's FC row, the plan's guards and the flash report. The data is `compat::PROVEN`
// and `betaflight::TARGETS`; the mock never talks to a build service.
import type { ApplyPlan, ApplyReport, Check, Device, FirmwareStatus, FlashParams } from "../types";

/** Boards QuadCam can flash, with the release `compat` proves on each. */
const PROVEN: Record<string, string> = { betafpvg473: "2025.12.5", betafpvg473_v2: "2026.6.0" };

const release = (v: string) => v.replace(/^[vV]/, "").split("-")[0];
const ok = (name: string): Check => ({ name, ok: true });
const fail = (name: string, code: string, reason: string): Check => ({ name, ok: false, refusal: { code: code as never, reason } });

/** `compat::check_writable` for a flash: the board must have a target, the release must be proven on it. */
export function compat(board: string | null, version: string | null): { code: string; reason: string } | null {
  const b = board?.toLowerCase() ?? "";
  if (!(b in PROVEN)) return { code: "unknown_board", reason: `Board ${board ?? "(none)"} cannot be flashed by QuadCam yet.` };
  if (!version || release(version) !== PROVEN[b]) return { code: "unknown_version", reason: `Betaflight ${version ?? "(none)"} on board ${board} is not proven; QuadCam reads it but does not write it.` };
  return null;
}

/** `Core::gear_firmware`: with the preview on, an FC on a proven pair is flashable. */
export function adjust(rows: FirmwareStatus[], preview: boolean): FirmwareStatus[] {
  if (!preview) return rows;
  return rows.map((r) => {
    if (r.kind !== "fc") return r;
    const bad = r.latest ? compat(r.board ?? null, r.latest) : { code: "unknown_version", reason: "The newest release is not known." };
    if (!bad) return { ...r, flashable: true, note: r.state === "update" ? null : r.note };
    return { ...r, flashable: false, note: r.state === "update" ? `QuadCam will not flash it: ${bad.reason}` : r.note };
  });
}

/** `Core::gear_flash_plan` for an FC. */
export function plan(d: Device, p: FlashParams, preview: boolean, dfuAttached: number): ApplyPlan {
  const id = d.identity ?? {};
  const target = release(p.version ?? id.version ?? "");
  const checks: Check[] = [];
  checks.push(preview ? ok("Betaflight flashing (preview) is on") : fail("Betaflight flashing (preview) is on", "disabled", "Betaflight flashing is a preview and is off. Turn on Betaflight flashing (preview) in Settings > Gear, or set betaflight_flash_preview=true."));
  checks.push(ok("One FC plugged in"), ok("The FC is this device"));
  const known = compat(id.board ?? null, target);
  checks.push(known ? fail("Known board and version", known.code, known.reason) : ok("Known board and version"));
  const diff: ApplyPlan["diff"] = [{ kind: "version", label: "Betaflight firmware", before: id.version ?? null, after: target }];
  if (!known && preview) {
    checks.push(ok("Firmware image"));
    diff.push({ kind: "files", label: "Firmware image", put: [`betaflight_${target}_${(id.board ?? "").toUpperCase()}.hex (300 KB)`, "SHA-256 5b0e91c4…", "STM32G473 at 0x08000000"], delete: [] });
  }
  checks.push(ok("Port free"));
  checks.push(dfuAttached === 0 ? ok("No DFU device attached") : fail("No DFU device attached", "several_devices", `${dfuAttached} DFU device is already attached. Unplug it: QuadCam finds the FC in DFU mode as the one new device.`));
  checks.push(ok("USB heat"));
  return {
    change: "flash",
    device: id,
    checks,
    diff,
    digest: checks.every((c) => c.ok) ? `bfflash-${d.id}-${target}` : "",
    warnings: [
      "QuadCam saves `diff all` and `dump all` first, flashes over DFU, reads the flash back, and puts your settings back through the FC apply. A setting the new version no longer has is listed in the report and skipped. Board lines (resources, serial ports) are not carried over.",
      "If the FC does not start: unplug USB and the battery, hold the FC's boot button, plug USB in, and flash an official build from Betaflight Configurator. The ROM bootloader cannot be overwritten, so the FC can always be flashed again.",
      "Betaflight flashing is a preview. It has not been tried on a real FC yet.",
    ],
  };
}

/** `Core::gear_flash` for an FC. */
export function flash(d: Device, p: FlashParams, pl: ApplyPlan, failNext: boolean): ApplyReport {
  const backup = `${d.id}/2026-10-09T120000-before_flash`;
  const base = { change: "flash", device: d.id, backup, after_backup: null, sent: [], failed_line: null, verify: [], files: [], notes: [] as string[], at: "2026-10-09T12:00:00Z" };
  const steps = [
    { name: "Back up", state: "done" as const, detail: `diff all and dump all, ${backup}` },
    { name: "Restart into the bootloader", state: "done" as const, detail: "DFU device FCCHIP" },
    { name: "Copy the current firmware", state: "done" as const, detail: `300 KB, read twice, saved as ${d.id}/2026-10-09T120000-before-flash` },
  ];
  if (failNext) {
    return {
      ...base,
      status: "failed",
      saved: false,
      steps: [...steps, { name: "Erase", state: "done", detail: "150 sectors" }, { name: "Write", state: "done", detail: "300 KB" }, { name: "Read back", state: "failed", detail: "The device holds different bytes than were written (first difference at 0x08001800)." }, { name: "Leave DFU", state: "skipped" }],
      message: `The flash failed: The device holds different bytes than were written. The FC has half a firmware and stays in its bootloader: run the flash again, or unplug and re-enter the bootloader first. If the FC does not start: unplug USB and the battery, hold the FC's boot button, plug USB in, and flash an official build from Betaflight Configurator. Your settings are in backup ${backup}.`,
    };
  }
  return {
    ...base,
    status: "verified",
    saved: true,
    notes: [...(pl.warnings ?? []).slice(0, 1), "Skipped, not in this Betaflight version (1): legacy_thing."],
    steps: [
      ...steps,
      { name: "Erase", state: "done", detail: "150 sectors" },
      { name: "Write", state: "done", detail: "300 KB" },
      { name: "Read back", state: "done", detail: "300 KB, equal" },
      { name: "Leave DFU", state: "done" },
      { name: "Restart", state: "done", detail: d.identity?.version ?? null },
      { name: "Read new settings", state: "done", detail: "2 to carry over, 1 skipped" },
      { name: "Re-apply settings", state: "done", detail: "3 lines, change c-1" },
    ],
    message: `Verified: ${d.name || "The FC"} runs ${release(p.version ?? "")}, the flash matches byte for byte, and its settings read back as saved.`,
  };
}
