#!/usr/bin/env python3
"""Fill a throwaway HOME with a made-up library, places, aircraft, flights, packs and repairs,
so a dev build opens on full pages instead of first-run screens.

    scripts/dev-seed.py HOME [--cli PATH] [--own-gear] [--force]

Every write goes through quadcam-cli, the app's own write paths: places, profiles and
settings; clips staged from generated "cards", dated against generated EdgeTX logs and
imported; stars and flags; packs, flights and repairs in the gear folder. The generated
sources (clips and radio logs) live in HOME/.dev-seed/. Nothing is written outside HOME.

- Refuses the account's real home folder, any folder that holds it, and a HOME whose
  library folder (the `outputDir` setting) is outside it.
- Idempotent: places, profiles, settings, packs and flights are upserts; a flying day whose
  clips are already in the library is skipped; a repair already logged is not logged again.
  --force deletes the library and HOME/.dev-seed first and seeds again.
- Gear (packs, flights, repairs) lives in the gear folder. When the `gearDir` setting points
  outside HOME, the seeder leaves it and skips gear data; --own-gear points `gearDir` at
  HOME's own gear folder first.

Needs ffmpeg and a quadcam-cli build (default: src-tauri/target/debug/quadcam-cli).
"""

import argparse
import json
import os
import pwd
import random
import shutil
import subprocess
import sys
from datetime import date, datetime, timedelta
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent

# Made-up spots around San Francisco; none is anybody's address.
PLACES = [
    ("Home", 37.7596, -122.4369),
    ("Farm", 38.2934, -122.4580),
    ("Ocean Beach", 37.7604, -122.5107),
    ("Crissy Field", 37.8040, -122.4655),
]

# name, aircraft, camera make, camera model, video system, EdgeTX model, keywords
PROFILES = [
    ("Air65", "BetaFPV Air65 II", "Analog", "DVR", "Analog", "Air65", "whoop,indoor"),
    ("Meteor75 O4", "BetaFPV Meteor75 Pro II O4", "DJI", "O4 Air Unit", "DJI O4", "Meteor75", "whoop,digital"),
    ("TinyHawk", "Emax TinyHawk III", "Analog", "DVR", "Analog", "TinyHawk", "whoop,outdoor"),
]
DEFAULT_PROFILE = "Air65"

# name, chemistry, cells, capacity, connector, warn mAh
PACK_TYPES = [
    ("LAVA II 320", "lihv", 1, 320, "BT2.0", 250),
    ("LAVA II 550", "lihv", 1, 550, "BT2.0", 430),
    ("LAVA II 680", "lihv", 1, 680, "BT2.0", 530),
]
# label, type, received, retired, the profile whose flights it flies
PACKS = [
    ("A1", "LAVA II 320", "2026-07-02", False, "Air65"),
    ("A2", "LAVA II 320", "2026-07-02", False, "Air65"),
    ("A3", "LAVA II 320", "2026-07-02", False, "Air65"),
    ("A4", "LAVA II 320", "2026-05-14", True, "Air65"),
    ("T1", "LAVA II 550", "2026-08-10", False, "TinyHawk"),
    ("T2", "LAVA II 550", "2026-08-10", False, "TinyHawk"),
    ("T3", "LAVA II 550", "2026-08-10", False, "TinyHawk"),
    ("M1", "LAVA II 680", "2026-08-20", False, "Meteor75 O4"),
    ("M2", "LAVA II 680", "2026-08-20", False, "Meteor75 O4"),
    ("M3", "LAVA II 680", "2026-08-20", False, "Meteor75 O4"),
]

