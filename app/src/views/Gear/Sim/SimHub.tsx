// Gear > Sim: one page with four sections. Fly (the plain room, behind the `simPreview`
// setting), Radio (the sim's radio calibration), Rates (the sims on this Mac against the
// quad's rates) and Profiles (fit a sim profile from a quad's logs). Each section is its own
// Gear page id, so a link can open it directly.
import { SegmentedControl } from "../../../components/SegmentedControl";
import { useStore } from "../../../store";
import { CalibrationPage } from "./CalibrationPage";
import { ProfilesPage } from "./ProfilesPage";
import { SimPage } from "./SimPage";
import { SimsPage } from "../Sims/SimsPage";

export type SimSection = "fly" | "radio" | "rates" | "profiles";

const PAGE_ID: Record<SimSection, string> = { fly: "sim", radio: "sim-radio", rates: "sims", profiles: "sim-profiles" };

export function SimHub({ at }: { at: SimSection }) {
  const preview = useStore((s) => s.values.simPreview === true);
  const openGear = useStore((s) => s.openGear);
  const sections: { value: SimSection; label: string }[] = [...(preview ? [{ value: "fly" as const, label: "Fly" }] : []), { value: "radio", label: "Radio" }, { value: "rates", label: "Rates" }, { value: "profiles", label: "Profiles" }];
  const cur = at === "fly" && !preview ? "rates" : at;
  const tabs = <SegmentedControl label="Sim sections" value={cur} onChange={(v) => openGear({ page: "slot", id: PAGE_ID[v] })} segments={sections} />;
  return cur === "fly" ? <SimPage tabs={tabs} /> : cur === "radio" ? <CalibrationPage tabs={tabs} /> : cur === "profiles" ? <ProfilesPage tabs={tabs} /> : <SimsPage tabs={tabs} />;
}
