# Changelog

User-facing changes in each release, newest first. Release notes on
[GitHub](https://github.com/noahkiss/quadcam/releases) hold the test steps for the later ones.

## 0.23.0 (2026-10-24)

- **Set up a radio's switches in QuadCam.** A radio's new **Setup** segment lists each job (arm, rates and the others) with its channel, control, direction and what each position sends. Pick a control from a pop-up, or press **Learn** and move the switch with the radio in USB Joystick mode. Mark a swapped switch module or a blanking plug per slot. **Copy from** takes another radio's models and setup, with the controls mapped across. **Review** stages it as one change for the apply sheet; **Discard staged setup** asks first. The CLI has `gear setup` and `gear setup-edit`, MCP `radio_setup` and `radio_setup_edit`.
- **More radios.** QuadCam knows 50 EdgeTX boards: chip, storage, switches and pots, and the steps to put each in DFU mode. Detection shows the board. The RadioMaster GX12 is proven for flashing EdgeTX 2.12.5, its ELRS module, its splash and card writes; the other boards are read-only until tried. A model copies from one radio to another with its controls mapped. After a flash, QuadCam plans the storage pack, the ELRS Lua script and the volume name.
- **Sim profiles fitted to your quads.** QuadCam fits a sim profile (hover, motors, pack, rates) from an aircraft's blackbox pulls and its latest FC backup. It keeps a fitted value only when it flies no worse than the base profile on the same logs. The Sim page's **Aircraft** menu lists your fitted profiles first, under **Your quads**. QuadCam now reads `.bbl` files and flash images with its own blackbox decoder. CLI `gear sim fit` and `gear sim profiles`; MCP `sim_fit`, `sim_fit_save`, `sim_profiles`, `sim_profile_delete`.
- **Telemetry on a clip.** `quadcam-cli library overlay` writes a library clip's flight numbers (battery, link, throttle, mode, timer) from the radio log or the goggles' `.srt` as a subtitle file. With `--burn` it also writes a copy with the numbers drawn in. Both go to the `overlay_dir` folder, outside the library, and never replace a file without `overwrite`. MCP takes it too; a burn needs confirm.
- **Betaflight flash preview.** An FC on its maker's channel flashes the maker's build with the matching settings file. An FC set by hand to Official Betaflight flashes the official build, with a warning. A flash over a blank chip, or of firmware that names no board, needs a linked DFU device.
- **ExpressLRS preview.** The link to a module is steadier: it sends heartbeats while it waits, gives a just-powered module up to 8 seconds to answer, and drops late replies. After a module flash, QuadCam says the radio can look frozen for a moment.
- **Dialogs and keys.** Return in a dialog's field clicks its main button. A question that deletes or discards starts on **Cancel**. Discarding a staged change asks first. The clip grid takes one Tab stop, and a popover takes the focus when it opens. A dialog that closes on its own counts as Cancel.
- **Safer cards and files.** **Erase** formats the card its dialog showed, never another. QuadCam refuses to format a DJI device. The Mount timer and **Done** never unmount a card while a job writes to it. A card restore refuses another radio's backup. Radio card writes put new files first, `radio.yml` next and deletes last, and refuse a value with a line break or a stray quote.
- **A sturdier library.** Index saves take a lock and write through a temporary file, and a newer QuadCam's index is refused, not replaced. Move to Trash moves the clip last, after its cuts; undo puts back only what that run moved. A library edit of a clip's metadata survives a crash and keeps other apps' tags. A file Photos does not take is reported, not marked as added.
- **FC writes.** QuadCam refuses FC commands that program the ESCs, spin the motors, write the blackbox or hand the port away, in any letter case. An ExpressLRS job through an FC runs the USB heat check first. A write that saved but whose FC did not come back is recorded as saved and failed.
- **Voices.** The speech check hears sound-alikes (hertz as "hurts") and short words. The sample set is now the 84 risky lines. A grouped re-take tries one line per wrong word other ways, then makes the rest of the group the way that won. Only one paid render runs at a time on this Mac, and every paid render confirms the price it showed.
- **The command line and the app agree.** While the app runs, the CLI's session commands go to the app, so a CLI import is no longer overwritten. Agents cannot turn on clip deletion, blackbox erase, or replace an open import.
- **Smaller fixes.** A log that fails to read is tried again and a bad byte costs one row. Undo and redo run one at a time. A failed settings write puts the old value back, and out-of-range values are refused. The Gear page and the status bar redraw only when what they show changes. Module updates stay on their maker's signed release path.

## 0.22.0 (2026-10-17)

- **The window fills the screen.** Pages now use the window's full width up to 1760 px, then centre; the 960 px limit on Gear pages is gone. The window can be as narrow as 420 px: at 720 px and narrower the sidebar hides behind a button at the left of the toolbar, header buttons fold into a **…** menu, card grids drop to one column, tables fold into stacked rows, and a drawer or sheet fills the window.
- **One line instead of tiles.** Connected, Devices, Bench, Pack up, Packs, Repairs, Firmware and the Sim page's Rates count what matters in one line under the title. Flights keeps its day tiles; Storage shows one usage bar with a legend.
- **One action per row.** Rows keep the action they exist for and put the rest in a **…** menu: Devices (**Open**, **Back up now**, **Edit…**, **Forget…**), Connected cards, Bench changes (**Discard**, with who staged it and when), Packs (**Edit…**, **Retire**), Repairs (**Mark repaired** on the row; **Mark open**, **Delete…** in the menu), sims (**Show in Finder**, **Restore…**), Storage (**Export…**, **Show in Finder**) and places.
- **Details in a drawer.** A flight's details, a pack's history and the backup import report open in a drawer beside the table, not under it.
- **What needs you comes first.** Connected cards and Firmware rows that need attention sort first. Bench opens the devices that are plugged in and folds the others to one row. Pack up folds the checks that passed, Repairs folds repaired crashes, Packs folds retired packs.
- **Library days on one line.** A day's head shows its clips, armed time, flights, lowest battery, place, best moments and **Share** on one line; the separate summary card is gone.
- **Sim in one place.** **Sim radio** and **Sims** are now the **Radio** and **Rates** sections of **Gear > Sim**; **Fly** shows while the Sim preview is on. The stage fills the page at its picture shape. Radio sets Calibrate beside Switches, folds the axes, and picks **Normal** or **Reversed** per axis. Sims with one profile take one row.
- **Controls side by side.** Sticks and Channels share a row; one **Open…** pop-up opens a card, a model or a dump; Modes, Not mapped and Notes fold.
- **Places on the Map page.** The Map page lists your places beside the map, with **Add place…** and a menu per place. Settings > Places keeps the place list only while the Map page is off.
- **Settings.** The sheet is up to 1200 px wide and the Gear section uses two columns. **When a device is plugged in** is a pop-up per device kind with a check mark on each step. In a narrow window the sheet fills it and the sections are a pop-up.
- **Firmware.** **Read firmware…** opens a sheet; saved firmware copies fold; the ExpressLRS section folds and remembers it.
- **The maker's builds.** An FC on its maker's own Betaflight build (BetaFPV's Meteor75 Pro II and Air65 II) is checked against the maker's newest file, not the newest Betaflight release. The row shows the build with the maker and day, such as `2025.12.5-alpha (BetaFPV, Jun 25 2026)`, and official Betaflight as information. The FC's page has a **Firmware source** pop-up (Automatic, the maker, Official Betaflight); the CLI takes `gear devices save --firmware-channel`, MCP `device_save` `firmware_channel`. QuadCam now reads an FC's maker code, cloud build key and gyro.
- **Review the speech check in a sheet.** A pack's speech check now opens as a sheet from the row's **…** menu or its speech check badge, for every pack on this Mac. **Needs review** lists the lines heard wrong, the re-takes and the flags first; **All** lists every line. Each line plays its take, and a line heard wrong plays the take QuadCam held back. **Accept**, **Accept selected** and, for a studio pack, **Re-take selected** act on the lines. A re-take shows its price first and pays only after **Re-take and pay**. MCP `gear_voice_retake` re-takes those lines by name.
- **Restart to update comes to the front.** After **Restart to update**, the new QuadCam opens its window in front. In Settings, **Restart to update** keeps its fill and **Check for Updates** keeps its border and readable text.
- **Long paths fit.** QuadCam shows every file path the same way: `~` for your home folder, cut in the middle with the last folder kept, and the whole path on hover and on copy. A long library folder no longer pushes the first-run card out of its box.
- **A lighter Map page.** The map changes only the pins that changed, sends pins only when they change, and does no work while nothing moves. Callout lines now show in full. The scale shows only while you zoom.

## 0.21.6 (2026-10-16)

- **Gear keeps up with cards again.** Gear looks at the mounted volumes again at least every 30 seconds, and at once after QuadCam deletes clips from a card or formats one. A card that macOS could not describe for a moment no longer stays hidden until something else is plugged in, and a card emptied by an import no longer lingers in Gear.

## 0.21.5 (2026-10-16)

- **No more "is eighty".** In a long ElevenLabs render, some lines kept the end of the sentence they were spoken in ("is eighty", "is 75") or lost their last syllable ("eighty" cut to "eight"). ElevenLabs' timestamps can run ahead of the audio for part of a batch; QuadCam now finds each sentence by the pauses in the audio and cuts those lines there. Render a pack again with the same settings to get the new cut: QuadCam cuts the cached takes again and sends nothing to ElevenLabs.

## 0.21.4 (2026-10-14)

- **Lower energy use while idle.** Gear no longer asks macOS for full details about every mounted volume every two seconds. It refreshes those details only when a volume changes, while USB serial and DFU devices remain quick to appear.

## 0.21.3 (2026-10-13)

- **A tidier Voices page.** ElevenLabs is now one section that holds the key, the voices, the models, the line sets, the cost, Sample, Render and the Samples. Click its title to collapse it; it stays collapsed until you open it again. A new **Mac and other voices** section holds **Render my voice** and names the provider in the settings. The summary and **Voice packs on this Mac** stay at the top.
- **Re-takes behind a click.** A pack row no longer lists every line that needs a re-take. It shows a count, such as "150 need a re-take, 2 flagged"; click it to list the lines with their reasons.
- **Full-width voice list.** The ElevenLabs voice list takes the full width and scrolls in its own box. The models sit below it.
- **Real names on the sample cards.** Sample cards from 0.19 and earlier show the voice's name ("Adam - Dominant, Firm"), not its folder name, and every card names the model ("Eleven v4 Turbo"). Each card's lines sit in an aligned grid. **Lines rendered** in the summary lists the top three voices by their short names.
- **Line sets under Full EdgeTX English.** In the ElevenLabs section, the aircraft sets and the telemetry scripts sit under **Full EdgeTX English**, the aircraft you fly first. Its box ticks or clears them all and shows a dash while only some are ticked. Its line count reads "412 of 795" and counts a line two sets share once. **FPV extras**, **Easter eggs** and **QuadCam default** stay sets of their own.
- **Pack rows fit the window.** A pack row no longer scrolls sideways. The voice's short name is bold, with the model, where the pack came from, its sets in one line ("Full EdgeTX English + 2"; point at it for the full list) and its radios under it. The date is short ("Oct 13"). **Apply to radios…**, **Check speech**, **Publish…** and **Delete…** moved into the row's **…** menu.
- **One model at a time.** The ElevenLabs model is now a pop-up menu, each model with its price per 1K characters. Beside it, the picked model shows its credits a character and the lines the voice has rendered in it. **Sample** and **Render pack** use that one model. The CLI and MCP still take several models.
- **Samples as a drawer.** Samples opens partly closed, showing the top of the first row of cards. **Show all N cards** opens it and **Show less** closes it; QuadCam remembers the choice on this Mac.
- **Line sets of a pack.** A pack row now lists every set the pack was rendered from. Before, a set with one line waiting for a re-take dropped out of the list.
- **Credits estimated from the dollar price.** ElevenLabs bills a pay-as-you-go account about $0.000364 a credit ($20 for 54,945 credits). The estimate now divides each model's USD price by that, so a $0.08 model costs about 0.22 credits a character and a $0.04 model about 0.11, not 1 and 0.5. A promo price lowers the credits too while it lasts. A subscription plan may price credits differently, so the figure stays marked as estimated. Before, the check refused renders the account could afford.
- **Short pack ids.** **Publish…** and `gear voice publish` without an id now name a pack after the voice's short name: "Xavier - Dominating, Metalic Announcer" gives `en-xavier`, not `en-xavier---dominating--metalic-announcer`.

## 0.21.2 (2026-10-13)

- **S and F keep their hiss.** A rendered line that starts with an s, z, f, h, th, sh or ch sound ("Signal critical", "Failsafe") no longer loses that first sound, which made an S sound like an F. The cut out of the carrier sentence now keeps up to 120 ms of the hiss before the vowel. Render a pack or the samples again with the same settings to get the new cut: QuadCam cuts the cached takes again and sends nothing to ElevenLabs.

## 0.21.1 (2026-10-12)

- **Restart to update in the Settings sidebar.** Once a check finds a newer version, **Check for Updates** under the version becomes **Restart to update**, on every Settings page. Settings > Advanced keeps the automatic check and the release notes.

## 0.21.0 (2026-10-12)

- **The DJI air unit unmounts like a card.** An O4 air unit (or goggles in storage mode) over USB now follows the same mount, work, unmount cycle as every card: QuadCam unmounts it when an import or job ends and says "safe to unplug", it stays on USB, and the next job or **Mount** (10 minutes) mounts it again. **Safe to remove** unmounts it instead of ejecting it. QuadCam still never formats a DJI device over USB.
- **Status bar.** The bar at the bottom of the window now shows each kind's name next to its state, gives an ExpressLRS device its own item, and VoiceOver announces each change of state (safe to unplug, still inserted, needs attention, removed).
- **Cues in the radio's voice.** Cues now speak with a voice pack when one is installed: the pack chosen for the radio plugged in last, or one picked in Settings > Gear > **Voice pack**. Each cue is one voice: your device's name and the cue ("Pocket", "Done, safe to unplug.") when your own pack has the name, else one sentence for the kind ("Radio done, safe to unplug.", "A step failed on the air unit."), else the macOS voice for the whole cue. Naming a device adds the name to your own lines for your next render. **Voice** > **macOS** keeps the Mac voice. The new lines join the QuadCam default line set, so packs rendered from now on hold them; older packs show as needing a new render. The setting `gear_cues` takes `voice_pack`.
- **Restore a saved firmware copy.** **Gear > Firmware** lists each radio's saved firmware copies (from **Read firmware**, or kept before a flash) with the date, the EdgeTX version, the size and the SHA-256. **Restore…** writes a copy back to the radio over DFU after the same checks a flash runs: the copy's hash, its board against the radio's, the image checks, a card backup and the one radio in DFU mode. QuadCam keeps the firmware the radio runs now as a copy first, unless a saved copy already holds it, then reads the write back and compares it. Not yet tried on a real radio. CLI `gear firmware --copies` and `--restore`; MCP `firmware_copies`, `firmware_restore_plan` and `firmware_restore`.
- **Radio settings.** A radio's new **Settings** segment edits the radio's own settings (`radio.yml`): sound and volumes, haptic, display and backlight, battery and inactivity alarms, RSSI shutdown alarm, time zone and GPS format, stick mode, default channel order, USB mode, owner ID and the selected model. Each edit stages into one "Radio settings" change, which the apply sheet shows as a diff and writes like any card change. Values are checked against the radio's ranges before they stage, and a staged setting shows the value still on the radio. CLI `gear radio-settings` and `gear radio-settings-edit`; MCP `radio_settings` and `radio_settings_edit`.
- **Card warnings before you apply.** The apply sheet now shows a card change's warnings (a haptic mode that drops special-function haptics, a new stick mode) before you apply, not only in the result.
- **Map pin callouts.** A click on a pin on the Map page now shows a callout instead of leaving the page. A place's callout shows its clip count and the first and last day of its clips, with **Show clips**. A clip's callout shows its name, day and time, its poster frame and the sky at flight time, with **Open**. Return does what the button does; Escape closes the callout. VoiceOver reads each pin and its callout. See [The map](docs/library.md#the-map).
- **The map remembers its look.** **Map**, **Hybrid** and **Satellite** now save the new `map_style` setting (`standard`, `hybrid`, `satellite`; `mapStyle` in the settings file), and the map opens in it. The CLI and MCP read and write it too. See [Map style](docs/settings.md#map-style).

## 0.20.2 (2026-10-12)

- **Samples play again.** Sample lines, **Play all** and the packs on this Mac play their sound again; in 0.20.0 and 0.20.1 they played nothing. A line that cannot play now says so in a message, once per **Play all**, instead of being skipped in silence.
- **Voice lists scroll in their own box.** The Voice studio's voice list and the **Browse voices…** results now scroll inside a box of their own, under a header that stays. The cost, the Samples and the packs stay close.
- **The studio remembers its line sets.** The first time, the Voice studio ticks every line set. After that it keeps the sets you ticked, in the gear folder, and ticks them again next time. **QuadCam default** is ticked each time the studio opens: QuadCam's own cues need it. Without it, a note by **Render pack** says that these cues use the Mac voice.
- **Every setting in a settings read.** `quadcam-cli settings` (in `named`) and the MCP `quadcam_settings` read now show every setting with its value in effect, including the Map page, the Sim and flash previews, Gear, voice and ExpressLRS settings. A write of one of them shows it in the answer too.
- **The sky at flight time.** With **Look up weather** on, an open clip also shows the sky with an icon (clear, partly cloudy, cloudy, fog, drizzle, rain, heavy rain, sleet, snow, thunderstorm or hail), cloud cover, precipitation, humidity and, where Open-Meteo has it, visibility. A clip looked up before is asked once more. MCP and CLI library output carry the new values.

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
