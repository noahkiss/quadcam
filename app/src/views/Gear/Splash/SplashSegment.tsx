// The radio's Splash segment (design 7.5): a PNG made into the radio's start-up picture,
// 128 x 64 and one bit, with a threshold and Invert. Flash with splash… opens the flash sheet
// with the picture patched into the board's EdgeTX binary. Nothing is written until Apply.
import { useEffect, useId, useState } from "react";
import { Banner } from "../../../components/Banner";
import { Button } from "../../../components/Button";
import { Checkbox, Field } from "../../../components/Field";
import { SectionCard } from "../../../components/dash/SectionCard";
import { api, errText, pickFiles } from "../../../ipc/api";
import type { SplashPreview } from "../../../ipc/types";
import { useStore } from "../../../store";
import { tilde } from "../../../lib/format";
import type { DeviceRef } from "../slots";
import styles from "./Splash.module.css";

export function SplashSegment({ d }: { d: DeviceRef }) {
  const home = useStore((s) => s.home);
  const openFlash = useStore((s) => s.openFlash);
  const [image, setImage] = useState<string | null>(null);
  const [threshold, setThreshold] = useState(128);
  const [invert, setInvert] = useState(false);
  const [preview, setPreview] = useState<SplashPreview | null>(null);
  const [error, setError] = useState<string | null>(null);
  const board = d.device?.identity?.board ?? d.connected?.identity?.board ?? null;

  useEffect(() => {
    if (!image) return;
    let gone = false;
    const t = setTimeout(() => {
      api.gearSplash({ image, threshold, invert, board }).then(
        (p) => {
          if (gone) return;
          setPreview(p);
          setError(null);
        },
        (e) => {
          if (gone) return;
          setPreview(null);
          setError(errText(e));
        },
      );
    }, 120);
    return () => {
      gone = true;
      clearTimeout(t);
    };
  }, [image, threshold, invert, board]);

  const choose = async () => {
    const [f] = await pickFiles("Choose a splash image", [{ name: "PNG image", extensions: ["png"] }]);
    if (f) setImage(f);
  };
  const device = d.device?.id;
  const shown = image ? preview : null;
  const ready = !!image && !!device && !!shown?.supported;
  const reasonId = useId();
  // Why Flash with splash… is off: the first missing step.
  const reason = !device
    ? "Save this radio first: plug it in and give it a name."
    : !image
      ? "Choose an image first."
      : error
        ? "The image has no preview."
        : shown && !shown.supported
          ? "This radio cannot take a splash yet."
          : null;

  return (
    <SectionCard
      title="Splash"
      actions={
        <Button variant="secondary" icon="gallery-add" onClick={() => void choose()}>
          Choose image…
        </Button>
      }
    >
      {error && <Banner kind="error">{error}</Banner>}
      {shown && !shown.supported && (
        <Banner kind="warning" icon="danger-triangle">
          {shown.reason}
        </Banner>
      )}
      <div className={styles.layout}>
        <figure className={styles.figure}>
          <div className={styles.frame}>
            {shown ? (
              <img
                className={styles.picture}
                alt={`Splash preview, ${shown.width} by ${shown.height}, ${shown.dark} dark pixels`}
                src={`data:image/png;base64,${shown.png_base64}`}
                width={256}
                height={128}
              />
            ) : (
              <span className={styles.placeholder}>128 × 64</span>
            )}
          </div>
          <figcaption className={styles.caption}>{shown ? `${shown.width} × ${shown.height}, 1 bit · ${shown.dark} dark pixels` : "PNG, scaled to fit, two tones"}</figcaption>
        </figure>
        <div className={styles.controls}>
          {image && <span className={`${styles.path} selectable`}>{tilde(image, home)}</span>}
          <Field label="Threshold" inline>
            {(id) => <input id={id} type="range" min={0} max={255} value={threshold} onChange={(e) => setThreshold(Number(e.target.value))} aria-valuetext={`${threshold} of 255`} />}
          </Field>
          <Checkbox label="Invert" checked={invert} onChange={(e) => setInvert(e.target.checked)} />
          <div className={styles.flash}>
            <Button
              variant="primary"
              disabled={!ready}
              aria-describedby={reason ? reasonId : undefined}
              onClick={() => device && image && void openFlash({ device, version: null, splash: { image, threshold, invert, board } })}
            >
              Flash with splash…
            </Button>
            {reason && (
              <span id={reasonId} className={styles.reason}>
                {reason}
              </span>
            )}
          </div>
        </div>
      </div>
    </SectionCard>
  );
}
