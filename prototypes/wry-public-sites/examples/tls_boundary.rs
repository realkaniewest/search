// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

#[cfg(not(target_os = "macos"))]
compile_error!("This TLS boundary probe measures WKWebView on macOS.");

use std::{env, thread, time::Duration};
use tao::{
    event::Event,
    event_loop::{ControlFlow, EventLoopBuilder},
    window::WindowBuilder,
};
use url::Url;
use wry::{NewWindowResponse, PageLoadEvent, PermissionResponse, WebViewBuilder};

enum ProbeEvent {
    Finished(String),
    Deadline,
}

fn main() {
    let raw = env::args().nth(1).expect("pass a local TLS fixture URL");
    let fixture = Url::parse(&raw).expect("invalid fixture URL");
    assert_eq!(fixture.scheme(), "https");
    assert_eq!(fixture.host_str(), Some("127.0.0.1"));
    assert!(fixture.port().is_some());
    assert!(fixture.username().is_empty() && fixture.password().is_none());

    let event_loop = EventLoopBuilder::<ProbeEvent>::with_user_event().build();
    let window = WindowBuilder::new()
        .with_title("Search Wry TLS boundary probe")
        .build(&event_loop)
        .expect("create probe window");
    let events = event_loop.create_proxy();
    let allowed = fixture.clone();
    let webview = WebViewBuilder::new()
        .with_url(fixture.as_str())
        .with_incognito(true)
        .with_permission_handler(|_| PermissionResponse::Deny)
        .with_download_started_handler(|_, _| false)
        .with_new_window_req_handler(|_, _| NewWindowResponse::Deny)
        .with_navigation_handler(move |target| Url::parse(&target).is_ok_and(|url| url == allowed))
        .with_on_page_load_handler(move |event, url| {
            if matches!(event, PageLoadEvent::Finished) {
                let _ = events.send_event(ProbeEvent::Finished(url));
            }
        })
        .build(&window)
        .expect("create probe WebView");
    let deadline = event_loop.create_proxy();
    thread::spawn(move || {
        thread::sleep(Duration::from_secs(8));
        let _ = deadline.send_event(ProbeEvent::Deadline);
    });

    event_loop.run(move |event, _, control_flow| {
        let _keep_alive = (&window, &webview);
        match event {
            Event::UserEvent(ProbeEvent::Finished(url)) if url == fixture.as_str() => {
                eprintln!("FAIL: TLS fixture reported a finished document");
                *control_flow = ControlFlow::ExitWithCode(1);
            }
            Event::UserEvent(ProbeEvent::Deadline) => {
                println!("OBSERVED: TLS fixture did not finish loading");
                *control_flow = ControlFlow::ExitWithCode(0);
            }
            _ => *control_flow = ControlFlow::Wait,
        }
    });
}
