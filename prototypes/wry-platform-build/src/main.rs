// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

#[cfg(target_os = "windows")]
use wry::WebViewBuilderExtWindows;
use wry::{PermissionKind, PermissionResponse, WebViewBuilder};

fn main() {
    let builder = WebViewBuilder::new()
        .with_html("<h1>Search platform build probe</h1>")
        .with_incognito(true)
        .with_permission_handler(|_: PermissionKind| PermissionResponse::Deny);

    #[cfg(target_os = "windows")]
    let _builder = builder.with_additional_browser_args("");
    #[cfg(not(target_os = "windows"))]
    let _builder = builder;

    // No window or WebView is created: this probe only checks compilation.
}
