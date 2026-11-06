// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

#[cfg(not(target_os = "macos"))]
compile_error!("This research probe is limited to macOS.");

use std::{
    fs,
    io::{Read, Write},
    net::TcpListener,
    path::PathBuf,
    thread,
    time::Duration,
};
use tao::{
    event::{Event, WindowEvent},
    event_loop::{ControlFlow, EventLoopBuilder},
    window::WindowBuilder,
};
use wry::{PermissionKind, PermissionResponse, WebViewBuilder};

const PAYLOAD: &[u8] = b"Search Wry download probe\n";

#[derive(Debug)]
enum ProbeEvent {
    Completed(bool),
    TimedOut,
}

fn main() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("failed to bind loopback server");
    let address = listener.local_addr().expect("missing loopback address");
    let url = format!("http://{address}/download");
    thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { break };
            let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
            let _ = stream.set_write_timeout(Some(Duration::from_secs(2)));
            let mut request = [0; 2048];
            let Ok(size) = stream.read(&mut request) else {
                continue;
            };
            if !request[..size].starts_with(b"GET /download HTTP/1.") {
                let _ = stream.write_all(
                    b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                );
                continue;
            }
            let header = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/octet-stream\r\nContent-Disposition: attachment; filename=probe.txt\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                PAYLOAD.len()
            );
            let _ = stream.write_all(header.as_bytes());
            let _ = stream.write_all(PAYLOAD);
        }
    });

    let destination_dir = tempfile::Builder::new()
        .prefix("search-wry-download-")
        .tempdir()
        .expect("failed to create private probe directory");
    let destination = destination_dir.path().join("probe.txt");

    let event_loop = EventLoopBuilder::<ProbeEvent>::with_user_event().build();
    let window = WindowBuilder::new()
        .with_title("Search Wry download probe")
        .build(&event_loop)
        .expect("failed to create window");
    let completed_proxy = event_loop.create_proxy();
    let timeout_proxy = event_loop.create_proxy();
    let allowed_url = url.clone();
    let completed_url = url.clone();
    let chosen_path = destination.clone();
    let html = format!(
        "<!doctype html><html><body><h1>Search download probe</h1><a id='download' href='{url}'>Download</a><script>setTimeout(() => document.getElementById('download').click(), 500);</script></body></html>"
    );
    let _webview = WebViewBuilder::new()
        .with_html(html)
        .with_incognito(true)
        .with_permission_handler(|_: PermissionKind| PermissionResponse::Deny)
        .with_download_started_handler(move |request_url, path| {
            eprintln!("START url={request_url} suggested={}", path.display());
            if request_url != allowed_url {
                eprintln!("DENY unexpected download URL");
                return false;
            }
            *path = chosen_path.clone();
            eprintln!("DESTINATION {}", path.display());
            true
        })
        .with_download_completed_handler(move |request_url, callback_path, success| {
            eprintln!(
                "COMPLETE url={request_url} callback_path={callback_path:?} success={success}"
            );
            if request_url == completed_url {
                let _ = completed_proxy.send_event(ProbeEvent::Completed(success));
            } else {
                eprintln!("IGNORED unexpected completion URL");
            }
        })
        .build(&window)
        .expect("failed to build webview");

    thread::spawn(move || {
        thread::sleep(Duration::from_secs(15));
        let _ = timeout_proxy.send_event(ProbeEvent::TimedOut);
    });

    event_loop.run(move |event, _, control_flow| {
        let _keep_private_directory_alive = &destination_dir;
        *control_flow = ControlFlow::Wait;
        match event {
            Event::UserEvent(ProbeEvent::Completed(success)) => {
                *control_flow = ControlFlow::ExitWithCode(if report_file(success, &destination) {
                    0
                } else {
                    1
                });
            }
            Event::UserEvent(ProbeEvent::TimedOut) => {
                eprintln!("TIMEOUT: no completion callback");
                report_file(false, &destination);
                *control_flow = ControlFlow::ExitWithCode(1);
            }
            Event::WindowEvent {
                event: WindowEvent::CloseRequested,
                ..
            } => {
                eprintln!("CLOSED before completion");
                *control_flow = ControlFlow::ExitWithCode(1);
            }
            _ => {}
        }
    });
}

fn report_file(success: bool, path: &PathBuf) -> bool {
    match fs::read(path) {
        Ok(bytes) => {
            let matches = bytes == PAYLOAD;
            eprintln!(
                "FILE path={} matches={matches} callback_success={success}",
                path.display()
            );
            success && matches
        }
        Err(error) => {
            eprintln!("FILE missing: {error}");
            false
        }
    }
}
