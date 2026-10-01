import { useStore } from "../../store";
import { sel } from "../../store/settings";
import { Button } from "../../components/Button";
import { Icon } from "../../components/Icon";
import { tilde } from "../../lib/format";
import { importAction, openFolderAction, pickLogs } from "../../actions/session";
import styles from "./FirstRun.module.css";

const LAYOUT = { year_day: "A folder per year, then per flying day.", day: "A folder per flying day.", flat: "Every file in one folder." };

/** The empty library: import, and the things to set up once. */
export function FirstRun() {
  const s = useStore();
  const profiles = sel.profiles(s).filter((p) => p.name);
  const places = sel.places(s);
  const logDir = sel.logDir(s);
  const tools = !!s.env?.tools;
  return (
    <section className={styles.first} aria-label="First run">
      <div className={styles.hero}>
        <span className={styles.heroIcon}>
          <Icon name="sd-card" size={40} />
        </span>
        <h2>Import your first flights</h2>
        <p>Insert the goggles' DVR card, or choose a folder of clips.</p>
        <div className={styles.row}>
          <Button variant="primary" icon="import" disabled={!tools} onClick={importAction}>
            Import…
          </Button>
          <Button icon="folder-open" disabled={!tools} onClick={openFolderAction}>
            Open folder…
          </Button>
        </div>
        <p className={styles.status}>
          <span className={styles.dot} />
          <span>Watching for cards</span>
          <span aria-hidden="true">·</span>
          {logDir ? (
            <span>Radio logs: {tilde(logDir, s.home)}</span>
          ) : (
            <>
              <Icon name="radio" size={14} />
              <span>Radio logs: none.</span>
              <button type="button" className={styles.link} onClick={pickLogs}>
                Choose the radio's LOGS folder
              </button>
            </>
          )}
        </p>
      </div>
      <section className={styles.setup} aria-labelledby="setup-title">
        <h2 id="setup-title">Set up once</h2>
        <div className={styles.grid}>
          <div className={styles.card}>
            <h3>
              <Icon name="quad" tint="pink" />
              Aircraft
            </h3>
            <p>{profiles.length ? profiles.map((p) => p.name).join(", ") : "No aircraft yet."}</p>
            <Button size="sm" variant="primary" icon="add" onClick={() => s.openSettings("aircraft")}>
              Add aircraft
            </Button>
          </div>
          <div className={styles.card}>
            <h3>
              <Icon name="map-point" tint="green" />
              Places
            </h3>
            <p>{places.length ? places.map((p) => p.name).join(", ") : "No saved places."}</p>
            <Button size="sm" icon="add" onClick={() => s.openSettings("places")}>
              Add place
            </Button>
          </div>
          <div className={styles.card}>
            <h3>
              <Icon name="folder-open" tint="blue" />
              Library folder
            </h3>
            <p className="mono selectable">{tilde(sel.outputDir(s), s.home)}</p>
            <p>{LAYOUT[sel.layout(s)]}</p>
            <Button size="sm" onClick={() => s.openSettings("library")}>
              Change…
            </Button>
          </div>
        </div>
      </section>
    </section>
  );
}
