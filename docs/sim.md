# Sim

QuadCam is growing a built-in FPV simulator that flies your own quads on your own radio
(design: [sim-design.md](sim-design.md)). This page grows with each part. Built so far: the
radio's calibration for the sim.

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

## From the command line and MCP

```bash
quadcam-cli --json gear sim calibration                 # the joystick now: which radio, its calibration
quadcam-cli --json gear sim calibration radio-0123…     # a saved radio's
quadcam-cli gear sim calibration radio-0123… --set cal.json [--product "Radio Joystick"]
quadcam-cli --json gear sim defaults --aircraft Whoop   # or --radio CARD|MODEL.yml --fc DUMP
```

MCP: `quadcam_gear` `sim_calibration` and `sim_defaults`; `quadcam_gear_edit`
`sim_calibration_save`. See [mcp.md](mcp.md).
