# Community contributions: design

Status: options for review, 2026-10-22. Nothing here is built.
Baseline: `main` after release 0.22.

QuadCam ships data tables that one bench cannot fill. The radio table alone has about 50 boards, and QuadCam has proven a write on two of them. Other
pilots own the rest. This doc is the plan to let them send what their gear shows, opt-in, with
nothing personal in it, through a public reviewed channel, and to ship the result back to
every QuadCam.

It extends [Gear design](gear-design.md) and [Radio design](radio-design.md). It reuses two
patterns that already work: the module pins (`modules.json`, [Modules](modules.md)) for the
fetch, and the voice pack publish (prepare, review, **Approve and publish**,
[Gear](gear.md#publish-a-pack)) for the send.

---

## Contents

1. [Rules this design keeps](#1-rules-this-design-keeps)
2. [What can be contributed](#2-what-can-be-contributed)
3. [What is sent and what is stripped](#3-what-is-sent-and-what-is-stripped)
4. [The opt-in and the preview](#4-the-opt-in-and-the-preview)
5. [Channels](#5-channels)
6. [From reported to proven](#6-from-reported-to-proven)
7. [How QuadCam gets the updated tables](#7-how-quadcam-gets-the-updated-tables)
8. [Recommendation and stages](#8-recommendation-and-stages)
9. [Decisions for the owner](#9-decisions-for-the-owner)

---

## 1. Rules this design keeps

- **Off by default.** QuadCam collects nothing in the background and sends no telemetry. Each
  contribution is one act by the person, on one item.
- **Built from an allow list.** A contribution is a typed record. QuadCam fills each field from
  a known value. It never sends a file or a dump and strips it afterwards.
- **The exact bytes, shown first.** The preview is the payload, byte for byte, plus a plain
  table of it. Nothing leaves the Mac until the person approves that preview's digest.
- **A person approves.** Approve is a GUI click, or the CLI at a terminal after the person
  types the item's id. No MCP call sends a contribution. An agent can prepare one and show
  the preview. This is the voice publish rule.
- **A report never unlocks a write.** Community data can inform: show a fact, a warning, an
  estimate. It never adds a row to `compat::PROVEN`, and it never changes a value a write
  guard reads. Proven stays a code change with a test, per release (`gear/compat.rs`).
- **No server to run, if a choice allows it.** Prefer GitHub, which already hosts the app, the
  releases and the voice index.

## 2. What can be contributed

| Table | Where it lives now | A contribution holds | Value to other pilots | How QuadCam gets the facts |
|---|---|---|---|---|
| Radio board profiles | `gear/radios/boards.rs`, about 50 rows, read-only unless proven | Board, firmware version, USB product string, VID and PID, the DFU device's descriptor (flash size, sector layout), the bootloader entry that worked, card layout facts (`radio.yml` schema version, sound pack folder), the switch and pot slots the radio reports | The next owner of that radio sees a profile checked on a real unit, not one written from EdgeTX source. It is the path for a board toward proven | Detection (`Connected.radio`), **Read firmware** (DFU, read-only), the card read |
| Measured USB speeds | `RadioProfile.usb_speed`, 2 of 50 measured | Board, direction (read, write), bytes, seconds, route (DFU, mass storage) | Honest time estimates for a flash, a firmware read and a card write | A read or write job already times itself |
| Compat rows | `gear/compat.rs` `PROVEN` | Product, board, version, the job, the result, and the verify step's result (read-back equal or not) | Shows which versions other pilots ran a job on with success. It is evidence for the owner's promotion to proven, not a promotion | The apply and flash reports, after their read-back |
| FC board notes | `gear/bf/boards.rs` `BoardNote` | Board, build version, when it applies (after a USB session, always), and the person's text | A known issue reaches every owner of that board before they meet it | Free text, typed by the person on the FC page |
| FC USB time limit | `bf/boards.rs` `usb_minutes` | Board, minutes on USB before the symptom, the symptom | A safer default for the USB limit cue | Free text plus a number |
| Vendor firmware feeds | `bf/boards.rs` `VENDOR_SOURCES` | Board, maker, product, the feed's address, the file prefix | Owners of other boards get their maker's own builds | Typed by the person; the address is checked later by a reviewer |
| Sim flight profiles | `sim/presets/*.json`, 4 presets, 2 fitted to logs | The profile's parameters as fitted (the 6.1 parameter set of [Sim design](sim-design.md)), the quad's frame, motor, prop and cell count, the validation table from `gear sim validate` | A fitted profile for a common quad, checked against logs, instead of an estimate | The fit and the validation run. The logs stay on the Mac |
| Voice structures | `structures` in `pack.json` and `samples.json` | Provider, model, voice id (only a voice from a public library), the line, the way that won, how many tries | A way that a voice says a hard word, so the next render of that voice spends no credits to find it | The grouped re-take |
| Risky lines | The `sample` line set, `resources/voice/sets/` | A line, provider, model, voice, what the speech check heard | A better sample set: the lines that fail most across voices | The speech check |

Not in scope here: clips, places, flight logs, radio logs and blackbox images. They hold
location and time, and nothing in them improves a shared table that the fields above do not
cover.

## 3. What is sent and what is stripped

Every record carries a small header: the record type, QuadCam's version, the schema version,
and the month it was made (not the day or time).

| Data | Sent | Why |
|---|---|---|
| Board name, maker, MCU, target, firmware version and build | Yes | The key of every table row |
| USB VID, PID and product string | Yes | The detection key. They name a product, not a unit |
| DFU flash size and sector layout | Yes | The board's guard values, to check the profile |
| Measured speeds, counts, results, the validation table | Yes | The fact itself |
| Sim parameters, frame, motor, prop, cells | Yes | The fact itself |
| Voice provider, model, public library voice id | Yes | The key of a structure |
| USB serial number, DFU serial, `mcu_id`, `signature` | **No** | They identify one unit |
| ELRS UID and binding phrase (`expresslrs_uid`) | **No** | They identify the link and let anyone bind to it |
| `craft_name`, `pilot_name`, every other `*_name`, the `# name:` header | **No** | The person's names. `bf::dump::scrub` already lists these |
| EdgeTX model names, radio owner name, device names and aircraft profile names given in QuadCam | **No** | Chosen by the person |
| Volume labels of cards | **No** | Often a name |
| File paths, the macOS user name, the host name | **No** | They name the person and the Mac |
| GPS: places, home points, coordinates in logs | **No** | Location |
| Account ids: GitHub, ElevenLabs, a cloned or private voice id | **No** | They name the person. A cloned voice may be their own voice |
| API keys and tokens | **No** | Never leave the Keychain |
| Dates and times finer than the month | **No** | A flight time plus a place finds a person |
| Raw files: dumps, `radio.yml`, models, logs, firmware images | **No** | Too much to check by eye. Firmware images also carry the maker's license |

**Free text** is the risk the allow list cannot close: board notes, symptoms, a feed's
description. QuadCam scans it before the preview and flags what looks like an email address,
a path under `/Users/`, a URL with a query, a serial-shaped token (long hex), or a coordinate
pair. A flag blocks the send until the person edits the text or confirms each one.

**The channel adds its own identity.** A GitHub issue or pull request shows the person's
GitHub account. A submissions endpoint sees an IP address. The preview says which, in one
line, before Approve.

## 4. The opt-in and the preview

### The setting

`contributions`, off by default, in **Settings > Advanced**. Off, QuadCam shows no
**Contribute…** control and offers nothing. On, it adds:

- **Contribute…** on a radio board page, an FC page, a sim profile and a sample voice card.
- One offer after a job that measured something the table lacks, for example after a firmware
  read on a board with no `usb_speed`: "Share this read speed?" with **Review…** and
  **Not now**. **Not now** does not ask again for that board and fact.

### The preview sheet

The sheet follows the voice publish review:

1. **What is sent**: a table of field, value, and where QuadCam got it.
2. **The payload**: the exact JSON, read-only, with its SHA-256.
3. **Left out**: a list of what the source held that the record leaves out, for example "USB
   serial, craft name, 2 model names". It shows the kind, not the value.
4. **Where it goes**: the channel, the public address, and the identity it shows ("your GitHub
   account, in public").
5. **Approve and send** sends exactly the bytes whose digest the sheet showed. A changed record
   refuses: "The record changed after the review."

### Surfaces

| Surface | Prepare | Approve |
|---|---|---|
| GUI | **Contribute…** | **Approve and send** |
| CLI | `quadcam-cli gear contribute <kind> <item>` prints the record and its digest | `--approve --digest D`, at a terminal, after the person types the item's id |
| MCP | `quadcam_gear_edit` `contribute`: prepares and returns the preview | None. The answer says to approve in the app |

A sent record is kept in `<gear>/contributions/` with its address, so the person can see what
they sent and find the issue or pull request.

## 5. Channels

Three options. Each one ends in the same reviewed data repo (a new public repo,
`noahkiss/quadcam-data`, like `quadcam-voices`).

### A. A GitHub issue form, opened in the browser

QuadCam opens `github.com/noahkiss/quadcam-data/issues/new?template=<kind>.yml` with the record
in the body as a fenced JSON block. The person sees it again in GitHub and clicks **Submit new
issue**. A GitHub Action checks the JSON against the schema and labels the issue. The owner
reads it and adds an `accept` label, and a second Action opens a pull request that adds the
record to the data.

### B. A pull request to the data repo, through `gh`

The voice publish route: with `gh` installed and signed in, QuadCam forks the data repo, adds
one file under `reports/<kind>/`, pushes a branch and opens a pull request. CI checks the
schema. Without `gh`, QuadCam shows the commands, or falls back to A.

### C. A submissions endpoint

A small HTTPS endpoint (for example a Cloudflare Worker) takes the record and, with its own
bot token, files it as an issue in the data repo. No GitHub account needed.

### Trade-offs

| | A. Issue form | B. Pull request via `gh` | C. Endpoint |
|---|---|---|---|
| What the contributor needs | A GitHub account and a browser | A GitHub account, `gh`, signed in | Nothing |
| Server to run | None (GitHub Actions) | None | One, with a secret token, a rate limit and monitoring |
| Review effort | One label per issue; the Action writes the pull request | Review a pull request; CI did the schema check | Same as A, plus spam the app cannot stop |
| Abuse | GitHub's own spam controls; accounts can be blocked | Same as A | Open to anyone who reads the app's code. Needs rate limits, and still floods |
| Privacy | Public, under the person's GitHub account | Same as A | No account shown. The endpoint sees the IP address; QuadCam must say whether it logs it |
| Independent reports (section 6) | One per GitHub account: easy to count | Same as A | Hard. An install id can be reset; IPs are shared |
| Payload size | The issue URL holds about 8 KB. A sim profile fits; a big record needs a paste step | No limit | No limit |
| Last step in the person's hands | GitHub's own Submit button: a second, visible consent | QuadCam's Approve | QuadCam's Approve |
| Code QuadCam already has | Opening a URL | `voice::publish` with `Gh` | None. The Worker is new infra to own |

## 6. From reported to proven

Each community row has a status. The app shows it on the row.

| Status | Means | Reached by | What it changes in the app |
|---|---|---|---|
| Built in | Written from docs or source, no unit seen | The table as it ships now | Nothing new |
| Reported | One person sent this | One accepted report | Shown with "reported by 1 pilot". Estimates use it, labelled |
| Confirmed | Several independent people agree | N accepted reports from distinct GitHub accounts, no report against it, or one report from the owner's bench | Shown as "confirmed by N pilots". Replaces a built-in value in estimates and notes |
| Proven | QuadCam tested a write on it | A test passes on that board and version, and a release adds the row to `compat::PROVEN` | Unlocks the write. Unchanged from today |

Rules:

- **Confirmed never becomes proven by count.** Proven needs a test the owner runs or a fixture
  in the test suite: for example, a contributed DFU descriptor plus a firmware read on a board
  the owner can borrow. Confirmed rows make a good queue for that work: the most-confirmed
  unproven boards first.
- **A conflicting report** (a different flash size for the same board, a failed result for a
  pair) holds the row at its current status and flags it for the owner.
- **Independence** is one per GitHub account per row. The owner's reports count as confirmed
  at once, since that bench is the reference.
- **A suggested N is 3.** A small number fits a small community; a wrong value only affects an
  estimate or a note, never a write.

## 7. How QuadCam gets the updated tables

The `modules.json` pattern, with a signature added.

1. **The data repo builds a bundle.** On each merge to `main`, an Action builds
   `community.json`: every accepted row with its status and report count, a schema version and
   a sequence number. It signs the bundle with minisign, using a key that is held only in that
   repo's secrets and is separate from the app updater's key. It attaches `community.json` and
   `community.json.minisig` to a release `data-<sequence>`.
2. **QuadCam checks the signature.** The app ships the public key. It reads
   `releases/latest/download/community.json` and the signature, checks both, and refuses a
   bundle with a bad signature, an unknown schema version, or a sequence lower than the one it
   has. A refused bundle leaves the cached one in place.
3. **When.** At launch and with the firmware check: on request, or once a day with
   `firmware_check` set to `daily`. One check at wake catches up; missed days are not replayed.
   A setting, `community_data`, turns the fetch off. It is on by default, since reading public
   data sends nothing but the request; the README's network list says so.
4. **Where it lands.** `~/Library/Caches/app.quadcam/community/`. QuadCam keeps the last good
   bundle and works offline with it, or with the compiled tables alone.
5. **How it merges.** The bundle is an overlay on the compiled tables:
   - It may add: a measured speed, a board note, a look-alike, a quirk, a sim preset, a voice
     structure hint, a reported or confirmed status.
   - It may not change: `compat::PROVEN`, flash sizes, DFU addresses, firmware formats, write
     routes, and any other value a write guard reads. A bundle row that tries is dropped and
     logged.
   - A vendor feed row from the bundle shows as a link only. QuadCam downloads from a vendor
     feed only when the compiled table lists it. A new download source needs a release.
6. **Releases fold it in.** Each QuadCam release copies the confirmed rows into the compiled
   tables, so a fresh install without a network has them too.

## 8. Recommendation and stages

**Recommend A, with B as an option for `gh` users, and no endpoint.** A needs no server and no
`gh`, its last step is a visible GitHub Submit, and GitHub accounts make independent reports
countable. B costs little, because the voice publish already has the `gh` code. C adds infra to
run and a spam problem for a small gain: pilots without a GitHub account. Revisit C only if the
issue count shows that the account is the barrier.

| Stage | Ships | Acceptance |
|---|---|---|
| 0. Data repo | `quadcam-data` with the schema, an issue form per kind, the check Action, the `accept` Action, and the owner's own reports as the first rows | An issue with a bad record gets a label and a comment; an accepted one opens a pull request that passes CI |
| 1. Contribute, two kinds | The `contributions` setting, the record builder, the free-text scan, the preview sheet, channel A. Kinds: radio board facts and USB speeds | Unit tests: each source with a serial, names, a path and a UID gives a record without them. The preview's digest matches the sent bytes. MCP prepares and never sends |
| 2. Fetch | `community.json`, the signature check, the overlay, statuses shown on board rows | A bundle with a bad signature, an old sequence, or a row that touches a guard value is refused or dropped, in tests on fixtures |
| 3. More kinds | FC board notes, USB time limits, compat results, sim profiles, voice structures and risky lines; channel B | Each kind has a strip test and a schema in the data repo |
| 4. Promotion work | A page in the data repo that ranks unproven boards by confirmations; releases fold confirmed rows in | The owner picks the next board to prove from it |

## 9. Decisions for the owner

1. **Channel.** A alone, A plus B (recommended), or C as well.
2. **N for confirmed.** 3 is suggested. Do the owner's own reports count as confirmed at once?
3. **The fetch default.** `community_data` on by default (recommended: it sends only a request)
   or off.
4. **The data repo.** A new public `quadcam-data` (recommended), or a folder in
   `quadcam-voices` renamed to a general data repo.
5. **The signing key.** A new minisign key for data (recommended), or reuse the updater's key.
6. **First kinds.** Radio board facts and USB speeds first (recommended: they unblock the 48
   unproven boards), or sim profiles first.
7. **Free text.** Allow it in board notes with the scan, or limit notes to picked symptoms from
   a list.
8. **The record's month.** Keep a month on each record, or send no date at all.
9. **Licenses.** The data repo's license for contributed facts (CC0 suggested), and a line in
   the preview that the person agrees to it.
