# Moments and cuts

QuadCam marks moments on each clip's timeline. You can export any range of a clip as an extra file, called a cut.

The same trim editor appears in two places:

- In the library: open a clip.
- In the Import sheet: select a clip.

Play starts at once. On import, QuadCam makes a small preview of each clip in the background after the check. Library clips in MP4 play from the file itself.

## The timeline

| Row | Shows |
|---|---|
| Video | Frames from the clip |
| Signal | Flying (the picture is there) and **dead air** (striped) |
| Moments | Markers found in the radio log's stick channels, or highlights from video analysis, over a heat strip that is stronger where the moments are |
| Cuts | The ranges that export as extra files |

The same heat strip runs along the bottom of a clip's thumbnail in the library grid. A clip without moments has none.

**Keep ranges** are the clip without its dead air. Select **Use keep ranges** to turn them into cuts.

## Dead air

Dead air is any stretch of 3 seconds or more that shows one of these:

- the receiver's blue no-signal screen
- static
- a colour-bar test pattern
- black
- a colourless breakup: the torn grey picture an analog receiver shows when it loses the colour signal

A shorter breakup inside a flight stays in.

QuadCam does not look for dead air in DJI clips. Blue screen and static are analog signals.

A black-and-white camera reads as colourless breakup. For one, set `MONO_MAX_SAT` in `moments::tune` to 0.

Dead-air detection needs no radio log. QuadCam samples two frames per second at 64 × 48 pixels, and decodes only those frames. A 10-minute, 1.4 GB clip takes about 4 seconds. QuadCam times each sample by the frame's own timestamp, because DVRs drop frames.

The thresholds were checked on real Fat Shark Echo footage only. Other receivers may show no signal differently (another blue, snow, black), and may need other thresholds. Keep ranges start and end within about a second of the picture coming and going.

## Moments

QuadCam finds moments in the radio log's stick channels:

| Moment | What QuadCam looks for |
|---|---|
| Roll | Aileron at 80 % or more of full stick, held 0.3 to 1.5 s |
| Flip | Elevator at 80 % or more of full stick, held 0.3 to 1.5 s |
| Punch-out | Throttle from 35 % or less to 85 % or more within 0.6 s |
| Dive | Throttle at 15 % or less for 1 s or more mid-flight, then a punch-out |
| Crash? | Big stick inputs in the last 1.5 s before the log stops (disarm). Low confidence |

Each moment has a score from 0 to 1. If the receiver sends attitude telemetry and it shows the quad upside down during a roll or flip, the score goes up.

Every threshold is in `moments::tune` (`src-tauri/src/moments.rs`).

**Best moments** in the sidebar lists the moments of every clip, best score first: kind, clip, day, time in the clip, source and score. Select a row to open the clip and play from that moment. The sidebar's filters (a day, a place, an aircraft, a search) narrow it.

### Highlights from video analysis

On macOS 27 or later, QuadCam also asks macOS's video analysis (MediaIntelligence) which stretches of each clip are interesting. A clip with no radio-log moments, such as a DJI clip or an analog clip without a log, shows these stretches as **Highlight** moments. A clip with radio-log moments shows only those; its highlights stay in the file in case they are needed later.

- Each moment in the list says where it came from: **Radio log**, **Video analysis** or **Video signal** (dead air).
- A highlight scores its mean interest level over the clip's highest level, so the best stretch of each clip scores near 1.
- A highlight that is mostly dead air is dropped: analog static can look interesting to the analyzer.
- The analyzer also picks a **key frame**, the frame that best stands for the clip. It becomes the clip's picture in the Import sheet and in the library (outside dead air). Hover still scrubs the strip.

The analysis runs in the background, at about 5 times real time: a 10-minute clip takes about 2 minutes on an M1 Pro. It starts after a card loads, and an import never waits for it. Clips the import added before it finished are analysed after the import. QuadCam keeps each answer by the clip's content, so a clip is analysed once.

To analyse clips that were in the library before, or to run it again: right-click a clip and select **Find highlights again**, or use `quadcam-cli library analyze-video` (MCP: `quadcam_library_files` with `analyze_video`). Settings > Import > **Find highlights in new clips** turns the background analysis off.

