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
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
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

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Single,
    Parallel,
}

fn main() {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    let mode = match arguments.as_slice() {
        [] => Mode::Single,
        [flag] if flag == "--parallel" => Mode::Parallel,
        _ => {
            eprintln!("Usage: search-wry-download [--parallel]");
            std::process::exit(2);
        }
    };
    let expected = if mode == Mode::Parallel { 2 } else { 1 };
    let listener = TcpListener::bind("127.0.0.1:0").expect("failed to bind loopback server");
    let address = listener.local_addr().expect("missing loopback address");
    let url = format!("http://{address}/download");
    thread::spawn(move || {
        for stream in listener.incoming().take(8) {
            let Ok(stream) = stream else { break };
            thread::spawn(move || serve_download(stream, mode));
        }
    });

    let destination_dir = tempfile::Builder::new()
        .prefix("search-wry-download-")
        .tempdir()
        .expect("failed to create private probe directory");
    let destinations: Vec<PathBuf> = (0..expected)
        .map(|index| destination_dir.path().join(format!("probe-{index}.txt")))
        .collect();

    let event_loop = EventLoopBuilder::<ProbeEvent>::with_user_event().build();
    let window = WindowBuilder::new()
        .with_title("Search Wry download probe")
        .build(&event_loop)
        .expect("failed to create window");
    let completed_proxy = event_loop.create_proxy();
    let timeout_proxy = event_loop.create_proxy();
    let allowed_url = url.clone();
    let completed_url = url.clone();
    let chosen_paths = destinations.clone();
    let started_count = Arc::new(AtomicUsize::new(0));
    let started_for_handler = started_count.clone();
    let second_click = if mode == Mode::Parallel {
        "setTimeout(() => document.getElementById('download').click(), 550);"
    } else {
        ""
    };
    let html = format!(
        "<!doctype html><html><body><h1>Search download probe</h1><a id='download' href='{url}'>Download</a><script>setTimeout(() => document.getElementById('download').click(), 500);{second_click}</script></body></html>"
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
            let index = started_for_handler.fetch_add(1, Ordering::SeqCst);
            let Some(chosen_path) = chosen_paths.get(index) else {
                eprintln!("DENY extra download");
                return false;
            };
            *path = chosen_path.clone();
            eprintln!("DESTINATION index={index} path={}", path.display());
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

    let mut completion_count = 0;
    let mut callbacks_succeeded = true;
    event_loop.run(move |event, _, control_flow| {
        let _keep_private_directory_alive = &destination_dir;
        *control_flow = ControlFlow::Wait;
        match event {
            Event::UserEvent(ProbeEvent::Completed(success)) => {
                completion_count += 1;
                callbacks_succeeded &= success;
                eprintln!("COMPLETION count={completion_count}/{expected}");
                if completion_count == expected {
                    let all_started = started_count.load(Ordering::SeqCst) == expected;
                    let all_files_match = destinations.iter().all(report_file);
                    *control_flow = ControlFlow::ExitWithCode(
                        if callbacks_succeeded && all_started && all_files_match {
                            0
                        } else {
                            1
                        },
                    );
                }
            }
            Event::UserEvent(ProbeEvent::TimedOut) => {
                eprintln!("TIMEOUT: no completion callback");
                for path in &destinations {
                    report_file(path);
                }
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

fn serve_download(mut stream: std::net::TcpStream, mode: Mode) {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
    let _ = stream.set_write_timeout(Some(Duration::from_secs(2)));
    let mut request = [0; 2048];
    let Ok(size) = stream.read(&mut request) else {
        return;
    };
    if !request[..size].starts_with(b"GET /download HTTP/1.") {
        let _ = stream
            .write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
        return;
    }
    let header = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: application/octet-stream\r\nContent-Disposition: attachment; filename=probe.txt\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        PAYLOAD.len()
    );
    let _ = stream.write_all(header.as_bytes());
    if mode == Mode::Parallel {
        thread::sleep(Duration::from_millis(800));
    }
    let _ = stream.write_all(PAYLOAD);
}

fn report_file(path: &PathBuf) -> bool {
    match fs::read(path) {
        Ok(bytes) => {
            let matches = bytes == PAYLOAD;
            eprintln!("FILE path={} matches={matches}", path.display());
            matches
        }
        Err(error) => {
            eprintln!("FILE missing: {error}");
            false
        }
    }
}
