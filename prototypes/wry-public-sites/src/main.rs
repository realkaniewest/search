// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

#[cfg(not(target_os = "macos"))]
compile_error!("This public-site research probe is macOS-only until other platforms are reviewed.");

use std::{env, process, thread, time::Duration};
use tao::{
    event::{Event, WindowEvent},
    event_loop::{ControlFlow, EventLoopBuilder},
    window::WindowBuilder,
};
use url::Url;
use wry::{NewWindowResponse, PageLoadEvent, PermissionKind, PermissionResponse, WebViewBuilder};

const TIMEOUT_SECS: u64 = 35;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Site {
    Rust,
    Wikipedia,
    Excalidraw,
}

impl Site {
    fn from_key(key: &str) -> Option<Self> {
        match key {
            "rust" => Some(Self::Rust),
            "wikipedia" => Some(Self::Wikipedia),
            "excalidraw" => Some(Self::Excalidraw),
            _ => None,
        }
    }

    fn key(self) -> &'static str {
        match self {
            Self::Rust => "rust",
            Self::Wikipedia => "wikipedia",
            Self::Excalidraw => "excalidraw",
        }
    }

    fn initial_url(self) -> &'static str {
        match self {
            Self::Rust => "https://rust-lang.org/",
            Self::Wikipedia => "https://www.wikipedia.org/",
            Self::Excalidraw => "https://excalidraw.com/",
        }
    }

    fn allows_host(self, host: &str) -> bool {
        match self {
            Self::Rust => matches!(host, "rust-lang.org" | "www.rust-lang.org"),
            Self::Wikipedia => host == "www.wikipedia.org",
            Self::Excalidraw => host == "excalidraw.com",
        }
    }

    fn dom_script(self) -> &'static str {
        match self {
            Self::Rust => {
                "(()=>{const e=document.querySelector('h1');const r=e?.getBoundingClientRect();return {host:location.hostname,title:document.title.slice(0,80),marker:!!e?.textContent?.includes('Rust')&&!!r&&r.width>0&&r.height>0}})()"
            }
            Self::Wikipedia => {
                "(()=>{const e=document.querySelector('input[name=search]');const r=e?.getBoundingClientRect();return {host:location.hostname,title:document.title.slice(0,80),marker:!!r&&r.width>0&&r.height>0}})()"
            }
            Self::Excalidraw => {
                "(()=>{const e=document.querySelector('canvas');const r=e?.getBoundingClientRect();return {host:location.hostname,title:document.title.slice(0,80),marker:!!r&&r.width>0&&r.height>0}})()"
            }
        }
    }
}

#[derive(Debug)]
enum ProbeEvent {
    LoadFinished(String),
    Inspect,
    Dom(String),
    TimedOut,
}

fn allowed_navigation(site: Site, target: &str) -> bool {
    let Ok(url) = Url::parse(target) else {
        return false;
    };
    url.scheme() == "https"
        && url.username().is_empty()
        && url.password().is_none()
        && url.port_or_known_default() == Some(443)
        && url.host_str().is_some_and(|host| site.allows_host(host))
}

fn deny_permission(_: PermissionKind) -> PermissionResponse {
    PermissionResponse::Deny
}

fn dom_matches(site: Site, raw: &str) -> bool {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(raw) else {
        return false;
    };
    let value = if let Some(inner) = value.as_str() {
        match serde_json::from_str::<serde_json::Value>(inner) {
            Ok(parsed) => parsed,
            Err(_) => return false,
        }
    } else {
        value
    };
    value.get("marker").and_then(|part| part.as_bool()) == Some(true)
        && value
            .get("title")
            .and_then(|part| part.as_str())
            .is_some_and(|title| !title.is_empty() && title.chars().count() <= 80)
        && value
            .get("host")
            .and_then(|part| part.as_str())
            .is_some_and(|host| site.allows_host(host))
}

