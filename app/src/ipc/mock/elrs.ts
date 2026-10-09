// The mock core's ExpressLRS tools (`core/elrs.rs`), a preview: read the device behind a
// saved radio or FC, stage options, plan and run a flash. Written by hand. The fake device
// answers the same options the core's `FakeElrs` does.
import type { ApplyPlan, ApplyReport, Check, Device, ElrsDeviceView, ElrsFlashParams, ElrsOption, ElrsReadParams, ElrsReadReport, ElrsSnapshot, ElrsView } from "../types";
import type { MockGear } from "./gear";

export interface MockElrs {
  /** Last read of each ELRS device, by device id. */
  snapshots: Record<string, ElrsSnapshot>;
  /** The next flash fails its hash check. */
  failNext: boolean;
  flashed: string[];
  /** Newest release the check found. */
  latest: string | null;
  /** The esptool module's installed version. */
  esptool: string | null;
}

export const freshElrs = (): MockElrs => ({ snapshots: {}, failNext: false, flashed: [], latest: "4.1.0", esptool: "5.4.0" });

const OFF = "ELRS tools (preview) are off. Turn them on in Settings > Gear, or set elrs_preview=true.";

const choice = (key: string, label: string, param: string, id: number, choices: string[], index: number): ElrsOption => ({ key, label, param, id, value: choices[index], choices, min: null, max: null });

const TX_OPTIONS = (): ElrsOption[] => [
  choice("packet_rate", "Packet rate", "Packet Rate", 1, ["50Hz(-115dBm)", "150Hz(-112dBm)", "250Hz(-108dBm)", "500Hz(-105dBm)"], 1),
  choice("telemetry_ratio", "Telemetry ratio", "Telem Ratio", 2, ["Std", "Off", "1:128", "1:64", "1:32", "1:16", "1:8", "1:4", "1:2"], 0),
  choice("power", "Max power", "Max Power", 6, ["10", "25", "50", "100", "250", "500", "1000"], 4),
  choice("dynamic_power", "Dynamic power", "Dynamic", 7, ["Off", "On", "AUX9", "AUX10", "AUX11", "AUX12"], 0),
  choice("switch_mode", "Switch mode", "Switch Mode", 3, ["Hybrid", "Wide"], 1),
  choice("model_match", "Model match", "Model Match", 4, ["Off", "On"], 0),
];
const RX_OPTIONS = (): ElrsOption[] => [choice("model_match", "Model match", "Model Match", 2, ["Off", "On"], 0)];

const idFor = (role: "tx" | "rx", host: string) => `elrs-${role}-${host.replace(/[^0-9a-f]/g, "").slice(0, 16).padEnd(16, "0")}`;

/** `gear_elrs_read`. */
export function read(st: MockElrs, g: MockGear, p: ElrsReadParams, values: Record<string, unknown>): ElrsReadReport {
  if (values.elrsPreview !== true) throw `Refused: ${OFF}`;
  const host = g.devices.find((d) => d.id === p.host);
  if (!host) throw `Unknown device "${p.host}". See \`gear devices\`.`;
  if (host.kind !== "radio" && host.kind !== "fc") throw `${host.name} is a ${host.kind}. Pick the radio (for its internal module) or the FC (for its receiver).`;
  const tx = host.kind === "radio";
  const role = tx ? "tx" : "rx";
  const name = tx ? "RM Pocket 2.4GHz TX" : "Vendor 2.4GHz AIO RX";
  const version = tx ? "4.1.0" : "3.5.3";
  const id = idFor(role, host.id);
  const snapshot: ElrsSnapshot = {
    device: id,
    role,
    host: host.id,
    name,
    target: name,
    version,
    version_source: "parameters",
    options: tx ? TX_OPTIONS() : RX_OPTIONS(),
    params: [],
    read_at: "2026-10-09T12:00:00Z",
  };
  const prev = st.snapshots[id];
  if (prev) snapshot.options = prev.options;
  st.snapshots[id] = snapshot;
  if (!g.devices.some((d) => d.id === id)) {
    g.devices.push({ id, kind: tx ? "elrs_tx" : "elrs_rx", name: "", aircraft: null, identity: { firmware: "ExpressLRS", version, target: name }, last_seen: "2026-10-09T12:00:00Z", last_backup: null } as Device);
  }
  return {
    device: id,
    snapshot: structuredClone(snapshot),
    notes: [
      tx ? "The radio's module has its pulses off until the radio restarts: restart the radio when you are done." : "The FC passes its receiver UART (2) through until you unplug USB: unplug it and plug it in again when you are done.",
      "The version and the options come from the device's parameters. Reading a device this way has not been checked on a real one yet.",
    ],
  };
}

