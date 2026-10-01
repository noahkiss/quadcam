import type { MouseEvent } from "react";
import { Icon } from "./Icon";
import styles from "./Stars.module.css";

interface Props {
  rating: number;
  /** Without it the stars only show the rating. */
  onRate?: (rating: number) => void;
  size?: number;
}

/** Zero to five stars. Clicking the current rating's star clears it. */
export function Stars({ rating, onRate, size = 14 }: Props) {
  const label = `${rating} of 5 stars`;
  if (!onRate) {
    return (
      <span role="img" aria-label={label} className={styles.stars}>
        {[1, 2, 3, 4, 5].map((i) => (
          <Icon key={i} name="star" size={size} className={i <= rating ? styles.on : styles.off} />
        ))}
      </span>
    );
  }
  const click = (i: number) => (e: MouseEvent) => {
    e.stopPropagation();
    onRate(rating === i ? 0 : i);
  };
  return (
    <span role="group" aria-label="Rating" className={styles.stars}>
      <span role="img" aria-label={label} className="visually-hidden" />
      {[1, 2, 3, 4, 5].map((i) => (
        <button
          key={i}
          type="button"
          aria-label={i === 1 ? "1 star" : `${i} stars`}
          aria-pressed={i <= rating}
          title={i === 1 ? "1 star" : `${i} stars`}
          className={styles.star}
          onClick={click(i)}
          onDoubleClick={(e) => e.stopPropagation()}
        >
          <Icon name="star" size={size} className={i <= rating ? styles.on : styles.off} />
        </button>
      ))}
    </span>
  );
}
