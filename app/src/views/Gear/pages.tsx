// Pages under Gear in the sidebar, after Connected and Devices. Later packages add theirs
// here (spec 2.1): Bench (WP5), Aircraft and Radios, Packs and Flights (WP12), Sims (WP8),
// Firmware (WP10), Storage (WP4).
import type { GearPageSlot } from "./slots";
import { ControlsPage } from "./Switches/ControlsPage";
import { StoragePage } from "./Storage/StoragePage";

export const GEAR_PAGES: GearPageSlot[] = [
  // The radio in USB Joystick mode, live (WP6).
  { id: "controls", label: "Controls", icon: "radio", render: () => <ControlsPage /> },
  { id: "storage", label: "Storage", icon: "hdd", render: () => <StoragePage /> },
];
