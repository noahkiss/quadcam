// The radio's Models segment (design 6.3, WP9): timers, telemetry screens, logging, alarms and
// callouts of one model. Each change stages through `gear_model_edit` into the radio's one
// "Model edits" change; nothing reaches the card until the apply sheet writes it.
import { Banner } from "../../../components/Banner";
import type { DeviceRef } from "../slots";
import { CalloutsEditor } from "./CalloutsEditor";
import { LoggingEditor } from "./LoggingEditor";
import { ModelPicker } from "./ModelPicker";
import { ScreensEditor } from "./ScreensEditor";
import { TimersEditor } from "./TimersEditor";
import { useModel } from "./useModel";
import styles from "./Models.module.css";

export function ModelsSegment({ d }: { d: DeviceRef }) {
  const device = d.device?.id ?? null;
  const m = useModel(device);
  return (
    <div className={styles.segment}>
      {!device && <p className={styles.muted}>Save this radio from Overview to edit its models.</p>}
      {m.error && (
        <Banner kind="error" icon="danger-triangle">
          {m.error}
        </Banner>
      )}
      {m.note && <p className={styles.note}>{m.note}</p>}
      {m.detail && (
        <>
          <ModelPicker detail={m.detail} file={m.file} pick={m.pick} />
          <TimersEditor detail={m.detail} stage={(ops) => m.stage(ops)} />
          <ScreensEditor detail={m.detail} stage={(ops) => m.stage(ops)} />
          <LoggingEditor detail={m.detail} stage={(ops) => m.stage(ops)} />
          <CalloutsEditor detail={m.detail} stage={(ops) => m.stage(ops)} />
        </>
      )}
    </div>
  );
}