/** `gear_elrs`. */
export function view(st: MockElrs, g: MockGear, values: Record<string, unknown>): ElrsView {
  const devices: ElrsDeviceView[] = g.devices
    .filter((d) => (d.kind === "elrs_tx" || d.kind === "elrs_rx") && st.snapshots[d.id])
    .map((d) => ({
      snapshot: structuredClone(st.snapshots[d.id]),
      name: d.name || (d.kind === "elrs_tx" ? "Unnamed ELRS TX" : "Unnamed ELRS RX"),
      status: null,
      staged: g.changeStore.changes.filter((c) => c.device === d.id && ["draft", "ready", "try", "read_first"].includes(c.status)).length,
    }));
  const warnings: string[] = [];
  const tx = devices.find((d) => d.snapshot.role === "tx")?.snapshot.version;
  const rx = devices.find((d) => d.snapshot.role === "rx")?.snapshot.version;
  if (tx && rx && tx.split(".")[0] !== rx.split(".")[0]) warnings.push(`A transmitter on ExpressLRS ${tx.split(".")[0]} and a receiver on ${rx.split(".")[0]} do not link. Flash the receiver first, then the transmitter.`);
  return {
    preview: values.elrsPreview === true,
    esptool: st.esptool,
    latest: st.latest,
    hosts: g.devices.filter((d) => d.kind === "radio" || d.kind === "fc").map((d) => ({ device: d.id, name: d.name || d.id, kind: d.kind })),
    devices,
    phrase_set: !!values.elrsBindingPhrase,
    region: (values.elrsRegion as string | undefined) ?? "FCC",
    wifi_interval: (values.elrsWifiInterval as number | undefined) ?? 60,
    warnings,
  };
}

const ok = (name: string): Check => ({ name, ok: true });
const fail = (name: string, code: string, reason: string): Check => ({ name, ok: false, refusal: { code: code as never, reason } });

/** `gear_elrs_flash_plan`. */
export function flashPlan(st: MockElrs, g: MockGear, p: ElrsFlashParams, values: Record<string, unknown>): ApplyPlan {
  if (values.elrsPreview !== true) throw `Refused: ${OFF}`;
  const d = g.devices.find((x) => x.id === p.device);
  const snap = st.snapshots[p.device];
  if (!d || !snap) throw "Read this device first (`gear elrs read`): its target name decides the image.";
  const version = (p.version ?? st.latest ?? "").replace(/^v/i, "");
  if (!version) throw "Name the version to flash (the newest release is not known yet; check firmware first).";
  const checks: Check[] = [ok("ELRS tools (preview) are on")];
  checks.push(st.esptool ? ok("The esptool module is installed") : fail("The esptool module is installed", "disabled", "Install esptool in Settings > Modules (quadcam-cli modules install esptool)."));
  checks.push(values.elrsBindingPhrase ? ok("A binding phrase is set") : fail("A binding phrase is set", "bad_setting", "Set the binding phrase first (quadcam-cli settings set elrs_binding_phrase=...). A device flashed without one would not bind to your others."));
  checks.push(ok("The radio or FC is plugged in on serial"));
  const known = snap.name === "RM Pocket 2.4GHz TX" || snap.name === "Vendor 2.4GHz AIO RX";
  checks.push(known ? ok("A known target and version") : fail("A known target and version", "unknown_board", `This release has no target named \`${snap.name}\`.`));
  if (known) checks.push(ok("The release image"));
  const ready = checks.every((c) => c.ok);
  return {
    change: "elrs-flash",
    device: d.identity ?? {},
    checks,
    diff: [
      { kind: "version", label: "ExpressLRS firmware", before: d.identity?.version ?? null, after: version },
      ...(known ? [{ kind: "files" as const, label: "Firmware image", put: [`${snap.name} (Unified, ${values.elrsRegion ?? "FCC"})`, "firmware.bin at 0x0000 (537 KB, SHA-256 a8b8f597…)", "Binding: UID fingerprint 9f3a52c1 (the phrase is not shown)", "WiFi starts after 60 s without a link"], delete: [] }] : []),
    ],
    digest: ready ? `elrs-flash-${p.device}-${version}` : "",
    warnings: [
      "ExpressLRS publishes no SHA-256 for its releases. QuadCam recorded a8b8f597efd7ff6f at the download; compare it with a trusted copy, or plan again with sha256.",
      "QuadCam reads the chip's current flash with esptool first and keeps it as a backup. Flashing ExpressLRS this way has not been tried on a real device yet.",
      "ExpressLRS 4 wipes a receiver's Options page; note your settings first, and read the device again afterwards.",
    ],
  };
}