On macOS 26 or older, or without QuadCam's `quadcam-intel` helper, nothing runs and nothing changes. The highlights are general-purpose: in tests on real FPV clips they did not line up with the rolls and flips a radio log found. Treat them as stretches worth a look.

### Better moments from the radio log

- **Log every 0.1 s.** EdgeTX logs every 0.5 s or 1 s by default. In the model's Special Functions, set the **SD Logs** function's interval to 0.1 s. At 0.5 s, QuadCam still finds moments, but their times are good only to half a second. Short moves can be missed, and the scores are lower.
- **Line up the log.** The log starts when you arm, but the DVR usually starts recording earlier. When an analog clip's picture placed the log (see [How a log matches](metadata.md#how-a-log-matches)), the log offset is already set, to within a few seconds. Otherwise QuadCam assumes that the clip starts at arm. Play the clip to the moment you arm, then select **Arm is here**. The moments move to match.

## Make a cut

1. Select a moment. QuadCam puts the in and out points around it.
2. Adjust the points. Use **Set in** and **Set out** (I and O), drag the handles, or type the time.
3. Select **Add cut** (C).

A clip can have up to 20 cuts.

### Split by flight

When a radio log matched the clip, **Split by flight (N)** adds one cut per flight, N being the number of flights. Each cut covers the flight from arm to disarm, plus up to 2 seconds before and after. Between two close flights, each cut gets at most half the gap, so the cuts do not overlap. Each cut stops at the start and the end of the clip, and a flight outside the clip gives no cut.

- The cuts are added to the cuts the clip has. A range the clip already has is not added twice.
- The original clip stays. On import, or with **Save N cuts** in the library, each cut becomes a file like any other cut.
- The cut files are numbered like other cuts, in time order: `_cut1`, `_cut2`. With no other cuts, the first flight is `_cut1`.
- There is nothing to split when the clip has no flights, or when its one flight leaves out less than 10 seconds or 10 % of the clip. QuadCam then says so and changes nothing.
- **Arm is here** moves the flights with the moments, so set it first.
- Library clips imported before QuadCam 0.6.3 do not have the flight times. Run **Match radio logs** in Settings > Library first.

The command line: `quadcam-cli cut 0 --by-flight` and `quadcam-cli library cut <id> --by-flight --export`. An agent: `split_by_flight` in `quadcam_suggest`, or the `split_by_flight` action of `quadcam_library_files`.

| Key | Does |
|---|---|
| Space | Play |
| J, K, L | Back, stop, forward |
| Left and right arrows | One frame |
| I, O | Set in, set out |
| C | Add cut |

## Cuts on import

On import, each cut becomes its own file next to the clip: `YYYY-MM-DD_<name>_cut1.mp4`, `_cut2`, and so on.

- QuadCam cuts from the original DVR file, not from the converted one. MJPEG has a keyframe on every frame, so each cut starts and ends on the exact frame.
- MP4 cuts are re-encoded like the full clip. MOV cuts copy the original frames.
- A DJI clip's MP4 cut is re-encoded too. Its MOV cut copies the H.264 or H.265 frames, so it starts at the keyframe before the cut. Cuts keep only the video and audio, not the DJI data streams.
- A cut carries its clip's metadata: name, note, date, place, aircraft, keywords and weather. Of the clip's moments it keeps only those inside its range, timed from the cut's own start. Keep ranges, flight stats and highlights describe the whole clip and stay with it.
- QuadCam verifies every cut: streams, frame count, duration, and the metadata it wrote.
- QuadCam does not write a verified cut again. You can add cuts later and import again.
- **Add to Photos** adds a clip's cuts with it.

## Cuts in the library

In the library, QuadCam writes new cuts when you select **Save N cuts** under the timeline. QuadCam cuts from the kept original when there is one. Otherwise it cuts from the clip itself. It verifies the cut the same way as on import.

## Remove a cut that is already a file

QuadCam asks first:

- **Keep the file** leaves the file where it is, as a clip of its own. It keeps its `_cutN` name, so a later cut that would take that name gets `_cutN-2`.
- **Move to Trash** moves the file to the Trash.

The command line needs `--removed keep` or `--removed trash`. An agent needs `removed_cuts`.
