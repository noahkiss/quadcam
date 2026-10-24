import type { SimPreset } from "../../../ipc/types";

const options = (list: SimPreset[]) =>
  list.map((p) => (
    <option key={p.id} value={p.id}>
      {p.label}
    </option>
  ));

/** The Aircraft menu's options: profiles fitted to the person's quads first, when there are any. */
export function ProfileOptions({ presets }: { presets: SimPreset[] }) {
  const fitted = presets.filter((p) => p.fitted);
  if (!fitted.length) return <>{options(presets)}</>;
  return (
    <>
      <optgroup label="Your quads">{options(fitted)}</optgroup>
      <optgroup label="Built in">{options(presets.filter((p) => !p.fitted))}</optgroup>
    </>
  );
}
