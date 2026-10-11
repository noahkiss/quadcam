// Pages under Gear in the sidebar, after Connected and Devices. Later packages add theirs
// here (spec 2.1): Bench (WP5), Aircraft and Radios, Sims (WP8), Firmware (WP10),
// Storage (WP4).
import type { GearPageSlot } from "./slots";
import { ControlsPage } from "./Switches/ControlsPage";
import { FlightsPage } from "./Flights/FlightsPage";
import { PacksPage } from "./Flights/PacksPage";
import { PackUpPage } from "./Flights/PackUpPage";
import { RepairsPage } from "./Flights/RepairsPage";
import { BenchPage } from "./Bench/BenchPage";
import { StoragePage } from "./Storage/StoragePage";
import { FirmwarePage } from "./Firmware/FirmwarePage";
import { CalibrationPage } from "./Sim/CalibrationPage";
import { SimPage } from "./Sim/SimPage";
import { SimsPage } from "./Sims/SimsPage";
import { VoicesPage } from "./Voice/VoicesPage";

export const GEAR_PAGES: GearPageSlot[] = [
  // The changes staged for each device, in the order the next session applies them (WP5).
  { id: "bench", label: "Bench", icon: "sliders", count: (s) => s.changes.length, render: () => <BenchPage /> },
  // The radio in USB Joystick mode, live (WP6).
  { id: "controls", label: "Controls", icon: "radio", render: () => <ControlsPage /> },
  // The voice packs on this Mac and the Voice studio; a radio's Voice segment picks one.
  { id: "voices", label: "Voices", icon: "volume-loud", render: () => <VoicesPage /> },
  // Fly the sim in the plain room (S5).
  { id: "sim", label: "Sim", icon: "quad", preview: "simPreview", render: () => <SimPage /> },
  // The sims on this Mac against the quad's rates; the badge counts the ones that differ (WP8).
  { id: "sims", label: "Sims", icon: "play", badge: (s) => ((s.gear?.sims_out_of_date ?? 0) > 0 ? "Out of date" : null), render: () => <SimsPage /> },
  // The sim's radio calibration (S4).
  { id: "sim-radio", label: "Sim radio", icon: "radio", render: () => <CalibrationPage /> },
  { id: "packup", label: "Pack up", icon: "check-circle", render: () => <PackUpPage /> },
  { id: "flights", label: "Flights", icon: "stopwatch", render: () => <FlightsPage /> },
  { id: "packs", label: "Packs", icon: "battery", render: () => <PacksPage /> },
  { id: "repairs", label: "Repairs", icon: "danger-triangle", render: () => <RepairsPage /> },
  // Installed against newest firmware, and the EdgeTX flash (WP10).
  { id: "firmware", label: "Firmware", icon: "upload-minimalistic", count: (s) => s.firmware?.devices.filter((d) => d.state === "update").length ?? 0, render: () => <FirmwarePage /> },
  { id: "storage", label: "Storage", icon: "hdd", render: () => <StoragePage /> },
];
