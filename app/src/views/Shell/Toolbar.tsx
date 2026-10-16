import { useStore } from "../../store";
import { screenOf } from "../../store/library";
import { sel } from "../../store/settings";
import { Button } from "../../components/Button";
import { Icon } from "../../components/Icon";
import { SearchField, Select } from "../../components/Field";
import { SegmentedControl } from "../../components/SegmentedControl";
import { MoreMenu } from "../../components/dash/MoreMenu";
import { setSort, setThumbSize, setView } from "../../actions/library";
import { importAction } from "../../actions/session";
import type { SortKey } from "../../lib/library";
import { onGlass } from "../../native";
import styles from "./Toolbar.module.css";

const SORTS: { key: SortKey; label: string }[] = [
  { key: "date", label: "Date" },
  { key: "rating", label: "Rating" },
  { key: "duration", label: "Duration" },
  { key: "name", label: "Name" },
];

/** The window's top bar: search, thumbnail size, sort, grid or list, Import, Settings. In a
 *  narrow window a button shows the sidebar, and sort and grid or list fold into a menu. */
export function Toolbar() {
  const screen = useStore(screenOf);
  const query = useStore((s) => s.query);
  const setQuery = useStore((s) => s.setQuery);
  const view = useStore(sel.libView);
  const thumb = useStore(sel.thumbSize);
  const sort = useStore(sel.sort);
  const tools = useStore((s) => !!s.env?.tools);
  const openSettings = useStore((s) => s.openSettings);
  const sidebarOpen = useStore((s) => s.sidebarOpen);
  const setSidebarOpen = useStore((s) => s.setSidebarOpen);
  const lib = screen === "library";
  return (
    <header className={styles.bar} data-tauri-drag-region={onGlass() ? "deep" : undefined}>
      <Button
        className={styles.sidebarButton}
        variant="ghost"
        icon="sidebar"
        aria-label={sidebarOpen ? "Hide sidebar" : "Show sidebar"}
        aria-expanded={sidebarOpen}
        aria-controls="sidebar"
        onClick={() => setSidebarOpen(!sidebarOpen)}
      />
      <div className={styles.brand}>
        <img className={styles.logo} src="/emblem.png" alt="" aria-hidden="true" draggable={false} />
        <h1>QuadCam</h1>
      </div>
      <div className={styles.mid}>
        {(screen === "library" || screen === "first-run") && <SearchField label="Search the library" placeholder="Search names, notes, places" value={query} onChange={(e) => setQuery(e.target.value)} className={styles.search} />}
      </div>
      <div className={styles.right}>
        {lib && (
          <>
            <div className={styles.wide}>
              <label className={styles.size} title="Thumbnail size">
                <Icon name="grid" size={12} />
                <input type="range" min={1} max={5} value={thumb} aria-label="Thumbnail size" disabled={view === "list"} onChange={(e) => setThumbSize(+e.target.value)} />
                <Icon name="grid" size={16} />
              </label>
              <label className={styles.sort}>
                <Icon name="arrow-down" size={14} />
                <Select aria-label="Sort by" value={sort.key} onChange={(e) => e.target.value !== sort.key && setSort(e.target.value as SortKey)}>
                  {SORTS.map((o) => (
                    <option key={o.key} value={o.key}>
                      {o.label}
                    </option>
                  ))}
                </Select>
              </label>
              <SegmentedControl
                label="View"
                value={view}
                onChange={setView}
                segments={[
                  { value: "grid", label: "Grid", icon: "grid", iconOnly: true },
                  { value: "list", label: "List", icon: "list", iconOnly: true },
                ]}
              />
            </div>
            <div className={styles.narrow}>
              <MoreMenu
                label="View options"
                items={[
                  ...SORTS.map((o) => ({ label: `Sort by ${o.label.toLowerCase()}`, checked: sort.key === o.key, run: () => o.key !== sort.key && setSort(o.key) })),
                  null,
                  { label: "Show as grid", checked: view === "grid", run: () => setView("grid") },
                  { label: "Show as list", checked: view === "list", run: () => setView("list") },
                ]}
              />
            </div>
          </>
        )}
        <Button variant="primary" icon="import" disabled={!tools} onClick={importAction}>
          Import…
        </Button>
        <Button variant="ghost" icon="settings" aria-label="Settings" title="Settings (⌘,)" onClick={() => openSettings("library")} />
      </div>
    </header>
  );
}
