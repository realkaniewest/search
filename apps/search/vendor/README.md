# Wry for Search

This directory contains Wry 0.57.1 from the crates.io package, based on upstream revision `2446feadae623d6f2ac8d2136e3ccef3d7197e0e`. See its original license files in the package directory.

The macOS patch in `wry/src/wkwebview/class/wry_web_view_ui_delegate.rs` presents the file chooser as a sheet and calls WebKit's completion block after confirmation or cancellation. A standalone modal open panel on the tested macOS 27.0.1 left the Open button disabled. The patch keeps WKWebView's file and directory selection options and passes only the URLs confirmed in the system sheet. The build script also declares the existing macOS availability cfg so Cargo recognizes it when compiling the local package.

When upgrading Wry, check whether the upstream chooser works on macOS before dropping the patch.
