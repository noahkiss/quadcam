#!/usr/bin/env bash
# Records the mock core's data from the real core: runs quadcam-cli on the synthetic corpus
# with a temporary HOME, then writes its JSON answers and small media to e2e/fixtures/.
# Paths are rewritten to a neutral home (/Users/pilot). Needs `cargo build --bin quadcam-cli`
# and `scripts/make-corpus.sh` first. The parity specs read these files through ipc/mock.ts.
set -euo pipefail

cd "$(dirname "$0")/.."
repo=$(cd .. && pwd)
cli="$repo/src-tauri/target/debug/quadcam-cli"
corpus="$repo/test-clips/synthetic"
out=e2e/fixtures
PATH="/opt/homebrew/bin:/usr/local/bin:$PATH"
[[ -x "$cli" ]] || { echo "build the CLI first: cargo build --bin quadcam-cli" >&2; exit 1; }
[[ -d "$corpus" ]] || { echo "build the corpus first: scripts/make-corpus.sh" >&2; exit 1; }

# A short HOME: the control socket path must fit in sockaddr_un.
home=$(mktemp -d /tmp/qcfix.XXXX)
trap 'rm -rf "$home"' EXIT
export HOME="$home" QUADCAM_PHOTOS=dry-run
q() { "$cli" --json "$@" 2>/dev/null; }
# The result of one call, paths made neutral.
save() { node -e '
  const [home, repo, file] = process.argv.slice(1);
  let t = require("fs").readFileSync(0, "utf8");
  t = t.split(home).join("/Users/pilot").split(repo).join("/Users/pilot/quadcam");
  const r = JSON.parse(t);
  if (!r.ok) { console.error(JSON.stringify(r.error)); process.exit(1); }
  require("fs").writeFileSync(file, JSON.stringify(r.result, null, 1) + "\n");
' "$home" "$repo" "$out/$1.json"; }

rm -rf "$out" && mkdir -p "$out/media"
q profiles save Whoop --aircraft "Tiny whoop" --camera-make Generic --models WHOOP --default >/dev/null
q profiles save Five-inch --aircraft "5 inch freestyle" --camera-make Generic --models FIVE >/dev/null
q places save "Home field" --location 40.0,-75.0 >/dev/null
q places save "Riverside park" --location 40.1,-75.1 >/dev/null

q stage "$corpus" >/dev/null
q analyze >/dev/null
q dates --no-logs --set 0=2026-09-27 --set 1=2026-09-27 --set 2=2026-09-28 --set 3=2026-09-28 \
  --time 0=10:15 --time 1=10:40 --time 2=17:05 >/dev/null
q meta 0 1 --place "Home field" --keywords backyard >/dev/null
q meta 2 --place "Riverside park" --profile Five-inch >/dev/null
q cut 1 10-25 >/dev/null
q show | save session-review

q import --name 0=backyard-loops --name 1=gap-run --name 2=river-dive --note "1=Two packs" \
  --skip 3 --skip 4 --format mp4 --output "$home/Movies/quadcam" >/dev/null
q show | save session-finished
q library list | save library
q settings | save settings
q gear osd "$repo/src-tauri/tests/fixtures/osd/pal-synthetic.dump_all.txt" | save osd

# Media: session thumbnails, a 10-frame strip per library clip, and one small MP4 that
# stands in for every preview.
cp "$home"/Library/Caches/app.quadcam/thumbs/*.jpg "$out/media/"
for f in "$home"/Movies/quadcam/*/*/*.mp4; do
  n=$(basename "$f" .mp4)
  [[ $n == *_cut* ]] && continue
  d=$(ffprobe -v error -show_entries format=duration -of csv=p=0 "$f")
  ffmpeg -v error -y -i "$f" -vf "fps=10/$d,scale=160:-2,tile=10x1" -frames:v 1 -q:v 6 "$out/media/$n-strip.jpg"
done
cp "$home"/Movies/quadcam/2026/2026-09-28/*river-dive.mp4 "$out/media/preview.mp4"
ls -la "$out" "$out/media"
