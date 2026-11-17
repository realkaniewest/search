// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

use std::{env, process, thread, time::Duration};
use tao::{
    event::{Event, WindowEvent},
    event_loop::{ControlFlow, EventLoopBuilder},
    window::WindowBuilder,
};
use url::{Host, Url};
use wry::{NewWindowResponse, PermissionKind, PermissionResponse, WebViewBuilder};

#[cfg(target_os = "linux")]
use tao::platform::unix::WindowExtUnix;
#[cfg(target_os = "linux")]
use wry::WebViewBuilderExtUnix;
#[cfg(target_os = "windows")]
use wry::WebViewBuilderExtWindows;

const DEFAULT_URL: &str = "http://127.0.0.1:43130/scenario.html";
const READY_TITLE: &str = "Search comparison ready";

#[derive(Debug)]
enum ProbeEvent {
    Ready,
    TimedOut,
}

fn validate_url(value: &str) -> Result<Url, &'static str> {
    let url = Url::parse(value).map_err(|_| "invalid URL")?;
    if url.scheme() != "http" {
        return Err("only local HTTP URLs are allowed");
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err("URL credentials are not allowed");
    }
    match url.host() {
        Some(Host::Ipv4(address)) if address.is_loopback() => {}
        Some(Host::Ipv6(address)) if address.is_loopback() => {}
        _ => return Err("only loopback IP addresses are allowed"),
    }
    if url.port().is_none() {
        return Err("an explicit local port is required");
    }
    Ok(url)
}

fn deny_permission(_: PermissionKind) -> PermissionResponse {
    PermissionResponse::Deny
}

fn allowed_navigation(base: &Url, target: &str) -> bool {
    validate_url(target).is_ok_and(|url| url.origin() == base.origin())
}

fn main() {
    let mut arguments = env::args();
    let _program = arguments.next();
    let input = arguments.next().unwrap_or_else(|| DEFAULT_URL.to_owned());
    if arguments.next().is_some() {
        eprintln!("FAIL: expected at most one local URL argument");
        process::exit(2);
    }
    let url = match validate_url(&input) {
        Ok(url) => url,
        Err(reason) => {
            eprintln!("FAIL: {reason}");
            process::exit(2);
        }
    };

    let event_loop = EventLoopBuilder::<ProbeEvent>::with_user_event().build();
    let window = WindowBuilder::new()
        .with_title("Search Wry comparison probe")
        .build(&event_loop)
        .expect("failed to create Tao window");

    let ready_proxy = event_loop.create_proxy();
    let navigation_origin = url.clone();
    let builder = WebViewBuilder::new()
        .with_url(url.as_str())
        .with_incognito(true)
        .with_permission_handler(deny_permission)
        .with_navigation_handler(move |target| allowed_navigation(&navigation_origin, &target))
        .with_new_window_req_handler(|_, _| NewWindowResponse::Deny)
        .with_document_title_changed_handler(move |title| {
            if title == READY_TITLE {
                let _ = ready_proxy.send_event(ProbeEvent::Ready);
            }
        });
    #[cfg(target_os = "windows")]
    let builder = builder.with_additional_browser_args("");

    #[cfg(target_os = "linux")]
    let webview = {
        let vbox = window.default_vbox().expect("Tao GTK container is missing");
        builder
            .build_gtk(vbox)
            .expect("failed to create WebKitGTK view")
    };
    #[cfg(not(target_os = "linux"))]
    let webview = builder
        .build(&window)
        .expect("failed to create native WebView");

    let timeout_proxy = event_loop.create_proxy();
    thread::spawn(move || {
        thread::sleep(Duration::from_secs(30));
        let _ = timeout_proxy.send_event(ProbeEvent::TimedOut);
    });

    event_loop.run(move |event, _, control_flow| {
        let _keep_window_alive = &window;
        let _keep_webview_alive = &webview;
        *control_flow = ControlFlow::Wait;
        match event {
            Event::UserEvent(ProbeEvent::Ready) => {
                println!("PASS: local fixture changed the document title to {READY_TITLE}");
                *control_flow = ControlFlow::ExitWithCode(0);
            }
            Event::UserEvent(ProbeEvent::TimedOut) => {
                eprintln!("FAIL: fixture did not report ready within 30 seconds");
                *control_flow = ControlFlow::ExitWithCode(1);
            }
            Event::WindowEvent {
                event: WindowEvent::CloseRequested,
                ..
            } => {
                eprintln!("FAIL: window closed before fixture was ready");
                *control_flow = ControlFlow::ExitWithCode(1);
            }
            _ => {}
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_only_loopback_http() {
        assert!(validate_url(DEFAULT_URL).is_ok());
        assert!(validate_url("http://[::1]:43130/scenario.html").is_ok());
        for invalid in [
            "https://127.0.0.1:43130/scenario.html",
            "http://example.com:43130/scenario.html",
            "http://127.0.0.1.evil.test:43130/scenario.html",
            "http://user:pass@127.0.0.1:43130/scenario.html",
            "file:///etc/passwd",
            "http://127.0.0.1/scenario.html",
        ] {
            assert!(
                validate_url(invalid).is_err(),
                "unexpectedly allowed {invalid}"
            );
        }
    }

    #[test]
    fn denies_all_permission_kinds() {
        for kind in [
            PermissionKind::Camera,
            PermissionKind::Microphone,
            PermissionKind::Geolocation,
        ] {
            assert_eq!(deny_permission(kind), PermissionResponse::Deny);
        }
    }

    #[test]
    fn navigation_stays_on_initial_origin() {
        let base = validate_url(DEFAULT_URL).unwrap();
        assert!(allowed_navigation(
            &base,
            "http://127.0.0.1:43130/start.html"
        ));
        assert!(!allowed_navigation(
            &base,
            "http://127.0.0.1:43131/scenario.html"
        ));
        assert!(!allowed_navigation(
            &base,
            "http://127.0.0.2:43130/scenario.html"
        ));
        assert!(!allowed_navigation(
            &base,
            "http://[::1]:43130/scenario.html"
        ));
        assert!(!allowed_navigation(
            &base,
            "https://127.0.0.1:43130/scenario.html"
        ));
    }
}
