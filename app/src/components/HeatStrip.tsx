import type { Moment } from "../ipc/types";
import { heat, heatGradient } from "../lib/moments";
import styles from "./HeatStrip.module.css";

/** A thin strip that is stronger where a clip's moments are. Nothing when it has none. */
export function HeatStrip({ moments, duration, bins = 48, className }: { moments: Moment[]; duration: number; bins?: number; className?: string }) {
  const bg = heatGradient(heat(moments, duration, bins));
  if (!bg) return null;
  return <span className={[styles.heat, className].filter(Boolean).join(" ")} style={{ backgroundImage: bg }} data-component="heat-strip" aria-hidden="true" />;
}
