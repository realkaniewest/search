// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

#[cfg(not(target_os = "macos"))]
compile_error!("This HTTPS window probe is for WKWebView on macOS.");

use std::{env, process::ExitCode};
use tao::{
    event::{Event, WindowEvent},
    event_loop::{ControlFlow, EventLoop},
    window::WindowBuilder,
};
use url::{Host, Url};
use wry::{NewWindowResponse, PermissionResponse, WebViewBuilder};

fn https_address(raw: &str) -> Option<Url> {
    let (scheme, rest) = raw.split_once("://")?;
    if !scheme.eq_ignore_ascii_case("https")
        || raw.contains('\\')
        || raw.chars().any(|ch| ch.is_whitespace() || ch.is_control())
    {
        return None;
    }
    let authority = rest.split(['/', '?', '#']).next()?;
    if authority.is_empty() || authority.contains('@') {
        return None;
    }
    let url = Url::parse(raw).ok()?;
    let Host::Domain(host) = url.host()? else {
        return None;
    };
    let host = host.trim_end_matches('.');
    (url.port_or_known_default() == Some(443)
        && host.contains('.')
        && !host.ends_with(".localhost")
        && !host.ends_with(".local"))
    .then_some(url)
}

fn main() -> ExitCode {
    let mut args = env::args().skip(1);
    let Some(url) = args.next().as_deref().and_then(https_address) else {
        eprintln!("Usage: https_window https://public-host.example/path");
        return ExitCode::FAILURE;
    };
    if args.next().is_some() {
        eprintln!("Pass exactly one HTTPS address.");
        return ExitCode::FAILURE;
    }

    let event_loop = EventLoop::new();
    let window = WindowBuilder::new()
        .with_title("Search HTTPS experiment")
        .build(&event_loop)
        .expect("create probe window");
    let webview = WebViewBuilder::new()
        .with_url(url.as_str())
        .with_incognito(true)
        .with_permission_handler(|_| PermissionResponse::Deny)
        .with_download_started_handler(|_, _| false)
        .with_new_window_req_handler(|_, _| NewWindowResponse::Deny)
        .with_navigation_handler(|target| https_address(&target).is_some())
        .build(&window)
        .expect("create probe WebView");

    event_loop.run(move |event, _, control_flow| {
        let _keep_alive = (&window, &webview);
        *control_flow = match event {
            Event::WindowEvent {
                event: WindowEvent::CloseRequested,
                ..
            } => ControlFlow::Exit,
            _ => ControlFlow::Wait,
        };
    });
}

#[test]
fn accepts_only_explicit_https_with_a_domain_and_default_port() {
    let url = https_address("HTTPS://EXAMPLE.ORG:443/@doc").unwrap();
    assert_eq!(url.as_str(), "https://example.org/@doc");
    for address in [
        "example.org",
        "http://example.org/",
        "https://example.org:8443/",
        "https://@example.org/",
        "https:///example.org/",
        "https://127.0.0.1/",
        "https://[::1]/",
        "https://service.local./",
        "https://service.localhost/",
        "https://localhost/",
        "file:///tmp/search-probe.html",
    ] {
        assert!(https_address(address).is_none(), "{address}");
    }
}
