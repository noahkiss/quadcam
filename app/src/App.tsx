import { useEffect } from "react";
import { useStore } from "./store";
import { screenOf } from "./store/library";
import { sel } from "./store/settings";
import { updateBanner } from "./store/update";
import { Banner } from "./components/Banner";
import { Button } from "./components/Button";
import { installModule } from "./actions/modules";
import { start } from "./events";
import { useKeys } from "./keys";
import { useMenu } from "./menu/useMenu";
import { Overlays } from "./views/Shell/Overlays";
import { Sidebar } from "./views/Shell/Sidebar";
import { Toolbar } from "./views/Shell/Toolbar";
import { StatusBar } from "./views/Shell/StatusBar";
import { UpdateBanner } from "./views/Shell/UpdateBanner";
import { GearView } from "./views/Gear/GearView";
import { LibraryView } from "./views/Library/LibraryView";
import { FirstRun } from "./views/FirstRun/FirstRun";
import { ClipDetail } from "./views/ClipDetail/ClipDetail";
import { ImportSheet } from "./views/ImportSheet/ImportSheet";
import { SettingsSheet } from "./views/Settings/SettingsSheet";
import styles from "./App.module.css";

/** Suggestions for the place, keyword, author and note fields. */
function Suggestions() {
  const places = useStore(sel.places);
  const recents: { keywords?: string[]; authors?: string[]; notes?: string[] } = useStore(sel.recents);
  const list = (id: string, values: string[] = []) => (
    <datalist id={id}>
      {values.map((v) => (
        <option key={v} value={v} />
      ))}
    </datalist>
  );
  return (
    <>
      {list("places-list", places.map((p) => p.name))}
      {list("recent-keywords", recents.keywords)}
      {list("recent-authors", recents.authors)}
      {list("recent-notes", recents.notes)}
    </>
  );
}

/** The update banner, in the same row as the ffmpeg banner. */
function UpdateBannerRow() {
  const shown = useStore((s) => !!updateBanner(s));
  return shown ? (
    <div className={styles.banner}>
      <UpdateBanner />
    </div>
  ) : null;
}

export function App() {
  const screen = useStore(screenOf);
  const env = useStore((s) => s.env);
  const setMenu = useStore((s) => s.setMenu);
  const select = useStore((s) => s.select);
  useKeys();
  useMenu();
  useEffect(() => {
    let stop: (() => void) | undefined;
    let gone = false;
    start().then((f) => (gone ? f() : (stop = f)));
    return () => {
      gone = true;
      stop?.();
    };
  }, []);
  const onMenu = (e: React.MouseEvent, id: string) => {
    e.preventDefault();
    if (!useStore.getState().selected.has(id)) select(id);
    setMenu({ id, x: e.clientX, y: e.clientY });
  };
  return (
    <div className={styles.app}>
      <Toolbar />
      <div className={styles.banners}>
        {env && !env.tools && (
          <div className={styles.banner}>
            <Banner
              kind="error"
              icon="danger-triangle"
              tint="red"
              action={
                <Button size="sm" variant="primary" onClick={() => void installModule("ffmpeg")}>
                  Install ffmpeg…
                </Button>
              }
            >
              <b>ffmpeg not found.</b> Import is blocked until QuadCam has ffmpeg. You can also run <code>{env.install_hint}</code>, then restart QuadCam.
            </Banner>
          </div>
        )}
        <UpdateBannerRow />
      </div>
      <Sidebar />
      <main className={styles.main}>
        {screen === "gear" ? <GearView /> : screen === "detail" ? <ClipDetail /> : screen === "first-run" ? <FirstRun /> : <LibraryView onMenu={onMenu} />}
      </main>
      <StatusBar />
      <ImportSheet />
      <SettingsSheet />
      <Overlays />
      <Suggestions />
    </div>
  );
}
