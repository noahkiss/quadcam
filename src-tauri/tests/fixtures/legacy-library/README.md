# legacy-library

A library folder that QuadCam 0.4.1's `quadcam-cli` built, with its `.quadcam/index.json`.
`tests/migration.rs` opens it with the current code. Do not rebuild it with a newer
version: its point is the ids 0.4.1 wrote.

It was made from two tiny synthetic DVR clips (160x120 MJPEG): `import --keep-originals
--cut 0=0.2-0.9` for `loops` and a plain import for `hover`, then a `library rebuild` that
adopted `2025-05-01_old-export.mp4` (made by ffmpeg, not QuadCam), `library rate`,
`library cut` without `--export` (unsaved cuts), and `library cut --clear --removed keep`
(which made `loops_cut1` a clip of its own).
