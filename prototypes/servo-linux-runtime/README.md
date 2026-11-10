# ServoShell Linux runtime probe

Research fixture for [issue #31](https://github.com/realkaniewest/search/issues/31). Uses the official [Servo v0.8.0](https://github.com/servo/servo/releases/tag/v0.8.0) x86_64 Linux archive, pinned by SHA-256 before extraction and execution. It opens a page served only on `127.0.0.1`; JavaScript changes a large rectangle from gray to teal. The script checks the HTTP request and pixels captured from the virtual display.

Run on Ubuntu 24.04 with Xvfb, ImageMagick and Servo's documented runtime libraries:

```sh
timeout 180s xvfb-run -a --server-args='-screen 0 1280x800x24' bash prototypes/servo-linux-runtime/run.sh
```

This proves only this narrow ServoShell scenario. It does not test embedding Servo in Search, user input, real websites, profiles, media, downloads, speed or physical Linux displays.
