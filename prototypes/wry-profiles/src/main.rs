// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

#[cfg(not(target_os = "macos"))]
compile_error!("This profile prototype is limited to macOS until other platforms are reviewed.");

use std::{
    collections::HashMap,
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    thread,
};

use tao::{
    dpi::{LogicalPosition, LogicalSize},
    event::{Event, WindowEvent},
    event_loop::{ControlFlow, EventLoop, EventLoopWindowTarget},
    window::{Window, WindowBuilder, WindowId},
};
use wry::{WebView, WebViewBuilder, WebViewBuilderExtDarwin};

const ADDRESS: &str = "127.0.0.1:43127";
const PROFILE_A: [u8; 16] = *b"SearchProfileA01";
const PROFILE_B: [u8; 16] = *b"SearchProfileB02";
const PAGE: &str = include_str!("profile.html");

fn serve(mut stream: TcpStream) -> std::io::Result<()> {
    stream.set_read_timeout(Some(std::time::Duration::from_secs(5)))?;
    let mut request_line = Vec::with_capacity(128);
    while request_line.len() < 2048 {
        let mut byte = [0_u8; 1];
        if stream.read(&mut byte)? == 0 {
            break;
        }
        request_line.push(byte[0]);
        if request_line.ends_with(b"\r\n") {
            break;
        }
    }
    let path_is_root = request_line.starts_with(b"GET / HTTP/")
        && request_line.ends_with(b"\r\n")
        && request_line.len() < 2048;
    let (status, body) = if path_is_root {
        ("200 OK", PAGE)
    } else {
        ("404 Not Found", "Not found")
    };
    let response = format!(
        "HTTP/1.1 {status}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nCache-Control: no-store\r\nX-Content-Type-Options: nosniff\r\nContent-Security-Policy: default-src 'none'; script-src 'unsafe-inline'; style-src 'unsafe-inline'\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    stream.write_all(response.as_bytes())
}

fn create_window(
    event_loop: &EventLoopWindowTarget<()>,
    title: &str,
    identifier: Option<[u8; 16]>,
    position: (f64, f64),
) -> Result<(WebView, Window), Box<dyn std::error::Error>> {
    let window = WindowBuilder::new()
        .with_title(title)
        .with_inner_size(LogicalSize::new(500.0, 560.0))
        .with_position(LogicalPosition::new(position.0, position.1))
        .build(event_loop)?;
    let builder = WebViewBuilder::new().with_url(format!("http://{ADDRESS}/"));
    let builder = match identifier {
        Some(identifier) => builder.with_data_store_identifier(identifier),
        None => builder.with_incognito(true),
    };
    let webview = builder.build(&window)?;
    Ok((webview, window))
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let listener = TcpListener::bind(ADDRESS)?;
    thread::spawn(move || {
        for stream in listener.incoming() {
            match stream {
                Ok(stream) => {
                    if let Err(error) = serve(stream) {
                        eprintln!("Local test page request failed: {error}");
                    }
                }
                Err(error) => eprintln!("Local test page listener failed: {error}"),
            }
        }
    });

    let event_loop = EventLoop::new();
    // Store the WebView first so it is dropped before the native window.
    let mut windows: HashMap<WindowId, (WebView, Window)> = HashMap::new();
    let mut private_id = None;
    for (title, identifier, position) in [
        ("Search profile A", Some(PROFILE_A), (50.0, 80.0)),
        ("Search profile B", Some(PROFILE_B), (110.0, 140.0)),
        ("Search private", None, (170.0, 200.0)),
    ] {
        let (webview, window) = create_window(&event_loop, title, identifier, position)?;
        if identifier.is_none() {
            private_id = Some(window.id());
        }
        windows.insert(window.id(), (webview, window));
    }
    let mut private_reopened = false;

    event_loop.run(move |event, event_loop_target, control_flow| {
        *control_flow = ControlFlow::Wait;
        if let Event::WindowEvent {
            window_id,
            event: WindowEvent::CloseRequested,
            ..
        } = event
        {
            let closing_private = private_id == Some(window_id);
            windows.remove(&window_id);
            if closing_private {
                private_id = None;
                if !private_reopened && !windows.is_empty() {
                    private_reopened = true;
                    match create_window(
                        event_loop_target,
                        "Search private (reopened)",
                        None,
                        (170.0, 200.0),
                    ) {
                        Ok((webview, window)) => {
                            private_id = Some(window.id());
                            windows.insert(window.id(), (webview, window));
                        }
                        Err(error) => eprintln!("Could not reopen private window: {error}"),
                    }
                }
            }
            if windows.is_empty() {
                *control_flow = ControlFlow::Exit;
            }
        }
    });
}
