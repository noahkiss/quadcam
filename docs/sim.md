# Sim

QuadCam is growing a built-in FPV simulator that flies your own quads on your own radio
(design: [sim-design.md](sim-design.md)). This page grows with each part. Built so far: the
radio's calibration for the sim, the flight model and its check against your logs, and the
Sim page, where you fly a plain room.

## Sim radio: calibrating the radio

Plug the radio in and choose **USB Joystick** on it, then open **Gear > Sim radio**.

**Which radio.** In joystick mode the radio's card is not mounted, so QuadCam matches the
joystick's USB name to a radio it has saved (read once in USB Storage mode):

| Saved radios of this model | What happens |
|---|---|
| One | That radio |
| Several | Pick it from **Which radio is this?** QuadCam remembers the answer for that model |
| None | A temporary key. Pick the radio under **Link to a saved radio** once its card has been read |

A calibration belongs to one radio. Two radios of the same model each keep their own, even
though they report the same USB serial number.

**What QuadCam suggests.** Pick the **Aircraft**. From its radio model and the quad's
Betaflight modes (the latest backups of the radio and FC linked to it), the page fills in:

| Control | Where it comes from |
|---|---|
| Sticks | The channel each stick drives in the model; else EdgeTX's default order, AETR |
| Arm, Angle, Horizon, Turtle, Air mode | The quad's `aux` ranges, named by the switch position that reaches them (`SA down`) |
| Reset | A button or trim that moves a channel but does nothing in the model or on the quad |

Each line says where it came from. **Change** sets a control by moving it on the radio;
**Clear** removes it. Channels 9 and up reach the joystick as buttons, on above 1500 µs.

**The steps.**

1. **Move** both sticks around their full travel. Each side of each stick must reach 80 %,
   and the step lasts at least 3 s so the ends are reached. **Done** accepts 50 %. For a
   model QuadCam knows, this is a short check: move each stick all the way both ways.
2. **Let go**: the centres come from 0.6 s of still sticks. Throttle needs none. If throttle
   and a stick were swapped, the one that stays where it was left is throttle.
3. **Flip the arm switch** (skipped when the quad's modes name it). **No arm switch** skips it.
4. **Press the reset control**: a free button or trim. **Keep** takes the suggestion.
5. **Review**: the sticks show the calibrated output live. Per axis: the channel (picking a
   channel another stick has swaps the two), **Reverse**, the two ends and the deadzone.

The ends are named by direction (Left end and Right end, Back end and Forward end, Bottom and
Top), and the names follow Reverse. An end you type is marked **(edited)** and wins over the
measured one until **Recalibrate**, which runs every step again and keeps the deadzones. The
deadzone is cut from the middle and the rest is rescaled, so full travel still reaches 100 %.

**Save** writes `<gear folder>/sim/calibrations.json`, keyed by the radio's Gear id.

## Sim: flying the plain room

The Sim is a preview and hidden at first: turn on **Settings > Gear > Preview > Show the Sim page**
(or `quadcam-cli settings set sim_preview=true`). Then open **Gear > Sim** and pick **Fly**. The sim starts at once on the profile chosen in the settings
(the 75 mm whoop by default) in a 5 × 4 × 2.5 m room with a crate, a low table and two gates.
The view is FPV, from the quad's camera. Calibrate the radio first (Sim radio): without a saved
calibration the sim still starts, but the arm, turtle and reset switches do nothing.

| Control | What it does |
|---|---|
| Arm switch | Arms with the throttle down. Turn it off and on again after a refusal; the OSD says why |
| Turtle switch | On while you arm, the quad flips over after a crash (throttle down) |
| Reset control | Puts the quad back on the start pad |
| **Reset** | The same, from the page |
| **Stop** | Ends the sim. Leaving the page stops it too |
| Settings button | Opens the settings popover |

The OSD shows flight mode and arm state, voltage, mAh used and the flight timer, and a warning
line (why the quad will not arm, a low pack, upside down, no radio). The stick display shows the
calibrated sticks in your stick mode, at the right edge above the OSD.

**Settings** save as you change them (the `simSettings` key of the settings file):

| Setting | What it does |
|---|---|
| Aircraft | The built-in profile to fly |
| View | FPV, or a chase camera behind the quad |
| Picture shape | 16:9 or 4:3; the picture sits letterboxed in the window |
| Camera tilt, Field of view | The camera's up-tilt and diagonal field of view (the profile's to start). A flat picture cannot show more than 100° vertically, so a wide whoop camera is cropped |
| Stick display, OSD | Show or hide each |

A click outside the popover, or Escape, closes it. Under the picture a line shows the frame
time and the radio-to-picture time. The sim is a preview: the room, the OSD and the camera are
the minimal version.

## The flight model

The sim flies a model of the quad, not a recording: a rigid body with four motors, props and
ducts, drag, ground effect, prop wash and vortex ring state, and a Betaflight-style flight
controller with the quad's own rates, throttle curve and modes. Physics runs at 2 kHz on its
own thread and keeps to the wall clock: when the Mac stalls, the sim skips ahead rather than
playing in slow motion. The battery holds 3.9 V per cell unless sag is turned on.

Built-in profiles:

| Profile | Quad |
|---|---|
| `meteor75` | 75 mm 1S whoop, 1102 21000 KV, 45 mm tri-blade props |
| `air65ii` | 65 mm 1S whoop, 0702 25000 KV, 31 mm tri-blade props |
| `five_inch` | 5-inch 6S freestyle |
| `seven_inch` | 7-inch 6S long range |

The two whoops are fitted to example blackbox logs; the 5-inch and 7-inch are estimates.

## Checking a profile against your logs

Decode the quad's blackbox logs to CSV with `blackbox_decode` (a separate program QuadCam
does not ship), put them in a folder, and run:

```bash
quadcam-cli gear sim validate meteor75 --logs ~/Desktop/decoded --text
```

QuadCam replays the logged sticks and motor commands through the sim and compares:

| Check | Passes when |
|---|---|
| Hover | Motor command within 0.02 and speed within 5 % of the logged level hover |
| Punch | Peak acceleration within 15 %, speed reaching 90 % within 15 ms, peak current within 10 % |
| Sag | Lowest pack voltage in a punch within 0.1 V |
| Roll, pitch | Lag behind the setpoint within 3 ms, overshoot within 10 points |
| Yaw | Lag within 5 ms |
| Coast-down | Deceleration within 15 % (needs a ground-speed column) |
| Fall recovery | Height lost within 20 % |

A check with no matching moment in the logs reads **not in log**. `--poles` sets the motor
pole count when it differs from the profile's.

## From the command line and MCP

```bash
quadcam-cli --json gear sim calibration                 # the joystick now: which radio, its calibration
quadcam-cli --json gear sim calibration radio-0123…     # a saved radio's
quadcam-cli gear sim calibration radio-0123… --set cal.json [--product "Radio Joystick"]
quadcam-cli --json gear sim defaults --aircraft Whoop   # or --radio CARD|MODEL.yml --fc DUMP
quadcam-cli --json gear sim validate air65ii --logs DIR  # a profile against decoded logs
```

MCP: `quadcam_gear` `sim_calibration`, `sim_defaults` and `sim_validate`; `quadcam_gear_edit`
`sim_calibration_save`. See [mcp.md](mcp.md).
