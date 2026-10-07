// Pages under Gear in the sidebar, after Connected and Devices. Later packages add theirs
// here (spec 2.1): Bench (WP5), Aircraft and Radios, Sims (WP8), Firmware (WP10),
// Storage (WP4).
import type { GearPageSlot } from "./slots";
import { FlightsPage } from "./Flights/FlightsPage";
import { PacksPage } from "./Flights/PacksPage";
import { PackUpPage } from "./Flights/PackUpPage";
import { RepairsPage } from "./Flights/RepairsPage";

export const GEAR_PAGES: GearPageSlot[] = [
  { id: "packup", label: "Pack up", icon: "check-circle", render: () => <PackUpPage /> },
  { id: "flights", label: "Flights", icon: "stopwatch", render: () => <FlightsPage /> },
  { id: "packs", label: "Packs", icon: "battery", render: () => <PacksPage /> },
  { id: "repairs", label: "Repairs", icon: "danger-triangle", render: () => <RepairsPage /> },
];
