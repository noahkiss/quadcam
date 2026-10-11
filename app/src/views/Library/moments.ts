import type { IconName } from "../../components/Icon";
import type { Moment, MomentKind } from "../../ipc/types";

export const KIND: Record<MomentKind, string> = { roll: "Roll", flip: "Flip", punch: "Punch-out", dive: "Dive", crash: "Crash?", dead_air: "Dead air", highlight: "Highlight" };
export const KIND_ICON: Record<MomentKind, IconName> = { roll: "roll", flip: "flip", punch: "bolt", dive: "dive", crash: "danger-triangle", dead_air: "close-circle", highlight: "sparkle" };
/** Where a moment's evidence came from. */
export const SOURCE: Record<Moment["source"], string> = { radio_log: "Radio log", video: "Video signal", video_analysis: "Video analysis" };