# The flying days: date, place, first takeoff, (profile, clip name, note) per clip. A place
# of None gives the day's clips a location of their own and no saved place.
DAYS = [
    ("2026-08-22", "Home", "18:05", [
        ("Air65", "backyard-loops", "First pack after the new props."),
        ("Air65", "fence-gap", ""),
        ("Air65", "patio-orbit", ""),
    ]),
    ("2026-08-29", "Farm", "09:40", [
        ("TinyHawk", "barn-dive", "Dove the barn roof twice."),
        ("TinyHawk", "fence-line", ""),
        ("TinyHawk", "hay-bales", ""),
        ("Air65", "tractor-gap", "Wind picked up after this one."),
    ]),
    ("2026-09-03", "Home", "19:10", [
        ("Air65", "dusk-lines", ""),
        ("Air65", "tree-gap", "Clipped a leaf, kept going."),
        ("Air65", "garden-hover", ""),
    ]),
    ("2026-09-07", "Ocean Beach", "08:20", [
        ("Meteor75 O4", "dune-cruise", "Smooth air, low tide."),
        ("Meteor75 O4", "surf-chase", ""),
        ("Meteor75 O4", "seawall-run", ""),
        ("Meteor75 O4", "fog-bank", ""),
    ]),
    ("2026-09-12", "Farm", "10:15", [
        ("TinyHawk", "orchard-weave", ""),
        ("TinyHawk", "silo-climb", "Punched to the top of the silo."),
        ("Meteor75 O4", "field-sweep", ""),
    ]),
    ("2026-09-14", "Home", "17:45", [
        ("Air65", "sunset-loops", ""),
        ("Air65", "hedge-skim", ""),
        ("Air65", "porch-gap", "Third try through the railing."),
    ]),
    ("2026-09-20", "Crissy Field", "08:50", [
        ("Meteor75 O4", "bridge-view", "Golden hour, bridge in the fog."),
        ("Meteor75 O4", "marsh-skim", ""),
        ("Meteor75 O4", "beach-run", ""),
    ]),
    ("2026-09-24", None, "12:30", [
        ("Meteor75 O4", "lunch-rip", "Park near work."),
        ("Meteor75 O4", "bench-gap", ""),
    ]),
    ("2026-09-27", "Farm", "09:05", [
        ("TinyHawk", "pond-skim", ""),
        ("TinyHawk", "barn-loop", ""),
        ("TinyHawk", "windmill", "Prop strike on the windmill."),
        ("Air65", "coop-gap", ""),
    ]),
    ("2026-10-03", "Home", "18:20", [
        ("Air65", "garage-dive", ""),
        ("Air65", "maple-orbit", ""),
        ("Air65", "night-lights", "String lights on the fence."),
    ]),
    ("2026-10-08", "Ocean Beach", "16:40", [
        ("Meteor75 O4", "pier-line", ""),
        ("Meteor75 O4", "kite-chase", "Followed a kite surfer."),
        ("Meteor75 O4", "wave-skim", ""),
        ("Meteor75 O4", "cliff-drop", ""),
    ]),
    ("2026-10-11", "Farm", "11:00", [
        ("TinyHawk", "harvest-run", ""),
        ("TinyHawk", "trailer-gap", ""),
        ("Air65", "fence-weave", ""),
        ("Meteor75 O4", "field-dive", "Last pack of the day."),
    ]),
]
# The days with no saved place put their clips here instead (a blue pin each).
LOOSE_SPOT = (37.7694, -122.4862)

# Stars and flags by clip name. Every other clip keeps none.
STARS = {
    "backyard-loops": 3, "barn-dive": 5, "tree-gap": 4, "dune-cruise": 4, "surf-chase": 3,
    "silo-climb": 5, "porch-gap": 4, "bridge-view": 5, "lunch-rip": 2, "windmill": 2,
    "night-lights": 4, "kite-chase": 5, "cliff-drop": 4, "field-dive": 3, "hedge-skim": 1,
}
PICKS = {"barn-dive", "silo-climb", "bridge-view", "kite-chase"}
REJECTS = {"garden-hover", "fog-bank", "bench-gap"}

# Clips that start with 3 s of blue no-signal screen (dead air to trim).
BLUE_START = {"tree-gap", "windmill", "garage-dive"}

# Packs marked charged (the rest read as flown or in storage).
CHARGED = {"A1", "A2", "M1"}

# clip name, how far into the clip (0 to 1), aircraft, what broke, parts, note, repaired
REPAIRS = [
    ("windmill", 0.7, "TinyHawk", "Two props, bent motor bell", "Props 40 mm,Motor 0802", "Windmill blade.", True),
    ("tree-gap", 0.8, "Air65", "Cracked canopy", "Canopy", "Canopy taped for now.", False),
]

SEED = 20260822


def die(msg):
    sys.exit(f"dev-seed: {msg}")


def real_home():
    return Path(pwd.getpwuid(os.getuid()).pw_dir).resolve()


def check_home(home):
    real = real_home()
    if home == real or real.is_relative_to(home):
        die(f"refusing {home}: that is (or holds) the real home folder")
    if not home.is_dir():
        die(f"{home} is not a folder")


