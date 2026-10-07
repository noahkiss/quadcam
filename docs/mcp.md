# MCP server

`quadcam-cli mcp` runs an MCP server on stdio. A coding agent can use it to import clips with you and to work on the library. It drives the same core as the app and the command-line tool.

## Add it to an agent

The Homebrew cask links `quadcam-cli` into Homebrew's `bin`. To add the server to Claude Code:

```bash
claude mcp add quadcam -- "$(brew --prefix)/bin/quadcam-cli" mcp
```

If you built from source, give the path to `QuadCam.app/Contents/MacOS/quadcam-cli` instead.

## App or headless

- If the app is running, the server works on the app's session. You see every change in the app as it happens.
- If the app is not running, the server works on the session file, like the command-line tool.

The server checks for the app on every call. You can quit and open the app again without a restart of the server. A call that started against the app is never retried headless.

## Tools

The server has 19 tools.

**Import:**

| Tool | What it does |
|---|---|
| `quadcam_status` | Shows the mode (app or headless), the cards with their source (analog or DJI), the radios, and the session with its source |
| `quadcam_load_clips` | Stages, checks, and dates a card or a folder. Joins recordings the DVR split into files; `join=false` keeps them apart for that load. See [Joined recordings](library.md#joined-recordings) |
| `quadcam_read_clips` | Reads the clips, their source, plans, moments, keep ranges and cuts. Returns thumbnails as images. The date source reads `radio log`, `clip clock`, `import` or `edited` |
| `quadcam_match_logs` | Dates the clips from EdgeTX logs |
| `quadcam_suggest` | Suggests names, dates, times, notes, skips, cuts, the log offset, or metadata (profile, place or location, keywords, author). `split_by_flight` adds one cut per radio-log pack; `joined` joins or parts a split recording |
| `quadcam_export` | Converts and verifies clips and cuts. Can also add them to Photos. When the `delete_clips_after_import` setting is on, deletes the clips that verified from the card; `keep_clips=true` keeps them for that run. See [Settings](settings.md#delete-clips-after-import) |
| `quadcam_verify` | Checks the outputs again |
| `quadcam_add_to_photos` | Adds verified outputs to Photos |
| `quadcam_eject` | Makes the card safe to remove (unmounts it) |
| `quadcam_format_card` | Erases an analog card after an import, or with `prep=true` a card with no session (card prep). Refuses a DJI card. See [Format safety](format-safety.md) |

**Library:**

| Tool | What it does |
|---|---|
| `quadcam_library` | Lists and searches the clips already in the library (read-only) |
| `quadcam_library_edit` | Changes library clips: rating, flag, name, note, keywords, author, place, aircraft, date, time |
| `quadcam_library_files` | Sets and writes cuts, adds one cut per radio-log pack (`split_by_flight`), moves clips to the Trash, adds them to Photos, rebuilds the index, renames clips to the file-name date format, matches radio logs to library clips |

**Setup:**

| Tool | What it does |
|---|---|
| `quadcam_places` | Lists, searches (address or place name), saves and deletes saved places |
| `quadcam_profiles` | Lists, saves and deletes aircraft profiles. Sets the default |
| `quadcam_settings` | Reads and writes the app's settings. Lists, installs and removes [modules](modules.md): `module_install` needs `confirm=true`, after the agent showed you the module's license |

### Gear

The Gear tools are split by what they can change, so an agent harness can allow the first
freely and ask before the others. See [Gear](gear.md).

| Tool | Changes | What it does |
|---|---|---|
| `quadcam_gear` | Nothing | `status`: the gear folder, the Gear settings, and what is plugged in. `devices`: the saved devices. `osd`: a Betaflight OSD layout from `paths` (dump or diff files), each profile drawn as text, with the overlap and off-screen check. `card`: an EdgeTX card's version, models, selected model and its aircraft, radio clock, and one model in full (`model`). `card_preview`: the checks and diff of card `edits`; writes nothing |
| `quadcam_gear_edit` | QuadCam's own data | `device_save` names a device or links it to an aircraft profile. `device_forget` removes it from the list |
| `quadcam_gear_apply` | A device | No actions yet. Each will need a plan's `digest` and `confirm=true` |

## Suggestions in the app

An agent's suggestions show in the app with a dashed outline and an **agent** badge. When you edit a field, the value becomes yours. The agent reads the final values back before it exports.

## Erase a card

`quadcam_format_card` needs the device, the volume UUID, and `confirm=true`. The agent reads the device and the volume UUID with `dry_run=true` first.

With `prep=true`, the tool erases a card that has no session: a new card, or a card whose clips are all in the library. The dry run then needs `mount`, the card's mount point. See [Card prep](format-safety.md#card-prep).

If the app is running, you must also select **Erase** in the app. Cancel, a closed dialog, or 3 minutes without a click refuses the erase.

## Photos from an agent

A headless server runs PhotoKit in its own process. macOS then asks for Photos permission for your terminal app, not for QuadCam. Keep the app open when an agent adds clips to Photos. See [Photos](photos.md).

## The control socket

The app serves the agent through a Unix socket:

```
~/Library/Application Support/app.quadcam/control.sock
```

The socket uses JSON-RPC 2.0, one object per line. Only your user account can open it: the folder is `0700` and the socket is `0600`. Each call opens a fresh connection with a ping, so an app restart never leaves a dead connection.
