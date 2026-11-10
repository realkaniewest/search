// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

#[cfg(not(target_os = "windows"))]
compile_error!("This research prototype runs only on Windows.");

use std::{thread, time::Duration};
use tao::{
    event::{Event, WindowEvent},
    event_loop::{ControlFlow, EventLoopBuilder},
    window::WindowBuilder,
};
use wry::{PermissionKind, PermissionResponse, WebViewBuilder, WebViewBuilderExtWindows};

const READY_TITLE: &str = "Search Windows smoke ready";
const HTML: &str = r#"<!doctype html>
<meta charset="utf-8">
<title>Search Windows smoke pending</title>
<h1>Search Windows smoke</h1>
<p id="status">pending</p>
<script>
  document.getElementById('status').textContent = 'ready';
  document.title = 'Search Windows smoke ready';
</script>
"#;

#[derive(Debug)]
enum ProbeEvent {
    Ready,
    TimedOut,
}

fn main() {
    let event_loop = EventLoopBuilder::<ProbeEvent>::with_user_event().build();
    let window = WindowBuilder::new()
        .with_title("Search Windows Wry runtime probe")
        .build(&event_loop)
        .expect("failed to create Tao window");

    let ready_proxy = event_loop.create_proxy();
    let builder = WebViewBuilder::new()
        .with_html(HTML)
        .with_incognito(true)
        .with_permission_handler(|_: PermissionKind| PermissionResponse::Deny)
        .with_additional_browser_args("")
        .with_document_title_changed_handler(move |title| {
            if title == READY_TITLE {
                eprintln!("TITLE {READY_TITLE}");
                let _ = ready_proxy.send_event(ProbeEvent::Ready);
            }
        });
    let webview = builder.build(&window).expect("failed to create WebView2");

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
                eprintln!("PASS: local HTML changed the document title via JavaScript");
                *control_flow = ControlFlow::ExitWithCode(0);
            }
            Event::UserEvent(ProbeEvent::TimedOut) => {
                eprintln!("FAIL: title did not change within 30 seconds");
                *control_flow = ControlFlow::ExitWithCode(1);
            }
            Event::WindowEvent {
                event: WindowEvent::CloseRequested,
                ..
            } => {
                eprintln!("FAIL: window closed before title marker");
                *control_flow = ControlFlow::ExitWithCode(1);
            }
            _ => {}
        }
    });
}