class Cli:
    def __init__(self, exe, home):
        self.exe = str(exe)
        self.env = {
            "HOME": str(home),
            "PATH": "/opt/homebrew/bin:/usr/local/bin:/usr/bin:/bin",
            "QUADCAM_PHOTOS": "dry-run",
            "TMPDIR": str(home / ".dev-seed" / "tmp"),
            "LANG": "en_US.UTF-8",
        }
        Path(self.env["TMPDIR"]).mkdir(parents=True, exist_ok=True)

    def __call__(self, *args, ok_fail=False):
        cmd = [self.exe, "--json", *map(str, args)]
        p = subprocess.run(cmd, env=self.env, capture_output=True, text=True)
        try:
            out = json.loads(p.stdout)
        except json.JSONDecodeError:
            die(f"{' '.join(cmd[1:])}: no JSON (exit {p.returncode})\n{p.stdout}\n{p.stderr}")
        if not out.get("ok"):
            if ok_fail:
                return None
            die(f"{' '.join(cmd[1:])}: {out.get('error')}")
        return out["result"]


# ---- generated sources -------------------------------------------------------------------

def ffmpeg(*args):
    subprocess.run(["ffmpeg", "-v", "error", "-y", *args], check=True)


FONT = next((f for f in ["/System/Library/Fonts/Supplemental/Arial.ttf",
                         "/System/Library/Fonts/Helvetica.ttc",
                         "/Library/Fonts/Arial.ttf"] if Path(f).exists()), None)


def picture(name, size, secs, seed, blue):
    """A filter graph: a moving gradient with the clip's name, after `blue` s of blue."""
    w, h = size
    rng = random.Random(seed)
    cols = ["0x%06x" % rng.randrange(0x202020, 0xe0e0e0) for _ in range(4)]
    g = (f"gradients=s={w}x{h}:r=30:c0={cols[0]}:c1={cols[1]}:c2={cols[2]}:c3={cols[3]}"
         f":nb_colors=4:speed=0.03:seed={seed}:d={secs - blue}")
    if FONT:
        g += (f",drawtext=fontfile='{FONT}':text='{name}':fontsize={h // 9}:fontcolor=white"
              f":borderw=3:x=(w-tw)/2:y=h*0.42+h*0.1*sin(t)")
    g += ",format=yuv420p"
    if blue:
        return f"color=c=0x1030c8:s={w}x{h}:r=30:d={blue},format=yuv420p[b];{g}[g];[b][g]concat=n=2:v=1[v]"
    return f"{g}[v]"


def make_analog(path, name, secs, seed, blue, mtime):
    if not path.exists():
        path.parent.mkdir(parents=True, exist_ok=True)
        tmp = path.with_name(path.name + ".tmp")
        graph = picture(name, (640, 480), secs, seed, blue)
        graph += f";sine=frequency=330:sample_rate=32000:d={secs}[a]"
        ffmpeg("-filter_complex", graph, "-map", "[v]", "-map", "[a]", "-t", str(secs), "-c:v", "mjpeg", "-q:v", "8", "-pix_fmt", "yuvj420p",
               "-c:a", "pcm_s16le", "-ac", "1", "-f", "avi", str(tmp))
        tmp.rename(path)
    os.utime(path, (mtime, mtime))


def make_dji(path, name, secs, seed, start):
    if not path.exists():
        path.parent.mkdir(parents=True, exist_ok=True)
        tmp = path.with_name(path.name + ".tmp")
        utc = datetime.utcfromtimestamp(start.timestamp()).strftime("%Y-%m-%dT%H:%M:%SZ")
        ffmpeg("-filter_complex", picture(name, (960, 540), secs, seed, 0), "-map", "[v]",
               "-t", str(secs), "-c:v", "libx264", "-preset", "ultrafast", "-crf", "30",
               "-metadata", "comment=EIS:RS;FOV:Wide;", "-metadata", f"creation_time={utc}",
               "-f", "mp4", str(tmp))
        tmp.rename(path)
        srt = path.with_suffix(".SRT")
        srt.write_text("".join(
            f"{i + 1}\n00:00:{i:02},000 --> 00:00:{i + 1:02},000\n"
            f"[iso: 100] [shutter: 1/240.0] [fnum: 2.8] [ev: 0]\n\n" for i in range(min(secs, 59))))
    os.utime(path, (start.timestamp() + 1, start.timestamp() + 1))


