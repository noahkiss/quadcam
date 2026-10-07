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
| Moments | Markers found in the radio log's stick channels |
| Cuts | The ranges that export as extra files |

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

The thresholds were checked on real Fat Shark Echo footage. Keep ranges start and end within about a second of the picture coming and going.

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

### Better moments from the radio log

- **Log every 0.1 s.** EdgeTX logs every 0.5 s or 1 s by default. In the model's Special Functions, set the **SD Logs** function's interval to 0.1 s. At 0.5 s, QuadCam still finds moments, but their times are good only to half a second. Short moves can be missed, and the scores are lower.
- **Line up the log.** The log starts when you arm, but the DVR usually starts recording earlier. When an analog clip's picture placed the log (see [How a log matches](metadata.md#how-a-log-matches)), the log offset is already set, to within a few seconds. Otherwise QuadCam assumes that the clip starts at arm. Play the clip to the moment you arm, then select **Arm is here**. The moments move to match.

## Make a cut

1. Select a moment. QuadCam puts the in and out points around it.
2. Adjust the points. Use **Set in** and **Set out** (I and O), drag the handles, or type the time.
3. Select **Add cut** (C).

A clip can have up to 20 cuts.

### Split by flight

When a radio log matched the clip, **Split by flight (N)** adds one cut per pack, N being the number of packs. Each cut covers the pack from arm to disarm, plus up to 2 seconds before and after. Between two close packs, each cut gets at most half the gap, so the cuts do not overlap. Each cut stops at the start and the end of the clip, and a pack outside the clip gives no cut.

- The cuts are added to the cuts the clip has. A range the clip already has is not added twice.
- The original clip stays. On import, or with **Save N cuts** in the library, each cut becomes a file like any other cut.
- The cut files are numbered like other cuts, in time order: `_cut1`, `_cut2`. With no other cuts, the first pack is `_cut1`.
- There is nothing to split when the clip has no packs, or when its one pack leaves out less than 10 seconds or 10 % of the clip. QuadCam then says so and changes nothing.
- **Arm is here** moves the packs with the moments, so set it first.
- Library clips imported before QuadCam 0.6.3 do not have the pack times. Run **Match radio logs** in Settings > Library first.

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
- QuadCam verifies every cut: streams, frame count, duration, and the metadata it wrote.
- QuadCam does not write a verified cut again. You can add cuts later and import again.
- **Add to Photos** adds a clip's cuts with it.

## Cuts in the library

In the library, QuadCam writes new cuts when you select **Save N cuts** under the timeline. QuadCam cuts from the kept original when there is one. Otherwise it cuts from the clip itself. It verifies the cut the same way as on import.

## Remove a cut that is already a file

QuadCam asks first:

- **Keep the file** leaves the file where it is, as a clip of its own.
- **Move to Trash** moves the file to the Trash.

The command line needs `--removed keep` or `--removed trash`. An agent needs `removed_cuts`.
