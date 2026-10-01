import { useStore } from "../../store";
import { screenOf } from "../../store/library";
import { sel } from "../../store/settings";
import { Button } from "../../components/Button";
import { Icon } from "../../components/Icon";
import { SearchField, Select } from "../../components/Field";
import { SegmentedControl } from "../../components/SegmentedControl";
import { setSort, setThumbSize, setView } from "../../actions/library";
import { importAction } from "../../actions/session";
import type { SortKey } from "../../lib/library";
import styles from "./Toolbar.module.css";

/** The window's top bar: search, thumbnail size, sort, grid or list, Import, Settings. */
export function Toolbar() {
  const screen = useStore(screenOf);
  const query = useStore((s) => s.query);
  const setQuery = useStore((s) => s.setQuery);
  const view = useStore(sel.libView);
  const thumb = useStore(sel.thumbSize);
  const sort = useStore(sel.sort);
  const tools = useStore((s) => !!s.env?.tools);
  const openSettings = useStore((s) => s.openSettings);
  const lib = screen === "library";
  return (
    <header className={styles.bar}>
      <div className={styles.brand}>
        <span className={styles.logo} aria-hidden="true">
          <span />
          <span />
          <span />
          <span />
        </span>
        <h1>QuadCam</h1>
      </div>
      <div className={styles.mid}>
        {screen !== "detail" && <SearchField label="Search the library" placeholder="Search names, notes, places" value={query} onChange={(e) => setQuery(e.target.value)} className={styles.search} />}
      </div>
      <div className={styles.right}>
        {lib && (
          <>
            <label className={styles.size} title="Thumbnail size">
              <Icon name="grid" size={12} />
              <input type="range" min={1} max={5} value={thumb} aria-label="Thumbnail size" disabled={view === "list"} onChange={(e) => setThumbSize(+e.target.value)} />
              <Icon name="grid" size={16} />
            </label>
            <label className={styles.sort}>
              <Icon name="arrow-down" size={14} />
              <Select aria-label="Sort by" value={sort.key} onChange={(e) => e.target.value !== sort.key && setSort(e.target.value as SortKey)}>
                <option value="date">Date</option>
                <option value="rating">Rating</option>
                <option value="duration">Duration</option>
                <option value="name">Name</option>
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
