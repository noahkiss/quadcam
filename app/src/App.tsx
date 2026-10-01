import { useEffect } from "react";
import { useStore } from "./store";
import { screenOf } from "./store/library";
import { Banner } from "./components/Banner";
import { start } from "./events";
import { useKeys } from "./keys";
import { useMenu } from "./menu/useMenu";
import { Overlays } from "./views/Shell/Overlays";
import { Sidebar } from "./views/Shell/Sidebar";
import { Toolbar } from "./views/Shell/Toolbar";
import { LibraryView } from "./views/Library/LibraryView";
import { FirstRun } from "./views/FirstRun/FirstRun";
import { ClipDetail } from "./views/ClipDetail/ClipDetail";
import styles from "./App.module.css";

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
      {env && !env.tools && (
        <div className={styles.banner}>
          <Banner kind="error" icon="danger-triangle" tint="red">
            <b>ffmpeg not found.</b> Import is blocked. Install it with <code>{env.install_hint}</code>, then restart QuadCam.
          </Banner>
        </div>
      )}
      <div className={styles.body}>
        <Sidebar />
        <main className={styles.main}>
          {screen === "detail" ? <ClipDetail /> : screen === "first-run" ? <FirstRun /> : <LibraryView onMenu={onMenu} />}
        </main>
      </div>
      <Overlays />
    </div>
  );
}
