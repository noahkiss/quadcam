// Pages under Gear in the sidebar, after Connected and Devices. Later packages add theirs
// here (spec 2.1): Bench (WP5), Aircraft and Radios, Sims (WP8), Firmware (WP10),
// Storage (WP4).
import type { GearPageSlot } from "./slots";
import { ControlsPage } from "./Switches/ControlsPage";
import { FlightsPage } from "./Flights/FlightsPage";
import { PacksPage } from "./Flights/PacksPage";
import { PackUpPage } from "./Flights/PackUpPage";
import { RepairsPage } from "./Flights/RepairsPage";
import { StoragePage } from "./Storage/StoragePage";
import { CalibrationPage } from "./Sim/CalibrationPage";
import { SimPage } from "./Sim/SimPage";

export const GEAR_PAGES: GearPageSlot[] = [
  // The radio in USB Joystick mode, live (WP6).
  { id: "controls", label: "Controls", icon: "radio", render: () => <ControlsPage /> },
  // Fly the sim in the plain room (S5).
  { id: "sim", label: "Sim", icon: "quad", render: () => <SimPage /> },
  // The sim's radio calibration (S4).
  { id: "sim-radio", label: "Sim radio", icon: "radio", render: () => <CalibrationPage /> },
  { id: "packup", label: "Pack up", icon: "check-circle", render: () => <PackUpPage /> },
  { id: "flights", label: "Flights", icon: "stopwatch", render: () => <FlightsPage /> },
  { id: "packs", label: "Packs", icon: "battery", render: () => <PacksPage /> },
  { id: "repairs", label: "Repairs", icon: "danger-triangle", render: () => <RepairsPage /> },
  { id: "storage", label: "Storage", icon: "hdd", render: () => <StoragePage /> },
];
