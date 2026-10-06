# Test corpus

Local only. Git tracks this README; the media stays ignored. The values below keep results
comparable across changes.

| Folder | Holds | How to fill it |
|---|---|---|
| `card/` | Real `*.AVI` files from a DVR card: a short clip, a long clip holding several flights, and a half-written clip if a power-off ever makes one | Copy from a card |
| `logs/` | The EdgeTX CSVs from the same day | Copy `LOGS/` from the radio |
| `synthetic/` | MJPEG + PCM AVIs from `ffmpeg -f lavfi -i testsrc`, a hand-truncated copy, a zero-byte file, and a DJI-like card in `dji-card/` | `scripts/make-corpus.sh` |

The automated tests (`cargo test` in `src-tauri/`) build their own synthetic clips in temp
folders. They do not need this corpus.

## synthetic/

720x480, 30 fps, MJPEG `-q:v 5`, mono PCM 32 kHz. Every clip gets the import date and the
**unmatched** badge, since no log covers it.

| Clip | Duration | Video packets | Size (bytes) | Expected |
|---|---|---|---|---|
| PICT0001.AVI | 5.000 s | 150 | 3,074,396 | OK |
| PICT0002.AVI | 120.000 s | 3600 | 78,131,276 | OK |
| PICT0003.AVI | 3.000 s | 90 | 1,651,322 | OK, no audio |
| PICT0004.AVI | header says 2.000 s / 60 | — | 1,231,251 | First half of a 4 s clip: incomplete (no `idx1`, RIFF size wrong), recovered and verified against the recovered length |
| PICT0005.AVI | — | — | 0 | Empty, skipped by default |

### synthetic/dji-card/

A card laid out like a DJI O4 air unit over USB: `DCIM/DJI_001/` with one clip and its
`.SRT`, and `MISC/FC8770.db` (ignored). The clip is H.264 (libx264, `ultrafast`) 640x360 at
30000/1001 fps, with a 320x180 MJPEG cover as an attached picture, the format tags
`comment=EIS:RS;FOV:Linear;` and `creation_time=2026-01-05T17:00:00Z`, and `moov` last. It
has no DJI data streams (`djmd`, `dbgi`): ffmpeg's MP4 muxer cannot write them.

| Clip | Duration | Video packets | Size (bytes) | Expected |
|---|---|---|---|---|
| DJI_20260105120000_0001_D.MP4 | 3.003 s | 90 (the cover is not counted) | 76,698 | OK; date source **clip clock**, 2026-01-05 12:00 local; MP4 export is a byte copy plus metadata |

## card/ and logs/

Not recorded yet. Fill in per clip: duration, video packet count, size, expected date and
match badge. Record the card's folder layout too (some DVRs write to the root, some into `DCIM/`).

## Size and speed (check 8)

`cargo test --test import size_and_speed -- --ignored --nocapture` on an Apple Silicon Mac, 2026-09-30.
Set `QUADCAM_BENCH_SECS` to change the clip length.

| 120 s synthetic clip | Size | Time (convert + verify) |
|---|---|---|
| Source AVI | 78.1 MB | — |
| MP4 (H.264, VideoToolbox `-q:v 65`) | 5.5 MB (14x smaller) | 5.0 s (24x real time) |
| MOV (MJPEG remux) | 78.0 MB | 0.14 s |

`testsrc` compresses far better than noisy analog footage. Re-measure on the long real clip.
