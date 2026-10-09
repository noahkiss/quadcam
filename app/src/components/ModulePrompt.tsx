// The consent prompt for a module download: name, version, size, license, source and where the
// files come from. Shown by Settings > Modules and by the "ffmpeg not found" banner.
import { openLink } from "../ipc/api";
import type { ModulePin } from "../ipc/types";
import styles from "../views/Settings/SettingsSheet.module.css";

export const mb = (bytes: number) => `${(bytes / 1e6).toFixed(1)} MB`;

export function Link({ href, children }: { href: string; children: string }) {
  return (
    <a
      href={href}
      onClick={(e) => {
        e.preventDefault();
        openLink(href).catch(() => {});
      }}
    >
      {children}
    </a>
  );
}

export function ModulePrompt({ pin }: { pin: ModulePin }) {
  return (
    <dl className={styles.prompt}>
      <dt>Version</dt>
      <dd>{pin.version}</dd>
      <dt>Size</dt>
      <dd>{mb(pin.assets.reduce((n, a) => n + a.size, 0))}</dd>
      <dt>License</dt>
      <dd>
        <Link href={pin.license_url}>{pin.license}</Link>
      </dd>
      <dt>Source</dt>
      <dd>
        <Link href={pin.source}>{pin.source}</Link>
      </dd>
      <dt>Download from</dt>
      <dd className="mono selectable">{pin.assets.map((a) => a.url).join("\n")}</dd>
    </dl>
  );
}
