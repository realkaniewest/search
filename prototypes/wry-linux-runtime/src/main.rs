// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

#[cfg(not(target_os = "linux"))]
compile_error!("This research prototype runs only on Linux.");

use std::{env, thread, time::Duration};
use tao::{
    event::{Event, WindowEvent},
    event_loop::{ControlFlow, EventLoopBuilder},
    platform::unix::WindowExtUnix,
    window::WindowBuilder,
};
use wry::{PermissionKind, PermissionResponse, WebViewBuilder, WebViewBuilderExtUnix};

const READY_TITLE: &str = "Search Linux smoke ready";
const INPUT_TITLE: &str = "Search Linux smoke input ready";
const HTML: &str = r#"<!doctype html>
<meta charset="utf-8">
<title>Search Linux smoke pending</title>
<style>
  body { margin: 0; background: white; font: 20px sans-serif; }
  #color { position: fixed; left: 40px; top: 70px; width: 320px; height: 140px; background: #14b8a6; }
  button { position: fixed; left: 40px; top: 250px; width: 300px; height: 100px; }
  input { position: fixed; left: 40px; top: 370px; width: 300px; height: 40px; }
</style>
<h1>Search Linux smoke</h1>
<p id="status">pending</p>
<div id="color" aria-label="Color probe"></div>
<button id="click">Click probe</button>
<input id="typing" aria-label="Typing probe">
<script>
  let clicked = false;
  document.getElementById('click').addEventListener('click', () => {
    clicked = true;
    document.getElementById('typing').focus();
  });
  document.getElementById('typing').addEventListener('input', event => {
    if (clicked && event.target.value === 'SEARCH_INPUT_OK') {
      document.title = 'Search Linux smoke input ready';
    }
  });
  document.getElementById('status').textContent = 'ready';
  document.title = 'Search Linux smoke ready';
</script>
"#;

#[derive(Debug)]
enum ProbeEvent {
    Ready,
    InputReady,
    TimedOut,
}

fn main() {
    let interactive = env::var_os("SEARCH_LINUX_INTERACTIVE").is_some();
    let event_loop = EventLoopBuilder::<ProbeEvent>::with_user_event().build();
    let window = WindowBuilder::new()
        .with_title("Search Linux Wry runtime probe")
        .build(&event_loop)
        .expect("failed to create Tao window");

    let ready_proxy = event_loop.create_proxy();
    let builder = WebViewBuilder::new()
        .with_html(HTML)
        .with_incognito(true)
        .with_permission_handler(|_: PermissionKind| PermissionResponse::Deny)
        .with_document_title_changed_handler(move |title| {
            if title == READY_TITLE {
                eprintln!("TITLE {READY_TITLE}");
                let _ = ready_proxy.send_event(ProbeEvent::Ready);
            } else if title == INPUT_TITLE {
                eprintln!("TITLE {INPUT_TITLE}");
                let _ = ready_proxy.send_event(ProbeEvent::InputReady);
            }
        });
    let vbox = window.default_vbox().expect("Tao GTK container is missing");
    let webview = builder
        .build_gtk(vbox)
        .expect("failed to create WebKitGTK view");

    let timeout_proxy = event_loop.create_proxy();
    thread::spawn(move || {
        thread::sleep(Duration::from_secs(if interactive { 60 } else { 20 }));
        let _ = timeout_proxy.send_event(ProbeEvent::TimedOut);
    });

    event_loop.run(move |event, _, control_flow| {
        let _keep_window_alive = &window;
        let _keep_webview_alive = &webview;
        *control_flow = ControlFlow::Wait;
        match event {
            Event::UserEvent(ProbeEvent::Ready) => {
                if interactive {
                    eprintln!("READY: waiting for screenshot and input");
                } else {
                    eprintln!("PASS: local HTML changed the document title via JavaScript");
                    *control_flow = ControlFlow::ExitWithCode(0);
                }
            }
            Event::UserEvent(ProbeEvent::InputReady) => {
                eprintln!("PASS: X11 click and keyboard input reached the page");
                *control_flow = ControlFlow::ExitWithCode(0);
            }
            Event::UserEvent(ProbeEvent::TimedOut) => {
                eprintln!("FAIL: expected title marker did not arrive");
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
