#!/usr/bin/env bash
# Builds the synthetic part of the local test corpus in test-clips/synthetic/.
# Git ignores the media; test-clips/README.md records the expected values.
set -euo pipefail

cd "$(dirname "$0")/.."
out=test-clips/synthetic
mkdir -p "$out"
PATH="/opt/homebrew/bin:/usr/local/bin:$PATH"

clip() { # name seconds [audio]
  local audio=()
  [[ "${3:-}" == audio ]] && audio=(-f lavfi -i sine=frequency=440:sample_rate=32000 -c:a pcm_s16le -ac 1)
  ffmpeg -v error -y -f lavfi -i testsrc=size=720x480:rate=30 "${audio[@]}" \
    -t "$2" -c:v mjpeg -q:v 5 -pix_fmt yuvj420p -f avi "$out/$1"
}

clip PICT0001.AVI 5 audio
clip PICT0002.AVI 120 audio
clip PICT0003.AVI 3
# Half-written stand-in: the first half of a 4 s clip.
clip full.tmp 4 audio
head -c $(( $(wc -c < "$out/full.tmp") / 2 )) "$out/full.tmp" > "$out/PICT0004.AVI"
rm "$out/full.tmp"
: > "$out/PICT0005.AVI"

for f in "$out"/PICT*.AVI; do
  pk=$(ffprobe -v error -count_packets -select_streams v:0 -show_entries stream=nb_read_packets -of csv=p=0 "$f" 2>/dev/null || echo -)
  dur=$(ffprobe -v error -show_entries format=duration -of csv=p=0 "$f" 2>/dev/null || echo -)
  printf '%s\t%s s\t%s packets\t%s bytes\n' "$(basename "$f")" "$dur" "$pk" "$(wc -c < "$f" | tr -d " ")"
done
