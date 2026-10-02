# Photos

QuadCam can add verified clips to the Photos app, into an album.

- **On import:** the Finish step has an **Add to Photos** panel. It shows the album and one button that adds every verified file not yet in Photos.
- **In the library:** right-click a clip and select **Add to *album***. This command shows when Settings names a Photos album.
- **Share** opens the macOS Share menu, which also includes Photos.

**Add to Photos** adds a clip's cuts with it. QuadCam adds only files that it verified.

## Album

Settings > Photos sets the album. The default is "Drone". QuadCam makes the album if it is missing. An empty album means the library only.

## Permission

QuadCam uses PhotoKit. The first time, macOS asks for permission:

- With an album set, macOS asks for full library access, because QuadCam must find the album.
- With no album, macOS asks only for permission to add.

If you deny access, QuadCam shows a message that points to System Settings > Privacy & Security > Photos.

Use the app to add to Photos. If you use the command-line tool or a headless MCP server, macOS asks for permission for your terminal app instead.

## Test without Photos

The environment variable `QUADCAM_PHOTOS` controls the Photos access:

| Value | Effect |
|---|---|
| `real` | Use PhotoKit |
| `dry-run` | Add nothing. Report what would be added |
| unset | Use PhotoKit, except in processes started by `cargo`: those always use `dry-run` |
