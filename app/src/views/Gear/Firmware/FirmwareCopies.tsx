// Gear > Firmware > Saved copies: each radio's firmware copies (a Read firmware, or the copy a
// flash or restore kept first), newest first, with Restore… per copy. Restore opens the apply
// sheet on the restore plan; nothing is written before its Restore click.
import { useEffect } from "react";
import { useStore } from "../../../store";
import { Button } from "../../../components/Button";
import { DataTable, type Column } from "../../../components/dash/DataTable";
import { EmptyState } from "../../../components/dash/EmptyState";
import { SectionCard } from "../../../components/dash/SectionCard";
import type { FirmwareCopy, FirmwareStatus } from "../../../ipc/types";
import { fmtWhen } from "../../../lib/backups";
import styles from "./Firmware.module.css";

const KIND_LABEL: Record<FirmwareCopy["copy"]["kind"], string> = {
  read: "Read firmware",
  before_flash: "Before a flash",
  before_restore: "Before a restore",
};

interface Props {
  radios: FirmwareStatus[];
}

export function FirmwareCopies({ radios }: Props) {
  const copies = useStore((s) => s.fwCopies);
  const load = useStore((s) => s.loadFwCopies);
  const openRestore = useStore((s) => s.openFirmwareRestore);

  useEffect(() => {
    void load();
  }, [load]);

  const radioName = (id: string | null) => (id ? radios.find((r) => r.device === id)?.name ?? id : "No radio linked");
  const columns: Column<FirmwareCopy>[] = [
    {
      key: "taken",
      header: "Taken",
      rowHeader: true,
      cell: (c) => (
        <>
          {fmtWhen(c.copy.taken_at)}
          <span className={styles.note}>{KIND_LABEL[c.copy.kind]}</span>
        </>
      ),
    },
    { key: "radio", header: "Radio", cell: (c) => radioName(c.radio) },
    {
      key: "version",
      header: "EdgeTX",
      cell: (c) => (c.copy.image_version ? `${c.copy.image_version}${c.copy.image_board ? ` · ${c.copy.image_board}` : ""}` : "Unknown"),
    },
    { key: "size", header: "Size", numeric: true, width: "6rem", cell: (c) => `${Math.round(c.copy.size / 1024)} KB` },
    {
      key: "hash",
      header: "SHA-256",
      optional: true,
      cell: (c) => (
        <span className="mono selectable" title={c.copy.sha256}>
          {c.copy.sha256.slice(0, 12)}…
        </span>
      ),
    },
    {
      key: "actions",
      header: <span className="visually-hidden">Actions</span>,
      width: "7rem",
      cell: (c) =>
        c.radio && (
          <div className={styles.actions}>
            <Button size="sm" variant="secondary" onClick={() => void openRestore({ device: c.radio!, copy: c.copy.id })}>
              Restore…
            </Button>
          </div>
        ),
    },
  ];

  return (
    <SectionCard title="Saved firmware copies">
      {copies && (
        <DataTable
          label="Saved firmware copies"
          columns={columns}
          rows={copies}
          rowKey={(c) => c.copy.id}
          empty={<EmptyState icon="upload-minimalistic">No firmware copies yet. Read firmware saves one.</EmptyState>}
        />
      )}
    </SectionCard>
  );
}