/** `gear_elrs_flash` and `gear_elrs_flash_click`. */
export function flash(st: MockElrs, g: MockGear, p: ElrsFlashParams, digest: string, confirm: boolean, values: Record<string, unknown>): ApplyReport {
  if (!confirm) throw "Refused: a flash needs the plan's digest and confirm=true.";
  const pl = flashPlan(st, g, p, values);
  const bad = pl.checks.find((c) => !c.ok);
  if (bad?.refusal) throw `Refused (${bad.refusal.code}): ${bad.refusal.reason}`;
  if (pl.digest !== digest) throw "Refused (before_mismatch): The release or the device changed since the plan; plan again.";
  const backup = `${p.device}/2026-10-09T120000-before_flash`;
  const base = { change: "elrs-flash", device: p.device, backup, after_backup: null, sent: [], failed_line: null, verify: [], files: [], notes: pl.warnings ?? [], at: "2026-10-09T12:00:00Z" };
  const first = { name: "Back up the current firmware", state: "done" as const, detail: `1024 KB, ${backup}` };
  if (st.failNext) {
    st.failNext = false;
    return { ...base, status: "failed", saved: false, steps: [first, { name: "Write", state: "done" }, { name: "Verify", state: "failed", detail: "esptool finished but did not report its hash check" }], message: `esptool finished but did not report that it verified the flash. Read the device again before you trust it. The firmware it ran before is in the backup (${backup}).` };
  }
  st.flashed.push(p.device);
  const d = g.devices.find((x) => x.id === p.device);
  if (d) d.identity = { ...d.identity, version: (p.version ?? st.latest ?? "").replace(/^v/i, "") };
  return { ...base, status: "verified", saved: true, steps: [first, { name: "Write", state: "done", detail: "1 files" }, { name: "Verify", state: "done", detail: "esptool: hash of data verified" }], message: `Verified: the device holds ExpressLRS ${p.version ?? st.latest} (esptool compared the flash with the image). Then read it again.` };
}

/** The options a staged change sets, resolved against the last read; throws as the core does. */
export function resolve(st: MockElrs, device: string, sets: { option: string; value: string }[]): { option: ElrsOption; to: string }[] {
  const snap = st.snapshots[device];
  if (!snap) throw "Read this device first (`gear elrs read`): its options are not known yet.";
  return sets.map((s) => {
    const key = s.option.trim().toLowerCase().replace(/[ -]/g, "_");
    if (key.includes("phrase") || key === "uid" || key.includes("bind")) throw "QuadCam does not stage the binding phrase. Set it as the elrs_binding_phrase setting; a flash plan then shows only its hash.";
    const o = snap.options.find((x) => x.key === key);
    if (!o) throw `The device has no ${key} parameter. Read it again, or it does not offer this option.`;
    const norm = (t: string) => t.split("(")[0].replace(/\s/g, "").toLowerCase().replace(/mw$/, "");
    const hit = o.choices.filter((c) => norm(c) === norm(s.value));
    if (hit.length !== 1) throw `\`${s.value}\` is none of: ${o.choices.join(", ")}.`;
    return { option: o, to: hit[0] };
  });
}