fn main() {
    let arguments: Vec<String> = env::args().collect();
    if arguments.len() != 2 {
        eprintln!("Usage: search-wry-public-sites <rust|wikipedia|excalidraw>");
        process::exit(2);
    }
    let Some(site) = Site::from_key(&arguments[1]) else {
        eprintln!("FAIL: unknown site key");
        process::exit(2);
    };

    let event_loop = EventLoopBuilder::<ProbeEvent>::with_user_event().build();
    let window = WindowBuilder::new()
        .with_title(format!("Search Wry public-site probe: {}", site.key()))
        .build(&event_loop)
        .expect("failed to create Tao window");
    let load_proxy = event_loop.create_proxy();
    let builder = WebViewBuilder::new()
        .with_url(site.initial_url())
        .with_incognito(true)
        .with_permission_handler(deny_permission)
        .with_download_started_handler(|_, _| false)
        .with_new_window_req_handler(|_, _| NewWindowResponse::Deny)
        .with_navigation_handler(move |target| allowed_navigation(site, &target))
        .with_on_page_load_handler(move |event, url| {
            if matches!(event, PageLoadEvent::Finished) {
                let _ = load_proxy.send_event(ProbeEvent::LoadFinished(url));
            }
        });
    let webview = builder
        .build(&window)
        .expect("failed to create native WebView");

    let timer_proxy = event_loop.create_proxy();
    thread::spawn(move || {
        for _ in 0..(TIMEOUT_SECS / 2) {
            thread::sleep(Duration::from_secs(2));
            let _ = timer_proxy.send_event(ProbeEvent::Inspect);
        }
        thread::sleep(Duration::from_secs(1));
        let _ = timer_proxy.send_event(ProbeEvent::TimedOut);
    });

    let dom_proxy = event_loop.create_proxy();
    let mut loaded = false;
    event_loop.run(move |event, _, control_flow| {
        let _keep_window_alive = &window;
        *control_flow = ControlFlow::Wait;
        match event {
            Event::UserEvent(ProbeEvent::LoadFinished(url)) => {
                if allowed_navigation(site, &url) {
                    loaded = true;
                } else {
                    eprintln!("FAIL: page finished at an unexpected URL");
                    *control_flow = ControlFlow::ExitWithCode(1);
                }
            }
            Event::UserEvent(ProbeEvent::Inspect) if loaded => {
                let proxy = dom_proxy.clone();
                if webview
                    .evaluate_script_with_callback(site.dom_script(), move |raw| {
                        let _ = proxy.send_event(ProbeEvent::Dom(raw));
                    })
                    .is_err()
                {
                    eprintln!("FAIL: could not inspect the loaded page");
                    *control_flow = ControlFlow::ExitWithCode(1);
                }
            }
            Event::UserEvent(ProbeEvent::Dom(raw)) if dom_matches(site, &raw) => {
                println!(
                    "PASS: {} loaded with expected bounded DOM marker",
                    site.key()
                );
                println!(
                    "LIMIT: no clicks, search, drawing, save, screenshot, or speed comparison"
                );
                *control_flow = ControlFlow::ExitWithCode(0);
            }
            Event::UserEvent(ProbeEvent::TimedOut) => {
                eprintln!("FAIL: no expected DOM marker within {TIMEOUT_SECS} seconds");
                *control_flow = ControlFlow::ExitWithCode(1);
            }
            Event::WindowEvent {
                event: WindowEvent::CloseRequested,
                ..
            } => {
                eprintln!("FAIL: window closed before the DOM marker was observed");
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
    fn fixed_keys_and_urls() {
        assert_eq!(Site::from_key("rust"), Some(Site::Rust));
        assert_eq!(Site::from_key("wikipedia"), Some(Site::Wikipedia));
        assert_eq!(Site::from_key("excalidraw"), Some(Site::Excalidraw));
        assert_eq!(Site::from_key("https://example.com"), None);
        for site in [Site::Rust, Site::Wikipedia, Site::Excalidraw] {
            assert!(allowed_navigation(site, site.initial_url()));
        }
    }

    #[test]
    fn navigation_rejects_cross_site_and_local_privileges() {
        let site = Site::Rust;
        assert!(allowed_navigation(site, "https://www.rust-lang.org/learn"));
        for target in [
            "http://rust-lang.org/",
            "https://user:pass@rust-lang.org/",
            "https://rust-lang.org:444/",
            "https://rust-lang.org.evil.example/",
            "https://www.wikipedia.org/",
            "http://127.0.0.1:43130/",
            "file:///etc/passwd",
            "javascript:alert(1)",
            "mailto:someone@example.com",
        ] {
            assert!(!allowed_navigation(site, target), "allowed {target}");
        }
    }

    #[test]
    fn dom_marker_requires_host_title_and_boolean() {
        assert!(dom_matches(
            Site::Wikipedia,
            r#"{"host":"www.wikipedia.org","title":"Wikipedia","marker":true}"#
        ));
        for raw in [
            r#"{"host":"evil.example","title":"Wikipedia","marker":true}"#,
            r#"{"host":"www.wikipedia.org","title":"","marker":true}"#,
            r#"{"host":"www.wikipedia.org","title":"Wikipedia","marker":"true"}"#,
            "not json",
        ] {
            assert!(!dom_matches(Site::Wikipedia, raw), "accepted {raw}");
        }
    }

    #[test]
    fn sensitive_permissions_are_denied() {
        for kind in [
            PermissionKind::Camera,
            PermissionKind::Microphone,
            PermissionKind::Geolocation,
        ] {
            assert_eq!(deny_permission(kind), PermissionResponse::Deny);
        }
    }
}
