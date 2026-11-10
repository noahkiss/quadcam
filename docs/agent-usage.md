# Agent and CLI usage

How scripts and coding agents drive QuadCam: `quadcam-cli` and its MCP server. `docs/cli.md` and `docs/mcp.md` are the user guides.

`quadcam-cli` does everything the GUI does, on the same core. While the app runs, its session
commands and the Gear commands on the app's own state (FC pause, stop, card mount) go to the
app's core through the control socket (`via` in `bin/quadcam-cli/main.rs`; not with
`--session FILE`), so one core owns the session. Every command takes `--json`
and prints one object: `{"ok":true,"result":...}` or
`{"ok":false,"error":{"code","exit","message"}}`. Exit codes: 0 ok, 1 failed, 2 usage,
3 refused by a safety guard, 4 no session, no card or no device. Runs share a session file
(`~/Library/Caches/app.quadcam/session.json`; override with `--session FILE`).

```bash
quadcam-cli --json cards                       # cards and radio log sources
quadcam-cli --json scan /Volumes/CARD          # list clips, copy nothing
quadcam-cli --json stage /Volumes/CARD         # copy to staging; new session
quadcam-cli --json analyze                     # probe, recover, thumbnails
quadcam-cli --json dates --logs /path/to/LOGS --day 2026-10-04 --set 2=2026-10-03
quadcam-cli --json import --name 0=backyard-loops --skip 3 --format mp4 --add-to-photos
quadcam-cli --json import --plan plan.json     # {"clips":[{"id":0,"name":"..","date":"..","time":"HH:MM","note":"..","skip":false}],"format":"mov","output_dir":".."}
quadcam-cli --json import --time 0=18:30       # manual time of day (default noon)
quadcam-cli --json import --keep-clips         # this run keeps the clips (delete_clips_after_import)
quadcam-cli --json stage /Volumes/CARD --no-join   # split DVR recordings stay separate this run
quadcam-cli --json import --separate 0         # clip 0's files (a split recording) import one by one
quadcam-cli --json cut 0 --by-flight           # one cut per radio-log flight
quadcam-cli --json verify                      # re-check the session's outputs
quadcam-cli --json clear                       # forget the session, delete the session file
quadcam-cli --json library list --group picks  # also rate, rebuild, rename, edit, cut, trash, photos
quadcam-cli --json library edit <id> --date 2026-09-28 --time 18:30 --profile Whoop
quadcam-cli --json places search "Golden Gate Park" [--provider nominatim]
quadcam-cli --json places save NAME --location LAT,LON | --search QUERY [--pick N]
quadcam-cli --json profiles save NAME --aircraft .. --camera-make .. --models A,B --default
quadcam-cli --json settings set layout=day place_folders=true   # `settings` shows them
quadcam-cli --json modules [check | install NAME --yes | remove NAME | manifest]
quadcam-cli --json library apply-name-format   # after settings set name_date_format=YY.MM.DD
quadcam-cli --json photos out.mp4 --album Drone
quadcam-cli --json eject                       # safe to remove: unmountDisk a card
quadcam-cli --json gear status                 # gear folder, Gear settings, what is plugged in
quadcam-cli --json gear devices save <id> --name N --aircraft PROFILE
quadcam-cli --json gear fc identify|read|check|notes|usb   # a Betaflight FC (read reboots it)
quadcam-cli gear osd quad.dump_all.txt [apply.cli] [--grid PAL] --text   # OSD per profile, checked
quadcam-cli --json gear osd-edit <fc> --move vbat=12,3 --copy 1:2         # stage OSD edits (one "OSD layout" change)
quadcam-cli --json format --plan               # runs every guard, prints device + volume UUID
quadcam-cli --json format --device /dev/diskN --volume-uuid <uuid> --yes
quadcam-cli --json format --prep --plan --mount /Volumes/CARD   # card prep: no session needed
quadcam-cli --json format --prep --device /dev/diskN --volume-uuid <uuid> --yes
```

`format` refuses (exit 3) unless every GUI guard passes and `--device` (the card's whole
disk), `--volume-uuid` and `--yes` all match the staged card. `format --prep` is card prep
(`docs/gear-design.md` 7.9): a card with no session, refused while any clip on it (by content
fingerprint) is not in the library. It moves to `gear card-prep` with the app's card-prep
button (WP13).

## MCP server

`quadcam-cli mcp` is an MCP server on stdio, a thin layer over the same core. `cargo tauri
build` also puts the CLI inside the bundle (`QuadCam.app/Contents/MacOS/quadcam-cli`), and the
cask links it into Homebrew's `bin`. Register it:

