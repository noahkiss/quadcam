import { useShallow } from "zustand/react/shallow";
import { useStore } from "../../store";
import { screenOf, selectedIds } from "../../store/library";
import { sel } from "../../store/settings";
import { Button } from "../../components/Button";
import { Icon } from "../../components/Icon";
import { albumAction } from "../../lib/library";
import { addLibToPhotos, rate, shareClips, trashClips } from "../../actions/library";
import styles from "./SelectionBar.module.css";

/** The bar for two or more selected clips: rate, flag, share, add to the album, trash. */
export function SelectionBar() {
  const ids = useStore(useShallow(selectedIds));
  const lib = useStore((s) => screenOf(s) === "library");
  const album = useStore((s) => albumAction(sel.photosAlbum(s)));
  const clear = useStore((s) => s.clearSelection);
  if (ids.length < 2 || !lib) return null;
  return (
    <div role="toolbar" aria-label="Selected clips" className={styles.bar}>
      <b>{ids.length} selected</b>
      <span className={styles.sep} aria-hidden="true" />
      <span role="group" aria-label="Rate" className={styles.rate}>
        {[1, 2, 3, 4, 5].map((i) => (
          <button key={i} type="button" aria-label={i === 1 ? "1 star" : `${i} stars`} title={i === 1 ? "1 star" : `${i} stars`} onClick={() => rate(ids, i)}>
            <Icon name="star" size={14} />
          </button>
        ))}
      </span>
      <Button size="sm" variant="ghost" icon="flag" aria-label="Pick" title="Pick" onClick={() => rate(ids, null, "pick")} />
      <Button size="sm" variant="ghost" icon="close" aria-label="Reject" title="Reject" onClick={() => rate(ids, null, "reject")} />
      <span className={styles.sep} aria-hidden="true" />
      <Button size="sm" variant="ghost" icon="share" onClick={(e) => shareClips(ids, e.currentTarget)}>
        Share
      </Button>
      {album && (
        <Button size="sm" variant="ghost" icon="photos" onClick={() => addLibToPhotos(ids)}>
          {album}
        </Button>
      )}
      <Button size="sm" variant="danger-ghost" icon="trash-bin-trash" onClick={() => trashClips(ids)}>
        Trash
      </Button>
      <Button size="sm" variant="ghost" icon="close-circle" aria-label="Deselect" title="Deselect" onClick={clear} />
    </div>
  );
}
