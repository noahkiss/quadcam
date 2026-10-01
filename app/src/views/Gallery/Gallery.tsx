// Every base component on one page, for design review and the contrast checks
// (e2e/theme.spec.ts). Reached with ?gallery in dev builds only.
import { useState } from "react";
import { Banner } from "../../components/Banner";
import { Button } from "../../components/Button";
import { AgentBadge, Chip, Kbd } from "../../components/Chip";
import { Dialog } from "../../components/Dialog";
import { Checkbox, SearchField, SelectField, TextField } from "../../components/Field";
import { FlagButtons, FlagMark } from "../../components/FlagButton";
import { Menu } from "../../components/Menu";
import { ProgressRing } from "../../components/ProgressRing";
import { SegmentedControl } from "../../components/SegmentedControl";
import { Stars } from "../../components/Stars";
import { Stepper } from "../../components/Stepper";
import { toast } from "../../components/toastStore";
import type { Flag } from "../../ipc/types";
import styles from "./Gallery.module.css";

export function Gallery() {
  const [view, setView] = useState<"grid" | "list">("grid");
  const [rating, setRating] = useState(3);
  const [flag, setFlag] = useState<Flag>("pick");
  const [dialog, setDialog] = useState(false);
  const [menu, setMenu] = useState<{ x: number; y: number } | null>(null);
  return (
    <main className={styles.page}>
      <h1>Components</h1>
      <section aria-label="Buttons" className={styles.row}>
        <Button variant="primary" icon="import">
          Import…
        </Button>
        <Button icon="folder-open">Open folder…</Button>
        <Button variant="ghost" icon="share">
          Share
        </Button>
        <Button variant="danger-ghost" icon="trash-bin-trash">
          Trash
        </Button>
        <Button variant="danger">Erase</Button>
        <Button icon="settings" variant="ghost" aria-label="Settings" />
        <Button size="sm">Small</Button>
        <Button variant="primary" disabled>
          Disabled
        </Button>
      </section>
      <section aria-label="Toggles" className={styles.row}>
        <SegmentedControl
          label="View"
          value={view}
          onChange={setView}
          segments={[
            { value: "grid", label: "Grid", icon: "grid", iconOnly: true },
            { value: "list", label: "List", icon: "list", iconOnly: true },
          ]}
        />
        <SegmentedControl label="Tabs" value={view} onChange={setView} segments={[{ value: "grid", label: "Details", icon: "tag" }, { value: "list", label: "Flight", icon: "radio" }]} />
        <Stars rating={rating} onRate={setRating} />
        <Stars rating={2} />
        <FlagButtons flag={flag} onFlag={setFlag} />
        <FlagMark flag="pick" />
        <FlagMark flag="reject" />
      </section>
      <section aria-label="Fields" className={styles.grid}>
        <TextField label="Name" defaultValue="farm-fun" />
        <TextField label="Date" type="date" defaultValue="2026-09-28" />
        <TextField label="Time" type="time" defaultValue="16:42" hint="Empty means noon" />
        <SelectField label="Aircraft" defaultValue="Whoop">
          <option>Whoop</option>
          <option>Five-inch</option>
        </SelectField>
        <SearchField label="Search the library" placeholder="Search names, notes, places" />
        <Checkbox label="Keep originals" detail="The DVR file goes in originals/ in the day folder." />
      </section>
      <section aria-label="Labels" className={styles.row}>
        <Chip icon="quad" tint="pink">
          Whoop
        </Chip>
        <Chip icon="map-point" tint="green">
          Home field
        </Chip>
        <Chip kind="accent" icon="sparkle">
          Last import
        </Chip>
        <span className={styles.thumb}>
          <Chip kind="overlay" icon="scissors" tint="sky">
            1 cut
          </Chip>
        </span>
        <AgentBadge />
        <Kbd>⌘</Kbd>
        <Kbd>Z</Kbd>
        <span className={styles.muted}>Muted text</span>
        <span style={{ color: "var(--success-text)" }}>Verified</span>
        <span style={{ color: "var(--warning-text)" }}>Warning</span>
        <span style={{ color: "var(--error-text)" }}>Failed</span>
        <span style={{ color: "var(--primary-text)" }}>Link</span>
      </section>
      <section aria-label="Progress" className={styles.row}>
        <ProgressRing value={0.72} label="Copying" />
        <Stepper
          label="Import steps"
          current="review"
          steps={[
            { id: "load", label: "Load" },
            { id: "review", label: "Review" },
            { id: "export", label: "Add to Library" },
            { id: "finish", label: "Finish" },
          ]}
        />
      </section>
      <section aria-label="Notices" className={styles.col}>
        <Banner icon="folder-open" tint="blue" action={<Button size="sm" variant="primary">Scan folder</Button>}>
          2 videos in ~/Movies/quadcam are not in the library.
        </Banner>
        <Banner kind="error" icon="danger-triangle" tint="red">
          <b>ffmpeg not found.</b> Install it with <code>brew install ffmpeg</code>, then restart QuadCam.
        </Banner>
      </section>
      <section aria-label="Overlays" className={styles.row}>
        <Button onClick={() => setDialog(true)}>Open dialog</Button>
        <Button onClick={() => toast("3 files moved to the Trash.")}>Show toast</Button>
        <Button onClick={() => toast("Settings not saved.", true)}>Show error toast</Button>
        <Button onClick={(e) => setMenu({ x: e.clientX, y: e.clientY })}>Open menu</Button>
      </section>
      <Dialog
        open={dialog}
        title="Move gap-run to the Trash?"
        onClose={() => setDialog(false)}
        actions={
          <>
            <Button type="submit" value="cancel" variant="ghost">
              Cancel
            </Button>
            <Button type="submit" value="ok" variant="danger">
              Move to Trash
            </Button>
          </>
        }
      >
        <p>Cuts and kept originals go too.</p>
      </Dialog>
      {menu && (
        <Menu
          label="Clip actions"
          at={menu}
          onClose={() => setMenu(null)}
          items={[
            { label: "Rename", icon: "pen", keys: "⏎", run: () => {} },
            { label: "Edit details", icon: "tag", keys: "⌘I", run: () => {} },
            null,
            { label: "Move to Trash", icon: "trash-bin-trash", keys: "⌘⌫", danger: true, run: () => {} },
          ]}
        />
      )}
    </main>
  );
}
