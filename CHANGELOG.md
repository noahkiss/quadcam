# Changelog

User-facing changes in each release, newest first. Release notes on
[GitHub](https://github.com/noahkiss/quadcam/releases) hold the test steps for the later ones.

## Unreleased

- **Samples play again.** Sample lines, **Play all** and the packs on this Mac play their sound again; in 0.20.0 and 0.20.1 they played nothing. A line that cannot play now says so in a message, once per **Play all**, instead of being skipped in silence.

## 0.20.1 (2026-10-12)

- **Update checks every hour.** QuadCam now checks for a newer release when it opens and every hour while it runs, not once a day. **Check for Updates** under the version in Settings checks now, from any Settings page.
- **Check for module updates.** The Modules pane's button has a name of its own, so it is not confused with the app's **Check for Updates**.

## 0.20.0 (2026-10-12)

- **Price and render several voices at once.** The Voice studio's cost now covers every ticked voice with every ticked model, not only the first: the tiles show the total and **Cost by pack** lists each pair. **Render packs** makes one pack per voice and model in one confirmed run; a failure keeps the packs made before it. CLI `gear voice render-packs`; MCP `voice_estimate` and `voice_render` take `voices` and `voice_models`.
- **Progress and Stop for paid runs.** **Sample and pay** and **Render and pay** show a progress bar (batches done of the total, and the voice and model) and a **Stop** that ends the run after the current batch. A batch already made is never billed again. MCP `stop` with the handle `voice-studio` stops a run in the app.
- **Browse voices.** The Voice studio's **Browse voices…** searches ElevenLabs' Voice Library by words, language (English at first), accent, gender, age, use and category, a page at a time. Each voice shows its labels, languages, a preview, any rate of its own (the estimate prices it) and its notice period. **Add…** asks, then adds the voice to your account's My Voices; a full account says it has no free voice slot. **Voice ID** and **Look up** find one voice by id. CLI `gear voice library` and `gear voice add`; MCP `voice_library` and `voice_add`.
- **Samples by voice.** Samples now have their own **Samples** section on the Voices page: one card per voice, model and speed, with the voice's description, accent and language, and each sample line as a small play chip. The cards wrap at any window width. A line the speech check heard wrong says so on its voice's card. Samples stay: they live in the gear folder (`voice-samples/`), not the cache, and the page reads them when it opens. Samples from 0.19 and earlier move there once. The packs on this Mac show as cards too. **Delete** removes one voice's samples, **Delete all…** every sample; both ask first, and sampling again is free. CLI `gear voice samples`; MCP `voice_samples` and `voice_samples_delete`.
- **Play all.** Each card's **Play all** plays its lines in order and marks the one playing; **Play all voices** plays every card. **Stop** ends it.
- **Flag a take.** The flag on a sample line, or on a line in a pack's speech check, records a reason (mispronounced, clipped, wrong tone, noise, other) and a note beside the take. Flags show on the card and in the pack's library row; **Clear flag** removes one. A flagged studio take stays out of later renders with the same seed, and **Re-take flagged** makes it again with another seed, priced first and paid only after you click. CLI `gear voice flags`, `gear voice flag` and `gear voice retake`; MCP `voice_flags`, `voice_flag` and `voice_retake` list each flag with the voice, model, speed, seed, line, text, heard text and file.
- **Accent and language columns.** The studio's voice table shows each voice's accent and languages in columns of their own, filters by them and by name, and plays each voice's preview.

## 0.19.0 (2026-10-11)

- **Map, off by default.** Settings > Places > **Show the Map page** adds **Map** to the sidebar: your places and clips on an Apple map (Apple Maps on your Mac, no account). A place's pin shows its clip count, and a click shows its clips in the library. A clip with a location but no place has its own pin, which opens the clip. **Map**, **Hybrid** and **Satellite** pick the look; **Show all** zooms to every pin. The `map_view` setting, in the CLI and MCP too. See [The map](docs/library.md#the-map).

## 0.18.0 (2026-10-11)

- **Shift time of many clips.** Select clips, then **Shift time…**: set the real date and time of one clip, and the rest move by the same offset. A clip that a radio log in the log folder matches takes the log's time instead, and the table says **Radio log**. The table shows every new time before you shift; a new day moves the files. CLI `library shift` (`--yes` writes); MCP `quadcam_library_edit` `shift` (`confirm` writes). See [Shift the time of many clips](docs/library.md#shift-the-time-of-many-clips).
- **Moment heat strips.** A thin strip on each card's thumbnail and on the clip timeline's Moments row is stronger where the clip's moments are.
- **Best moments.** A new sidebar item lists the moments of every clip, best first. Select one to open its clip and play from there. The day summary's best moments use the same list, without dead air.
- **Sun tags.** A clip with a place and a time of day shows where the sun stood: dawn, sunrise, golden hour, sunset or dusk. QuadCam works it out on your Mac; nothing leaves it. The grid shows a sun icon, the list a **Sun** column, an open clip its tag under **Details**, and search finds it. CLI `library list` and MCP `quadcam_library` carry `sun`. See [Sun and weather](docs/library.md#sun-and-weather).
- **Weather, off by default.** Settings > Import > **Look up weather** gets the wind and temperature at each clip's place and hour from Open-Meteo (free, no account), once per clip, and saves them in the file. It sends the place, rounded to about 1 km, and the date; off, QuadCam sends no weather request. An open clip shows it under **Details**. CLI `library weather`; MCP `quadcam_library_files` `weather`; the `weather` setting.
- **Use current location.** Settings > Places > **Use current location** adds a place where your Mac is, from macOS Location Services, named by Apple's geocoder. The first use shows macOS's permission prompt. CLI `places here`; MCP `quadcam_places` `here`.
- **Name from coordinates.** The tag button on a place row names its latitude and longitude with Apple's geocoder. CLI `places reverse LAT,LON`; MCP `quadcam_places` `reverse`.

## 0.17.1 (2026-10-11)

- **The MCP server follows updates.** When QuadCam updates, an agent's `quadcam-cli mcp` moves to the new version in the same process, between calls, and tells the agent its tool list changed. You no longer restart the agent after an update. It never switches during a call or a job, and never to an older version; `QUADCAM_MCP_RELOAD=0` turns it off. See [Updates](docs/mcp.md#updates).
- **Restart to update in Settings.** When a newer version is known, Settings > Advanced shows **Restart to update** (download with progress, signature check, install, restart) and the release notes link beside **Check now**, as the banner does. It waits while a job runs, as the banner does.
- **Updates find the newest release.** Once QuadCam knew of an update, it stopped checking, so the banner could not move on to a newer release. Every check now runs while an update is known; a newer release replaces the known one in the banner and Settings, and drops an older download. **Restart to update** checks once more before it downloads, so it installs the newest release. QuadCam never installs a version at or below the one running.
- **Development: tests never reach the real gear folder.** A process started by cargo whose `HOME` is the account's own home now refuses QuadCam's support folder (settings, gear folder, control socket) unless `QUADCAM_PATHS=real`. A test core built without a settings file once wrote into the real gear folder. `cargo tauri dev` needs a temporary `HOME` or `QUADCAM_PATHS=real`.

## 0.17.0 (2026-10-11)

- **QuadCam Voices.** Voice packs now have a home: [noahkiss/quadcam-voices](https://github.com/noahkiss/quadcam-voices), with a site at <https://noahkiss.github.io/quadcam-voices/> that lists each pack and plays a few of its lines in the browser. The `voice_index` setting reads its index by default; an empty setting turns it off. Packs download from the repo's releases, still checked against the index's hash.
- **Preview before you install.** On the Voices page, **Preview** on an index pack plays its preview lines, each checked against its hash. CLI `gear voice preview`; MCP `quadcam_gear` `voice_preview`.
- **Publish a pack.** **Publish…** on a pack you rendered prepares it for QuadCam Voices: the zip, about ten AAC preview lines and the index entry. It refuses until the speech check heard every line right or you accepted each line it flagged, and it refuses packs made with macOS `say`. The review lists every line with a play button and what was heard, accepted lines first, and a random spot check of 10 to 15 lines. Only **Approve and publish** publishes: it creates the release and opens a pull request for the index with GitHub's `gh`, or shows the commands to run. An agent can prepare and show the review (MCP `quadcam_gear_edit` `voice_publish`, CLI `gear voice publish`) but cannot approve; `gear voice publish --approve` works only at a terminal. See [Publish a pack](docs/gear.md#publish-a-pack).
- **Kokoro packs.** [docs/gear.md](docs/gear.md#publish-a-pack) and the voices repo describe rendering a pack with a Kokoro server on this Mac through the `openai` provider.
- **Highlights from video analysis.** On macOS 27 or later, QuadCam asks macOS's video analysis which stretches of each clip are interesting, in the background after a card loads; an import never waits for it. A clip with no radio-log moments (a DJI clip, an analog clip without a log) shows them as **Highlight** moments; a clip with radio-log moments keeps only those. The analyzer's key frame becomes the clip's picture in the Import sheet and the library. Each moment in the list now says where it came from. **Find highlights again** in a clip's menu, `quadcam-cli library analyze-video` and MCP `quadcam_library_files` `analyze_video` analyse clips already in the library. Settings > Import > **Find highlights in new clips** turns it off. On an older macOS nothing runs.
- **On-device clip names.** On a Mac with Apple's on-device model (macOS 26 or later, Apple Intelligence on), QuadCam suggests a short name, keywords and a note for each clip after a card loads, from the clip's date and time of day, place, aircraft, length, moments and flight, and on macOS 27 a frame from mid-flight. Clips of one outing share one name with pack numbers (`morning loops pack 1`, `pack 2`), continuing after the day's numbered clips in the library. It fills only empty fields: a name you typed, or an agent suggested, stays. Its values carry an **on-device** badge; a name you typed shows the model's under it with **Use**. **Suggest names** in the Review step asks again. Offline and free. **On-device suggestions** in Settings > Import turns it off. CLI `names`; MCP `quadcam_suggest` `on_device`.
- **Summarize day.** The session report's **Summarize day** adds a short paragraph about the day from the same model; Copy and Save include it. CLI `gear report --summarize`; MCP `quadcam_gear` `session_report` `summarize`.
- **For developers:** `quadcam-intel` adds `video` (MediaIntelligence highlights and key frame, macOS 27) and `suggest` (Foundation Models, weak-linked: clip names, keywords, notes and a day's summary).

## 0.16.0 (2026-10-11)

- **Speech check of voice lines.** On macOS 26 or later, QuadCam listens to each line of a voice pack with the speech recognizer built into macOS and compares what it heard with the line's text (spelling, numbers, case and punctuation aside). It runs after every render and sample, and on request: **Check speech** on a Voices row opens the pack's check, with the heard text and a result per line. In a pack the Voice studio rendered, a line heard wrong leaves the pack as a re-take: **Re-take** prices it and renders it again, and a render with the same seed sends nothing. **Accept** keeps what was heard and puts the line back. It is free, offline, and never calls ElevenLabs. **Install speech assets** asks macOS for its English speech assets when they are missing. CLI `gear voice check`; MCP `quadcam_gear_edit` `voice_check`.
- **For developers:** `quadcam-intel`, a Swift helper in `intel/`, carries Apple's Swift-only frameworks (Speech now). The build makes it and the app bundles, signs and notarizes it beside `quadcam-cli`; on an older macOS or without it, its features are unavailable. CI and releases run on GitHub's `xcode-27` image (a public preview).
- **A native window on macOS 26 and later.** The sidebar sits on Liquid Glass and runs the window's full height, with the window buttons over it. The toolbar spans the content beside it, and you drag the window by the toolbar or the strip above the sidebar. Focus rings take the system accent color. macOS 13 to 15 keep the standard title bar.
- **Pop-up menus** such as **Sort by** open as a list drawn like the app's other menus, with a check beside the current choice, where the system's web view supports it (macOS 27). Elsewhere they open the system menu.
- **For developers:** `src-tauri/src/glass.rs` sets up the window chrome; `html[data-glass]` marks the page for it. `e2e/glass.spec.ts` covers both layouts. `docs/development.md` "Window chrome and theme" describes it.

## 0.15.0 (2026-10-11)

- **Connected as a dashboard.** Tiles count the devices plugged in, the ones that need attention, the ones working and the changes staged for them. Each device is a card with its state and its next step: **Save…**, **Review…** or **Import…**.
- **Devices as a table:** name, kind, state, last backup, firmware (installed → newest) and staged changes.
- **Device Overview.** The header adds the firmware. Summary tiles show the firmware against the newest, the last backup, the staged changes and the latest blackbox pull (FC), the aircraft count (radio), or the new clips, last card check and free space (DVR and goggles cards); a tile opens its section. The facts sit in a **Details** card, two columns wide. The row of section links under them is gone: the tabs above do that.
- **Forget…** moves into the **…** menu on a device page. The header's buttons are plain buttons, primary last.
- **One left edge.** Every Gear page and device section lines up with the page title: the Bench, OSD, Rates, Switches, Models, Checklists and Voice sections lost their extra inset. Empty lines are left-aligned under the title, not centred.
- **For developers:** shared `PageHeader`, `EmptyState`, `FactList`, `StatusDot` and `MoreMenu` in `app/src/components/dash/`, and the `--s-5` spacing token the Apply sheet, Bench and Firmware already used. `docs/development.md` describes how to build a dashboard page.
- **Pack up as a dashboard.** Tiles show the checks that pass, the packs charged, card space and backups. The checks are a table with a state dot per row.
- **Flights as a dashboard.** Tiles sum up the day: flights, air time, the longest flight, the worst link and dropouts. The Pack column is a compact pop-up button. A flight's details and the range chart sit in cards. When every flight on a chart was on one day, the axis shows the first and last time, not the same date twice.
- **Session report** shows its numbers as tiles, and pack use and crashes as tables.
- **Packs:** tiles for packs in use, charged, weak and cycles. **Mark charged** and **Edit…** are buttons. **Add pack…** and **Add type…** sit in the page header.
- **Repairs:** one table grouped by aircraft, so the columns line up, with tiles for open and repaired crashes and the last crash.
- **Sims as a table.** Tiles count the sims found, out of date and running. Each sim is a row with its state and files, and each profile a row under it with the maximum rates, the difference from the quad and a clear **Sync…** button. The Rates segment shows the same table in a **Sims** card with **Open Sims**.
- **Rates and OSD say where they came from:** "From backup" and its time, or the file names, with **Open dump…** on the right. The rate table lines up its numbers. The OSD shows its problem count as a badge.
- **Switches:** tiles count the controls, modes, conflicts and unmapped switches. Conflicts come first in their own card; modes and unmapped switches are tables.
- **Controls and Sim radio** use the page header and cards. Controls keeps its 24 buttons under **Buttons**. Sim radio lists its switches in a table with **Change**, and the calibration in a **Calibrate** card.
- **Firmware as a dashboard.** **Check for updates** sits in the header with the last check under the title. Tiles count the devices, the updates and the devices up to date, and show the last check. The table shows **Installed → newest** and a status dot; before the first check a row says **Not checked**, so **Unknown** means the version is unknown. **ExpressLRS (preview)** and **Read radio firmware** are cards, and the read steps are numbered inside the card. An ELRS device's options open under the table.
- **Radio Models and Checklists** say where the model came from in words: "From the radio's card" or "From backup · Oct 6, 2:20 PM", not a backup id. The checklist is a card with its empty line inside.
- **Radio Splash** is a card: the preview sits in a frame of the radio's 128 × 64 screen at twice the size, beside the controls. A line under **Flash with splash…** says why it is off.
- **Aircraft on this radio** is a card: the add picker is in its head, the aircraft are in the shared table, and the selected model has a **Selected** badge.
- **Settings > Gear** reads as a macOS form: each group's title on the left, its controls on the right.
- **Bench as a dashboard.** **Copy as Markdown** and **Copy settings…** sit on the right of the title. Tiles count the staged and ready changes, the devices, the ones plugged in and the applied Try changes waiting for **Keep**. Each device is a card with its state and **Review…**, and its changes as a table: title, edit, status, who staged it, **Review…** and **Discard**.
- **Changes as a table.** An FC's or radio's **Changes** uses the Bench's table, with tiles for staged, ready, verified and the last apply. Its **Review…** is primary and counts the changes. A row's status can now be set there too, and applied Try changes show **Keep** and **Revert…**.
- **Backups without tabs in tabs.** Tiles show the count, pinned, last backup and size. The backups list sits on the left; the selected backup's changes from the one before and its files sit on the right, both at once.
- **Blackbox tiles.** **Flash used** shows the FC's flash use against its size while it is plugged in, and whether a stored pull holds it; tiles also show the pulls, the last pull and its logs. The logs table's column reads "Flight, paired by order" in place of the paragraph under it.
- **Storage as a dashboard.** The gear folder is the subtitle with **Show in Finder**; **Import backups…** and **Prune now** sit in the header; tiles show the total, backups, stored files and radio logs.
- **CLI and MCP:** `gear blackbox summary [--port P]` and MCP `quadcam_gear blackbox_summary` read an FC's flash size, bytes used and ready state over one MSP message, and say whether a stored pull holds them. No pull, no cue.

## 0.14.1 (2026-10-11)

- **Voice studio: a take that no longer cuts is removed.** A line reused from an earlier take whose files cannot be read or cut no longer fails every render: QuadCam removes the take before it sends anything, and the next estimate prices its lines. Before, the render failed on that take every time until its cache file was deleted by hand.
- **Voice studio: nothing is sent that the estimate did not price.** A batch the estimate counted as cached but whose take cannot be read is removed, not sent; the next estimate prices it. A render that fails part way keeps the batches it made, so their lines are not billed again.
- **Import takes:** an imported take no longer shares its cache key with a one-line batch under the carrier `{line}`, and its start is not checked for a carrier's last sound (it had none). Takes imported by 0.14.0 keep working; importing the folder again files them under the new key.

## 0.14.0 (2026-10-11)

- **Voice studio: a line is billed once.** A line any earlier render or sample spoke in the same voice, model and speed is cut again from that take, whatever set it comes in. Before, the cache knew whole batches only, so adding a set, rendering a second aircraft set (they share the radio's prompts, numbers and units) or rendering after a sample sent lines again that were already paid for. A re-take with a new seed now sends only the lines that need one.
- **Voice studio: line sets say what they hold.** Each set shows a one-line description, and the sets group into **For your quads**, **Any aircraft** and **Other aircraft**, from the aircraft your profiles name (quads when none does). The sets for your aircraft start ticked: for quads, FPV quad and FPV extras.
- **Voice studio: progress.** With one voice and one model ticked, each set shows "N lines · M rendered", and the cost line says how many lines are rendered and how many are left to render.
- **Voice studio: compact samples.** Sample results show one row per line in a grid. With several voices or models, a numbered list names the pairs and each line has a numbered play button per pair.
- **Voices page as a dashboard.** A summary row shows the ElevenLabs credits (with a meter), the packs on this Mac, the radios with a pack and the lines rendered per voice. The packs, voices, models and line sets are tables: each voice and model shows how many lines it has rendered, each model its price per 1K characters with a promo badge. The cost shows as tiles: lines to render, characters, cost, credits after. Every paid call still asks first and pays only for the plan it priced.
- **Import takes…** copies a folder of earlier ElevenLabs takes of single lines (a `.json` with text, voice, model, speed and seed beside a `.pcm`) into QuadCam's cache. They count as rendered and their lines are never sent again at that voice, model and speed. The folder is only read. A **Speed** control in the studio renders at the speed such takes were made at. CLI `gear voice import-takes FOLDER [--dry-run]`, MCP `quadcam_gear_edit voice_import_takes`.
- **Cut version:** a line reused from an earlier take is cut and checked again by the current cut, so a cut change reaches every reused line at no cost.
- **CLI and MCP:** `gear voice sets` takes `--voice` (an id) and `--model`, and MCP `voice_sets` takes `voice` and `voice_model`, for the per-set counts; both list the aircraft flown, the first picks and the lines each voice, model and speed have rendered (`rendered`). The estimate adds `rendered`, `retakes` and `to_render`.

## 0.13.2 (2026-10-11)

- **Library:** the day summary keeps each number and its label on one line, so the stats line up (`3.20 V` no longer wraps). Best moments chips shorten a long clip name with an ellipsis and show it in full on hover. A narrow window puts Best moments below the stats.
- **A Draft change no longer applies.** The core refuses a Draft at the plan (`draft`, new refusal code), for an FC, a radio card and ExpressLRS options alike; before, only the app's **Review…** was off. An ExpressLRS options change marked Read first now refuses too (`read_first`), as FC and card changes did.
- **CLI help:** `gear apply --help` names every device a staged change applies to (an FC, a radio card, ExpressLRS options); `gear changes --status` lists every status (`try`, `read_first` and `reverted` too); `gear firmware --help` names the Betaflight FC flash beside the EdgeTX one.
- **Voice studio cuts.** A line no longer starts with the hiss of the carrier's "is" (the "s" heard before "Battery critical"), and its end keeps the word's tail. ElevenLabs' character times can run 0.1 s off, so QuadCam now places each cut in the quiet the audio shows between the carrier and the line. A check flags a cut that starts with the carrier's hiss and a gap. To re-cut, **Sample** again and **Render pack** again with the same voice, model, sets and seed: both cut from the cached batches and cost no credits.

## 0.13.1 (2026-10-11)

- **Development:** the integration tests build as one test binary per crate (`src-tauri/tests/it/`), so a test build links the app 10 times instead of 57. No change to the app.

## 0.13.0 (2026-10-11)

- **In-app updates.** QuadCam checks for a newer release when it opens and once a day, and **QuadCam > Check for Updates…** checks at any time. A banner offers **Restart to update** (download with progress, signature check, install, restart) or **Later**, with a link to the release notes. It never restarts during an import, a card or device write, or a flash. **Settings > Advanced > Check for updates automatically** (`update_check`) turns the automatic check off. A failed check never opens a dialog.
- **Homebrew:** the cask is now marked `auto_updates`, so `brew upgrade` leaves QuadCam to its own updater. `brew upgrade --cask --greedy quadcam` still updates it through Homebrew.
- **Agents:** `quadcam_status` reports the app's update state in `status.update` (read-only). `quadcam_settings` takes `update_check`.

## 0.12.1 (2026-10-11)

- **Secrets are write-only everywhere.** QuadCam shows a stored secret (the ElevenLabs key, `google_places_key`, `tts_key`, `elrs_binding_phrase`) only as its hint: the first and last characters and the length, such as `sk_9…8d4d (51 chars)`, never more than half the value. It replaces "ends in 3f9a" and "(set)" in the Voice studio, the Settings window, the ELRS section, the CLI and MCP. `settings show` and the settings answer keep `"(set)"` in `values` and add `secrets` with each hint; the key status adds `prefix`, `suffix` and `length`.
- **Replace… for a stored secret.** The Google Places key, the binding phrase and the ElevenLabs key show the hint and **Replace…**; **Keep current** closes the field.
- **CLI `settings set-secret KEY`** reads a secret from stdin (hidden at a terminal), for example piped from a password manager. `settings set` of a secret still works and notes that other processes can read a command's arguments.
- **MCP `voice_key_set`** (`quadcam_gear_edit`) stores the ElevenLabs key; the answer shows only its hint. `quadcam_settings` `read` lists the secrets' hints. Both tool descriptions say that a value passed through MCP sits in the agent's transcript.
- **Voice pack install.** An index entry without a hash no longer installs. The index must be an https address or a file; plain http works only on this Mac. A zip with a symbolic link, or a folder entry with `..`, refuses before anything is written, and QuadCam checks every unpacked file is a plain file.
- **Voice keys.** Each provider reads its own key: `QUADCAM_ELEVENLABS_KEY` (the older `QUADCAM_TTS_KEY` still works, for ElevenLabs only) and `QUADCAM_OPENAI_KEY`, then the `tts_key` setting, for an openai server. Before, `QUADCAM_TTS_KEY` went to any provider, a remote openai server included.
- **ElevenLabs renders only in the Voice studio.** With `tts_provider` `elevenlabs`, a line-by-line render, `build-pack` and **My text…** refuse and point to the studio, which prices a render and checks the credits first.
- **My text… on a provider that may charge** shows the characters and **Render and pay**, which confirms with the digest of that text. CLI `gear voice edit --dry-run` and `--digest`; MCP `voice_edit` `dry_run` and `digest`. A paid confirm needs the digest.
- **Studio packs are named by the voice id.** Two voices with one name (a clone named like a premade voice) no longer share a pack. A pack from 0.12.0 or earlier moves to the new name at the next render of that voice and model; the radios that chose it, and line overrides that take its takes, follow it.
- **Samples** keep the card path in their file names, so `armed.wav` and a script's `armed.wav` no longer overwrite each other.
- **Recorded credit rate.** QuadCam keeps the rate ElevenLabs reports only within 5% of the model's credits a character, so a plain character count never doubles the estimate. It logs every count in `voice/charcost.log` in the cache.
- **Deleting a voice pack** also clears each line override that takes its take (`overrides` in the result).
- The settings and MCP descriptions name `elevenlabs` as a `tts_provider`.
- **Radio verify over USB serial.** A backup file in a folder the radio's CLI cannot list (a space in the folder name, an `ls` that fails) or with a name that is not plain ASCII is reported as not checked (`not_checked`), not as missing. A folder the card lacks still counts as missing.
- **Radio backups with a reset clock.** A backup reads again every card file dated before 2020, and every YAML file once a radio log is dated before 2020. Before, a model edit of the same length that got the same time from a reset radio clock kept the old bytes in the backup.
- **EdgeTX flash and firmware read are jobs.** Gear status lists a running radio flash or **Read firmware** (`dfu:<serial>`), and a second flash or read of the same radio refuses with `port_busy` instead of failing on the USB claim.
- **EdgeTX flash: the right radio.** Before the erase, a flash compares the board the radio's current firmware names with the saved radio's board, and refuses another board. This also covers a DFU device not yet linked to a radio. **Read firmware** says when the image names another board than the radio you picked.
- **Betaflight flash: the right FC.** Right before its backup, a flash reads the FC on the port again. Before, it trusted the port's last identity, so another FC plugged into the same port was backed up under the planned FC before the bootloader step refused it.
- **Betaflight adjustments.** A staged `adjrange` edit keeps the slot's center and scale. Before, it wrote one word too many and the old select channel became the center.
- **EdgeTX flash: one proven release.** A radio flash needs the exact release QuadCam has proven on the board (the Pocket on 2.12.4), not any 2.12 version, and a prerelease tag (`2.12.5-rc1`) always refuses. **Flash …** on the Firmware page follows the same rule.
- **Agent requests wait their turn.** An agent's apply or flash request, or **Apply on connect**, no longer replaces a sheet that is open, such as your own apply while it writes. The request waits, the sheet says so, and it opens when you close that sheet. A result always lands in the sheet it started from.
- **Approved requests show their outcome.** After you click **Apply** on an agent's or **Apply on connect**'s request, the sheet stays open and shows the progress, the result and **Restore backup**. Before, it closed at the click.
- **ExpressLRS flash requests.** An agent's ExpressLRS flash opens a sheet titled **Flash** that says what the flash does (the chip's flash kept as a backup, esptool verifies). Before, it described an options write.
- **Settings > Gear** offers **Apply ready changes** only for FCs and radios, the kinds it runs for.
- **Changes:** **Review…** is off for a Draft or Read first change, as on the Bench.
- **Splash:** **Make firmware…** is now **Flash with splash…**.

## 0.12.0 (2026-10-11)

- **Sim sync:** two profiles in one sim file (for example two Liftoff profiles) now sync together. Before, the second write put the first profile back to its old rates.
- **Sim sync plan:** the check "Rewrites unchanged" is now "Reads back as written", which is what it checked.
- **Import card:** the unmount after an import, a later card step and **Safe to remove** find the card by its volume UUID first. If the card was pulled and another disk took its disk number, QuadCam leaves that disk alone.
- **Delete clips after import:** when the card does not mount again for the delete, each clip's result says so. Before, it said the clip was no longer on the card.
- **Docs:** `gear blackbox pull` erases with no `--yes` when **Erase blackbox after download** is on; MCP `blackbox_pull` needs `confirm=true`. The CLI guide now says so.
- **Surface guard:** a new MCP tool's required parameters no longer fail the 1.0 surface test, so the documented baseline update works. The guard now also checks the keys of object parameters (settings names under `values`, profile `fields`, cut and suggestion items) and fails when a value list narrows a parameter or flag that took any value.
- **ExpressLRS (preview) guards.** An ELRS job through an FC refuses an FC that does not identify itself over MSP as the saved FC. A flash checks the device first: a receiver must answer a CRSF ping with the name read before, and its bootloader reply must name the planned target (an empty reply or a bare `UNIFIED` refuses); a radio must be the board the read went through, before its module's boot pin is held. A receiver flash through an FC runs esptool at the receiver UART's 420000 baud, not 460800. An options apply refuses a choice list that changed since the read, and reads the device again after each write. A flashed image now carries the target's hardware overrides from `targets.json` (such as its power table) and the prior target name in capitals, as ExpressLRS's configurator writes them; a target with a screen logo refuses. The `quadcam_gear_edit` description now says that `elrs_read` stops a radio's RF output.
- **A radio flies many aircraft.** A radio's page lists **Aircraft on this radio**: each aircraft profile that names the radio, its EdgeTX model, whether that model is on the card or in the latest backup, and which one the radio selects. **Add aircraft** and **Remove** set the profile's radio, and **Settings > Aircraft** has **Radio** and **EdgeTX model file** for the same link. An FC keeps its one aircraft.
- A radio's own aircraft link from 0.11 or earlier moves onto the profile the first time QuadCam reads `gear.json`. A link the profile cannot take stays in the radio's entry as `legacy_aircraft`.
- CLI `gear devices save --aircraft` and MCP `device_save` `aircraft` add the aircraft to a radio. `gear devices` and MCP `devices` list a radio's aircraft in `radio_aircraft`; a radio's `aircraft` is the one whose model the radio selects, or empty.
- **Voices page** (Gear > Voices). The Voice studio and the voice packs moved out of a radio's Voice segment. A library lists every pack on this Mac with its model, line sets, lines, size and date, and the lines that need a re-take, plays a sample, and deletes a pack after a confirmation that names the radios that chose it (their choice is cleared; the raw takes stay unless you tick the box). **Apply to radios…** stages the pack on each EdgeTX radio you tick, or **All radios**, and shows which cards are connected now; the others apply when plugged in. A radio's Voice segment keeps its pack picker, the per-line overrides and its apply. CLI `gear voice choose --all-radios` and `gear voice delete`; MCP `voice_choose` with `radios` or `all_radios`, and `voice_delete`.
- **USB timer.** The FC's USB heat timer keeps counting while the port drops out for a reboot, USB disk mode or a flash. Before, every job that rebooted the FC started it again at zero.
- **USB heat checks.** A flash, an FC apply and a blackbox pull read the battery before their heat check. A battery in with no timer counting (reads paused, or just plugged in) refuses a flash and is a warning on the others.
- **Betaflight flash reports.** A flash that fails after the erase points to Betaflight Configurator to flash an official build, not to a second QuadCam flash. A flash that fails before the erase says nothing was erased and starts the old firmware again. The sheet and the guide list the firmware copy step.
- **Active profiles.** The settings a Betaflight flash carries over, and an FC Restore, select the PID and rate profile the FC had again. Before, the FC ended on profile 0 after a flash.
- **FC range check.** The check before an FC write reads the `get` answer of the setting itself. Betaflight's `get` also lists settings whose names contain the name, and their ranges could refuse a valid value.
- **Blackbox over USB disk mode.** A pull through the setting reads over MSP when the disk's files do not verify as the flash, instead of failing.

## 0.11.2 (2026-10-10)

- **Voice studio: pay only for what was priced.** **Sample and pay** and **Render and pay** confirm the exact plan the question priced, by its digest. A picker change closes the question. The CLI takes `--digest`, MCP `digest`.
- **Voice studio: re-takes.** A cut that fails a check (silent, too short, too long) no longer goes into the pack or onto the radio. The result lists it as needing a re-take, and **Re-take** renders again with a new seed, paying only for the batches that hold one. MCP takes `seed`.
- **Voice studio: no bad take kept.** A batch whose timestamps do not match its text is not cached, so the next render asks again instead of failing from the cache. A paid request is no longer retried after a server error, which may already be billed.

- **Safer card writes.** A card write that macOS does not finish in time is now waited for before QuadCam puts files back; if it is still running, QuadCam leaves the card mounted and names the backup to restore. "The card is as it was" now shows only after every file read back as before.
- **Model names and card paths.** A model name with `/`, `..` or a leading `.` is refused, since its checklist file is named after it. The card writer refuses any path outside the card.
- **Mount kept.** An apply that refuses no longer unmounts a card you mounted with **Mount**.
- **Clean ._ files** deletes only on a detected card or an EdgeTX card folder.
- **Firmware copies** made in the same second no longer replace each other.

## 0.11.0 (2026-10-10)

- **Blackbox.** The FC page has a Blackbox segment. QuadCam reads the flight controller's blackbox flash over MSP, or from its USB disk, and keeps each pull as a record. It pairs logs with flights by order. Erasing the flash needs a confirmation. CLI and MCP have matching actions.
- **ExpressLRS (preview).** Read an ExpressLRS device, stage changes to its options, and plan and apply a flash. The tools show on the Firmware page when **Show the ELRS tools** is on in Settings > Gear.
- **Betaflight flashing (preview).** The Firmware page plans and applies a Betaflight flash behind the **Betaflight flashing (preview)** setting: backup, bootloader, DFU flash, then your old settings.
- **Voice line sets.** The voice studio offers a full EdgeTX English set (747 prompts) and sets for aircraft kinds, cars and scripts, with more FPV extras.

## 0.10.0 (2026-10-09)

- **Voice studio** (Gear > your radio > Voice). Paste an ElevenLabs API key (stored in the macOS Keychain), pick voices and models, see your credits and a cost estimate, render a sample grid, pick line sets, render a pack and apply it. Raw takes are cached, so cutting again costs nothing.
- **Automatic mount and unmount** for every card job: import, backups, checks, repair, apply, card prep and cleanup. The card unmounts when the job ends.
- **Card prep** in the app: **Prepare card…** on the device page and in the DJI Finish step.
- **Sims page** with an "Out of date" badge, and **Restore** for a sim's backup.
- **ffmpeg through the app.** An **Install ffmpeg…** banner replaces the Homebrew requirement.
- **Radio over USB.** USB Storage and the EdgeTX serial CLI (identify, list files, play a sound, reboot).
- **Firmware.** **Read firmware** saves a copy of a radio's firmware over DFU and changes nothing. A flash needs a verified copy first.
- **MCP and CLI schema version 1**, with a guard against silent removals or renames.

## 0.9.0 (2026-10-09)

- **Rate editor.** Edit a rate profile (type, rates, expo, limits, throttle curve, name) with live curves. **Convert** between Actual and Betaflight rates.
- **Sim sync.** Write a quad's rate profile into Liftoff, Micro Drones, The Zone or Uncrashed. QuadCam backs up the sim's file, refuses while the sim runs, reads back, and rolls back on failure.
- **Scoped Revert.** Revert undoes only that change's lines and warns when a later change touched them.
- **Radio model editors.** Checklists, telemetry screens, logging, timers, RF alarms and callouts, staged as card changes.
- **Voice.** A Voice segment with spelling rules, a render cache, loudness trim, pack building and **Choose voice**. Providers: macOS `say` and a local OpenAI-compatible speech endpoint.
- **Firmware.** Firmware check, Firmware page, splash image editor, and an EdgeTX flash plan over DFU with a backup first.

## 0.8.0 (2026-10-09)

- **OSD editor.** Drag elements on the grid or move them with the arrow keys, turn elements on or off per profile, copy a layout between profiles.
- **Rates segment.** Curves for Betaflight, Actual and Quick rates per profile, the throttle curve, and a compare view for profiles or two quads.
- **Radio card apply.** Staged card changes are backed up, written, read back and rolled back on any mismatch.
- **Mount button.** Mount a radio card to browse it. It unmounts at **Done** or after 10 minutes.
- **Bench page.** Every device's staged changes with Try, Keep and Revert. **Apply ready changes** on plug-in is optional and off by default.
- **Copy settings** between quads, with a compatibility check and a diff.

## 0.7.2 (2026-10-09)

- **Staged changes and FC apply.** Stage changes for a quad, review them in the apply sheet, then apply. QuadCam backs up first, saves, waits through the reboot, verifies every line against a fresh `dump all`, and backs up again. The first editor is **Edit setting**.
- **Sim preview** (off by default): a first-person view in a plain room.
- **Pack-up check** for devices that are not plugged in, from the latest backup.
- **Session report** can be saved from the app, the CLI and MCP.
- **Range trend** per place on Flights.
- **Switch map** shows combined switch positions and lists switches it cannot map.
- The Import sheet shows the card's latest check.

## 0.7.1 (2026-10-09)

- **Fixed:** a changed radio log stored as `<name> (2).csv` no longer shows as a second flight.
- **Fixed:** QuadCam no longer reads an FC port that another program has open. **Pause reads** on the Gear Overview stops FC reads until you resume.

## 0.7.0 (2026-10-07)

- **Gear.** A new sidebar section and status bar for the FPV bench. QuadCam finds EdgeTX radios, DJI goggles and air units, DVR cards, Betaflight flight controllers, ExpressLRS modules and radios in DFU mode. Save a device to name it and link it to an aircraft.
- **Backups** of radio cards and flight controllers on connect, with each file stored once, retention, browse and diff, and **Gear > Storage**.
- **Cues.** QuadCam speaks, plays a sound or posts a notification when a job ends. Mute, per-cue toggles and quiet hours are in Settings.
- **Betaflight link** over MSP and the CLI, with a USB timer that warns before a quad on USB heats up.
- **EdgeTX cards.** Reads the board, version, models and the radio clock.
- **Card check** with `diskutil verifyVolume` before a backup, and **Repair…**.
- **OSD** drawn per profile on the NTSC, PAL or HD grid, with overlap checks.
- **Switch map and Controls.** What each radio switch does, position by position, live in USB Joystick mode.
- **Flights and packs.** Flights read from radio logs (hover, sag, resting voltage, mAh, dropouts). Packs, a session report, **Pack up** and **Repairs**.
- **Modules.** **Settings > Modules** installs ffmpeg and esptool from their upstream, checked against pinned hashes. **QuadCam > Acknowledgements** lists third-party notices.
- **Card prep** formats a card with no import session. A removable DJI goggles card becomes exFAT.
- **Built-in SD slot.** A card in the Mac's SD slot counts as removable.
- **Fixed:** QuadCam detects a DVR's split length instead of assuming 600 s. Card format has no size refusal, and each source picks its file system. A radio-log armed segment is a "flight" everywhere.
- Every Gear feature has `quadcam-cli gear` commands and MCP tools.

## 0.6.4 (2026-10-07)

- Match analog clips to radio logs by their picture, for clips with dead air.

## 0.6.3 (2026-10-07)

- **Split by flight** makes one cut per radio-log flight.
- **Joined recordings.** Files an analog DVR split from one recording import as one clip. The **Join split recordings** setting controls it.

## 0.6.2 (2026-10-06)

- Shape-first radio-log matching for every source, including logs from a radio with a reset clock.
- Re-match radio logs to clips already in the library.
- **Delete clips after import** deletes clip files that verified from the card. It is off by default.

## 0.6.1 (2026-10-06)

- First release signed with a Developer ID and notarized.

## 0.6.0 (2026-10-06)

- **DJI O4 import.** Detect DJI units and goggles cards, date clips by their clock, and byte-copy the MP4 files. A kept `.SRT` sidecar moves with its original.
- **Clip clock skew** field under Log matching.
- No **Format card** for a DJI device.

## 0.5.3 (2026-10-02)

- The toolbar shows the app emblem.
- New README with screenshots; the reference moves to `docs/`.

## 0.5.2 (2026-10-02)

- Faster releases. No change to the app.

## 0.5.1 (2026-10-02)

- **Fixed:** the library scrolls again.

## 0.5.0 (2026-10-02)

- A new interface in the Mocha (dark) and Latte (light) themes.
- Clip ids come from a specified hash. Libraries from 0.4 and earlier move to the new ids on load, and the old ids still work.
- Library cuts read their metadata back before they count.

## 0.4.1 (2026-10-01)

- New app icon.

## 0.4.0 (2026-10-01)

- The app is named QuadCam.
- Place search with Apple, OpenStreetMap, US Census and Google Places providers.
- Edit a clip's date, time and aircraft in the library. Rename in place. Undo and redo for library edits, including Move to Trash.
- Choose the file-name date format: `YYYY-MM-DD` or `YY.MM.DD`.
- Native menu bar, Share menu, previous and next clip, sort control and drag-and-drop import.
- Faster scrolling in large libraries.
- The full MCP surface and matching CLI commands.

## 0.3.0 (2026-10-01)

- The library is the home screen, and import is a sheet over it.
- Library folder layout by year and flying day. The index rebuilds from file metadata.
- Ratings and flags.
- One trim editor for the import review and the library.

## 0.2.1 (2026-10-01)

- QuadCam restores the last session on relaunch. **Start over** clears it.
- The MCP server survives an app restart.

## 0.2.0 (2026-10-01)

- Moments from radio logs and dead air from the video, with cuts and trimmed exports.
- QuickTime metadata that Photos reads: location, local creation date, gear and keywords.
- Places and aircraft profiles, in the app, the CLI and the MCP server.

## 0.1.0 (2026-10-01)

- First release: import analog FPV DVR clips on macOS.
