import type { IconName } from "../../components/Icon";
import type { MomentKind } from "../../ipc/types";

export const KIND: Record<MomentKind, string> = { roll: "Roll", flip: "Flip", punch: "Punch-out", dive: "Dive", crash: "Crash?", dead_air: "Dead air" };
export const KIND_ICON: Record<MomentKind, IconName> = { roll: "roll", flip: "flip", punch: "bolt", dive: "dive", crash: "danger-triangle", dead_air: "close-circle" };