```bash
claude mcp add quadcam -- "$(brew --prefix)/bin/quadcam-cli" mcp
```

- When the app is running, the server drives the app's session through the control socket
  (`~/Library/Application Support/app.quadcam/control.sock`: JSON-RPC 2.0, one object per
  line, folder 0700, socket 0600). The person sees every change live. When the app is not
  running, it falls back to a headless core on the shared session file. Each call opens a
  fresh connection (with a ping), so an app restart never leaves a dead pipe. A call that
  started against the app is never retried headless.
- Tools: the import flow `quadcam_status`, `quadcam_load_clips`, `quadcam_read_clips`
  (thumbnails as image content), `quadcam_match_logs`, `quadcam_suggest`, `quadcam_export`,
  `quadcam_verify`, `quadcam_add_to_photos`, `quadcam_eject`, `quadcam_format_card`; the
  library `quadcam_library` (read-only search), `quadcam_library_edit` (rating, flag, name,
  details, aircraft, date, time), `quadcam_library_files` (cuts, Trash, Photos, rebuild);
  setup `quadcam_places` (list, search, save, delete), `quadcam_profiles` (list, save,
  delete, set_default), `quadcam_settings` (read, write, modules, module_install, module_remove). Keep the surface this small:
  add an action or a field to a tool before adding a tool. Each tool's arguments are a type in
  `mcp/params.rs`; `mcp/tools.rs` derives the input schema from it (schemars) and keeps the
  descriptions as written. The handler deserializes the arguments into that type once. A
  schema change shows in the `snapshots` test `mcp_tools`. Gear's tools are `quadcam_gear` (reads),
  `quadcam_gear_edit` (QuadCam's own data) and `quadcam_gear_apply` (devices; digest and
  `confirm=true`); a Gear feature adds an action to one of them, in `mcp/gear.rs`.
  `quadcam_library_edit` is one `library_update`
  call, which checks every id and value before any file changes.
- **The MCP and CLI surface is frozen at 1.0** (`SCHEMA_VERSION` in `schema.rs`, now 1;
  `docs/mcp.md` and `docs/cli.md` hold the Stability rules). To add to it: add the tool,
  `action` value, optional parameter, command or flag; give the MCP action a CLI command (or
  the reverse); name a concept as the surface already does (`log_dir`, `rate_profile`,
  `device` for a saved device id, `confirm` in MCP and `--yes` in the CLI, a plan before a
  write); run `QUADCAM_UPDATE_SURFACE=1 cargo test --test it surface::` and commit the changed
  baseline lines with `INSTA_UPDATE=always cargo test --test it snapshots::`. Never remove or
  rename an item, change a parameter's type, or make a parameter required: the surface test
  fails. To retire one, add a `Deprecation` to `schema::DEPRECATIONS` (the item keeps
  working and warns, in MCP and in the CLI), and remove it only in a later release that
  bumps `SCHEMA_VERSION`. A rename before the deprecation period uses a serde `alias`
  (MCP) or a clap `alias` (CLI) so the old name keeps working.
- **Updates:** while idle, `serve_stdio` stats the file it started from (`mcp/reload.rs`). A
  newer build settled there (probe: `QUADCAM_MCP_PROBE=1 quadcam-cli mcp`) runs in the same
  process through `exec`, with the session's state in `QUADCAM_MCP_HANDOFF`, and sends
  `notifications/tools/list_changed` when its tools differ. Never while a line is half read or
  `Core::restart_blocker` names a job; never to an older version. `HANDOFF_FORMAT` versions the
  handoff; a probe lists the formats a build reads. Tests: `tests/it/mcp_reload.rs` (`QUADCAM_MCP_TEST_BUILD`, cargo only).
- Agent suggestions show in the GUI with a dashed accent outline and an "agent" badge until
  the person edits the field. Read back with `quadcam_read_clips` before export.
- `quadcam_format_card` needs `device`, `volume_uuid` and `confirm=true` (read them with
  `dry_run=true`). `prep=true` (with `mount` for the dry run) is card prep, the `card_prep_plan`
  and `card_prep` methods in `core/prep.rs`. The plan names the file system and any size
  advice (`warnings`). With the app running, the person must also click Erase in the app;
  Cancel, closing the dialog or 3 minutes without a click refuses.
- Photos from the CLI or a headless MCP server runs PhotoKit in that process, so macOS
  attributes the permission prompt to the terminal. Prefer the app for Photos.