HEADER = ("Date,Time,1RSS(dB),RQly(%),RSNR(dB),TPWR(mW),RxBt(V),Curr(A),Capa(mAh),Bat%(%),"
          "FM,Rud,Ele,Thr,Ail,TxBat(V)\n")


def log_rows(day, start, armed_s, capacity, rng):
    """Rows every 0.1 s for one flight: 2 s disarmed, armed_s s armed, 2 s disarmed. Rolls,
    flips and punches at random times, so the import finds moments."""
    out = []
    events = []
    t = 4.0
    while t < armed_s - 5:
        events.append((t, rng.choice(["roll", "flip", "punch", "hang"])))
        t += rng.uniform(5, 11)
    n_armed = int(armed_s * 10)
    weak = rng.random() < 0.3
    rest = rng.uniform(3.68, 3.86)
    mah_per_s = capacity * rng.uniform(0.62, 0.78) / armed_s
    for k in range(-20, n_armed + 20):
        ts = start + timedelta(milliseconds=100 * k)
        armed = 0 <= k < n_armed
        s = k / 10
        thr, ail, ele, rud = (0.0, 0.0, 0.0, 0.0)
        if armed:
            thr = 0.45 + 0.08 * ((k // 7) % 3)
            ail = 0.15 * ((k // 13) % 3 - 1)
            ele = 0.1 * ((k // 17) % 3 - 1)
            for at, kind in events:
                if at <= s < at + 1.0:
                    if kind == "roll":
                        ail = 0.95
                    elif kind == "flip":
                        ele = 0.95
                    elif kind == "punch":
                        thr = 0.97
                elif kind == "punch" and at - 1.0 <= s < at:
                    thr = 0.2
                elif kind == "hang" and at <= s < at + 1.6:
                    thr = 0.05
                elif kind == "hang" and at + 1.6 <= s < at + 2.4:
                    thr = 0.95
        frac = max(0.0, min(1.0, s / armed_s)) if k >= 0 else 0.0
        capa = int(mah_per_s * min(s, armed_s)) if k >= 0 else 0
        rxbt = 4.30 - 0.55 * frac - (0.25 * thr if armed else 0) if k < n_armed else rest
        curr = (2.0 + 9.0 * thr) if armed else 0.1
        lq = rng.randint(68, 90) if weak and rng.random() < 0.03 else 100
        rssi = -52 - int((30 if weak else 15) * frac) - rng.randint(0, 6)
        fm = "ACRO" if armed else "ACRO*"
        to_ch = lambda v: int(v * 1024)  # noqa: E731
        out.append(
            f"{day},{ts:%H:%M:%S}.{ts.microsecond // 1000:03},{rssi},{lq},9,25,{rxbt:.2f},"
            f"{curr:.1f},{capa},{max(0, 100 - capa * 100 // capacity)},{fm},{to_ch(rud)},"
            f"{to_ch(ele)},{int(thr * 2048 - 1024)},{to_ch(ail)},7.9\n")
    return out


def sources(work):
    """Writes the cards and radio logs. Returns the groups to import, one per day and
    video system: (day, kind, folder, [(clip file, plan)])."""
    rng = random.Random(SEED)
    groups = []
    log_dir = work / "radio-logs"
    capacity = {t[0]: t[3] for t in PACK_TYPES}
    pack_type = {p[4]: p[1] for p in PACKS}
    model = {p[0]: p[5] for p in PROFILES}
    pict = 1
    for day, place, first, clips in DAYS:
        d = date.fromisoformat(day)
        clock = datetime.combine(d, datetime.strptime(first, "%H:%M").time())
        logs = {}
        by_kind = {}
        dji = 1
        for prof, name, note in clips:
            secs = rng.randint(28, 52)
            seed = rng.randrange(1, 1 << 30)
            kind = "DJI" if prof.startswith("Meteor") else "Analog"
            folder = work / "cards" / f"{day}-{kind.lower()}"
            if kind == "DJI":
                f = folder / "DCIM" / "DJI_001" / f"DJI_{clock:%Y%m%d%H%M%S}_{dji:04}_D.MP4"
                dji += 1
                make_dji(f, name, secs, seed, clock)
            else:
                f = folder / "DCIM" / f"PICT{pict:04}.AVI"
                pict += 1
                blue = 3 if name in BLUE_START else 0
                make_analog(f, name, secs, seed, blue, (clock + timedelta(seconds=secs)).timestamp())
            lead = 3.5 if name in BLUE_START else 1.5
            armed_start = clock + timedelta(seconds=lead)
            rows = log_rows(day, armed_start, secs - lead - 1.5, capacity[pack_type[prof]], rng)
            logs.setdefault(model[prof], []).append((armed_start, rows))
            plan = {"name": name, "note": note, "profile": prof, "date": day,
                    "time": f"{clock:%H:%M}"}
            if place:
                plan["place"] = place
            else:
                lat, lon = LOOSE_SPOT
                plan["location"] = {"lat": round(lat + rng.uniform(-0.004, 0.004), 5),
                                    "lon": round(lon + rng.uniform(-0.006, 0.006), 5)}
            by_kind.setdefault(kind, (folder, []))[1].append((f, plan))
            clock += timedelta(seconds=secs + rng.randint(150, 420))
        for m, flights in logs.items():
            start = flights[0][0] - timedelta(seconds=2)
            path = log_dir / f"{m}-{start:%Y-%m-%d-%H%M%S}.csv"
            if not path.exists():
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_text(HEADER + "".join(r for _, rows in flights for r in rows))
        for kind, (folder, items) in by_kind.items():
            groups.append((day, kind, folder, items))
    return groups, log_dir


# ---- seeding through the CLI -------------------------------------------------------------

def seed_setup(cli, home, own_gear):
    for name, lat, lon in PLACES:
        cli("places", "save", name, "--location", f"{lat},{lon}")
    for name, aircraft, make, model, system, edgetx, kw in PROFILES:
        args = ["profiles", "save", name, "--aircraft", aircraft, "--camera-make", make,
                "--camera-model", model, "--video-system", system, "--models", edgetx,
                "--keywords", kw, "--author", "Pilot"]
        if name == DEFAULT_PROFILE:
            args.append("--default")
        cli(*args)
    cli("settings", "set", "map_view=true", "sim_preview=true", "elrs_preview=true",
        "betaflight_flash_preview=true")
    gear = home / "Library/Application Support/app.quadcam/gear"
    if own_gear:
        cli("settings", "set", f"gear_dir={gear}")
    current = cli("settings")
    return current


def gear_dir_of(settings, home):
    for key in ("gear_dir", "gearDir"):
        v = find_key(settings, key)
        if isinstance(v, str) and v:
            return Path(v).expanduser()
    return home / "Library/Application Support/app.quadcam/gear"


def find_key(v, key):
    if isinstance(v, dict):
        if key in v and not isinstance(v[key], (dict, list)):
            return v[key]
        for x in v.values():
            r = find_key(x, key)
            if r is not None:
                return r
    elif isinstance(v, list):
        for x in v:
            r = find_key(x, key)
            if r is not None:
                return r
    return None


def library(cli):
    return cli("library", "list")["clips"]


def seed_clips(cli, groups, log_dir):
    have = {(c["date"], c["title"]) for c in library(cli)}
    for day, kind, folder, items in groups:
        todo = [p for _, p in items if (day, p["name"]) not in have]
        if not todo:
            print(f"  {day} {kind}: in the library")
            continue
        if len(todo) != len(items):
            die(f"{day} {kind}: only some clips are in the library; run again with --force")
        cli("clear")
        cli("stage", folder)
        cli("analyze")
        cli("dates", "--logs", log_dir, "--day", day)
        session = cli("show")
        staged = sorted(session["clips"], key=lambda c: str(c.get("source") or c.get("path") or c.get("name")))
        files = sorted(items, key=lambda x: str(x[0]))
        if len(staged) != len(files):
            die(f"{day} {kind}: staged {len(staged)} clips, expected {len(files)}")
        plan = {"clips": []}
        for c, (_, p) in zip(staged, files):
            plan["clips"].append({"id": c["id"], "skip": False, **p})
        pf = Path(cli.env["TMPDIR"]) / "plan.json"
        pf.write_text(json.dumps(plan))
        cli("import", "--plan", pf)
        cli("clear")
        print(f"  {day} {kind}: imported {len(items)} clips")


def seed_ratings(cli):
    clips = library(cli)
    by = {c["title"]: c for c in clips}
    for name, n in STARS.items():
        if name in by and by[name].get("rating") != n:
            cli("library", "rate", by[name]["id"], "--stars", n)
    for name in PICKS | REJECTS:
        want = "pick" if name in PICKS else "reject"
        if name in by and by[name].get("flag") != want:
            cli("library", "rate", by[name]["id"], f"--{want}")
    return by


def seed_gear(cli, log_dir, clips):
    for name, chem, cells, capacity, conn, warn in PACK_TYPES:
        cli("gear", "packs", "type", "save", name, "--chemistry", chem, "--cells", cells,
            "--capacity", capacity, "--connector", conn, "--full", "4.35", "--storage", "3.85",
            "--charge-a", round(capacity / 1000, 2), "--warn-mah", warn)
    for label, ptype, received, retired, _ in PACKS:
        cli("gear", "packs", "save", label, "--type", ptype, "--received", received,
            "--retired", str(retired).lower(), *(["--note", "Puffed; kept for the bench."] if retired else []))
    cli("gear", "packs", "notes", "Storage-charge anything not flown within two days.")
    if str(log_dir) not in [str(p) for p in cli("gear", "flights", "folders")]:
        cli("gear", "flights", "folders", "--add", log_dir)
    view = cli("gear", "flights")
    flights = view.get("flights", view) if isinstance(view, dict) else view
    # Each aircraft's flights take its live packs in turn; the place comes from the clip.
    rot = {}
    for f in sorted(flights, key=lambda f: f["flight"]["start"]):
        live = [p[0] for p in PACKS if p[4] == f.get("aircraft") and not p[3]]
        if not live:
            continue
        i = rot.get(f["aircraft"], 0)
        rot[f["aircraft"]] = i + 1
        if f.get("pack") != live[i % len(live)]:
            cli("gear", "flights", "set", f["flight"]["id"], "--pack", live[i % len(live)])
    for p in cli("gear", "packs")["packs"]:
        if p["pack"]["label"] in CHARGED and not p["pack"].get("charged_at"):
            cli("gear", "packs", "save", p["pack"]["label"], "--charged")
    logged = cli("gear", "crashes")
    logged = logged if isinstance(logged, list) else logged.get("crashes", [])
    for name, t, aircraft, broke, parts, note, repaired in REPAIRS:
        if any(c.get("broke") == broke for c in logged) or name not in clips:
            continue
        c = clips[name]
        cli("gear", "crashes", "--aircraft", aircraft, "--clip", c["id"], "save", "--time", round(c["duration"] * t, 1),
            "--day", c["date"], "--broke", broke, "--parts", parts, "--note", note,
            "--repaired", str(repaired).lower())


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument("home", type=Path, help="the throwaway HOME to fill")
    ap.add_argument("--cli", type=Path, default=REPO / "src-tauri/target/debug/quadcam-cli")
    ap.add_argument("--own-gear", action="store_true",
                    help="point gearDir at HOME's own gear folder and seed gear data there")
    ap.add_argument("--force", action="store_true",
                    help="delete the seeded library and sources in HOME first")
    a = ap.parse_args()
    home = a.home.expanduser().resolve()
    check_home(home)
    if not a.cli.exists():
        die(f"no quadcam-cli at {a.cli}: build it (cargo build --bin quadcam-cli in src-tauri)")
    if not shutil.which("ffmpeg"):
        die("needs ffmpeg (brew install ffmpeg)")
    work = home / ".dev-seed"
    if a.force:
        for p in (home / "Movies/quadcam", work):
            if p.exists() and p.resolve().is_relative_to(home):
                shutil.rmtree(p)
    cli = Cli(a.cli.resolve(), home)
    print("dev-seed: places, profiles, settings")
    settings = seed_setup(cli, home, a.own_gear)
    out = Path(settings["effective"]["output_dir"]).resolve()
    if not out.is_relative_to(home):
        die(f"the library folder {out} is outside {home}; unset outputDir in its settings.json")
    print("dev-seed: clips and radio logs")
    groups, log_dir = sources(work)
    seed_clips(cli, groups, log_dir)
    clips = seed_ratings(cli)
    gear = gear_dir_of(settings, home).resolve()
    if gear.is_relative_to(home):
        print(f"dev-seed: packs, flights, repairs in {gear}")
        seed_gear(cli, log_dir, clips)
    else:
        print("dev-seed: gearDir is outside HOME; skipped packs, flights and repairs (--own-gear seeds them)")
    print(f"dev-seed: done, {len(clips)} clips")


if __name__ == "__main__":
    main()
