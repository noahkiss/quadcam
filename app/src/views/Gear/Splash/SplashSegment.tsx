// The radio's Splash segment (design 7.5): a PNG made into the radio's start-up picture,
// 128 x 64 and one bit, with a threshold and Invert. Make firmware… opens the flash sheet
// with the picture patched into the board's EdgeTX binary. Nothing is written until Apply.
import { useEffect, useState } from "react";
import { Banner } from "../../../components/Banner";
import { Button } from "../../../components/Button";
import { Checkbox, Field } from "../../../components/Field";
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

  return (
    <section className={styles.segment} aria-label="Splash">
      <div className={styles.bar}>
        <Button variant="secondary" icon="gallery-add" onClick={() => void choose()}>
          Choose image…
        </Button>
        {image && <span className={`${styles.muted} selectable`}>{tilde(image, home)}</span>}
      </div>
      {error && <Banner kind="error">{error}</Banner>}
      {shown && !shown.supported && (
        <Banner kind="warning" icon="danger-triangle">
          {shown.reason}
        </Banner>
      )}
      {!device && (
        <Banner icon="info">Save this radio first: plug it in and give it a name.</Banner>
      )}
      <div className={styles.controls}>
        <Field label="Threshold" inline>
          {(id) => <input id={id} type="range" min={0} max={255} value={threshold} onChange={(e) => setThreshold(Number(e.target.value))} aria-valuetext={`${threshold} of 255`} />}
        </Field>
        <Checkbox label="Invert" checked={invert} onChange={(e) => setInvert(e.target.checked)} />
      </div>
      {shown ? (
        <figure className={styles.figure}>
          <img className={styles.picture} alt={`Splash preview, ${shown.width} by ${shown.height}, ${shown.dark} dark pixels`} src={`data:image/png;base64,${shown.png_base64}`} width={512} height={256} />
          <figcaption className={styles.muted}>
            {shown.width} × {shown.height}, 1 bit · {shown.dark} dark pixels
          </figcaption>
        </figure>
      ) : (
        !image && <p className={styles.muted}>Choose a PNG. It is scaled to fit and cut to two tones.</p>
      )}
      <div className={styles.bar}>
        <Button variant="primary" disabled={!ready} onClick={() => device && image && void openFlash({ device, version: null, splash: { image, threshold, invert, board } })}>
          Make firmware…
        </Button>
      </div>
    </section>
  );
}
