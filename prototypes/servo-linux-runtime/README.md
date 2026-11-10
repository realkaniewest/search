# ServoShell Linux runtime probe

Research fixture for [issue #31](https://github.com/realkaniewest/search/issues/31). Uses the official [Servo v0.8.0](https://github.com/servo/servo/releases/tag/v0.8.0) x86_64 Linux archive, pinned by SHA-256 before extraction and execution. It opens a page served only on `127.0.0.1`; JavaScript changes a large rectangle from gray to teal. The script checks the HTTP request and pixels captured from the virtual display.

Run on Ubuntu 24.04 with Xvfb, ImageMagick and Servo's documented runtime libraries:

```sh
timeout 180s xvfb-run -a --server-args='-screen 0 1280x800x24' bash prototypes/servo-linux-runtime/run.sh
```

The screenshot covers the isolated Xvfb root display, so the color check is evidence of rendered content in this controlled display rather than a precise Servo-window capture. The preflight uses HTTP HEAD; the logged GET must come from ServoShell. The script attempts to stop its direct child processes and remove temporary files on normal exit, TERM and INT. If ServoShell spawns descendants, this cleanup alone does not prove they exited.

This proves only this narrow ServoShell scenario. It does not test embedding Servo in Search, user input, real websites, profiles, media, downloads, speed or physical Linux displays.
