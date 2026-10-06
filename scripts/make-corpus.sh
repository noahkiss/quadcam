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

# A DJI-like card: DCIM/DJI_001/ with one clip and its .SRT, and DJI's MISC/ housekeeping.
# H.264 at 30000/1001 fps with an attached cover picture and DJI's comment tag. DJI's own
# data streams (djmd, dbgi) are not reproduced: ffmpeg's MP4 muxer cannot write them.
dji="$out/dji-card"
rm -rf "$dji"
mkdir -p "$dji/DCIM/DJI_001" "$dji/MISC/THM"
printf 'housekeeping' > "$dji/MISC/FC8770.db"
ffmpeg -v error -y -f lavfi -i testsrc=size=320x180 -frames:v 1 "$out/cover.tmp.jpg"
ffmpeg -v error -y -f lavfi -i testsrc=size=640x360:rate=30000/1001 -i "$out/cover.tmp.jpg" \
  -map 0:v -map 1:v -t 3 -c:v:0 libx264 -preset ultrafast -pix_fmt yuv420p \
  -c:v:1 copy -disposition:v:1 attached_pic \
  -metadata comment="EIS:RS;FOV:Linear;" -metadata creation_time=2026-01-05T17:00:00Z \
  -f mp4 "$dji/DCIM/DJI_001/DJI_20260105120000_0001_D.MP4"
rm "$out/cover.tmp.jpg"
printf '1\n00:00:00,000 --> 00:00:01,000\nsubtitle\n' > "$dji/DCIM/DJI_001/DJI_20260105120000_0001_D.SRT"

for f in "$out"/PICT*.AVI "$dji"/DCIM/DJI_001/*.MP4; do
  pk=$(ffprobe -v error -count_packets -select_streams v:0 -show_entries stream=nb_read_packets -of csv=p=0 "$f" 2>/dev/null || echo -)
  dur=$(ffprobe -v error -show_entries format=duration -of csv=p=0 "$f" 2>/dev/null || echo -)
  printf '%s\t%s s\t%s packets\t%s bytes\n' "$(basename "$f")" "$dur" "$pk" "$(wc -c < "$f" | tr -d " ")"
done
