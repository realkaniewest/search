// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

#[cfg(not(target_os = "macos"))]
compile_error!("This permission prototype is limited to macOS.");

use std::{
    error::Error,
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
        mpsc,
    },
    thread,
    time::{Duration, Instant},
};

use tao::{
    event::{Event, WindowEvent},
    event_loop::{ControlFlow, EventLoop},
    window::WindowBuilder,
};
use wry::{PermissionKind, PermissionResponse, WebViewBuilder};

const PAGE: &str = include_str!("probe.html");

fn parse_report(report: &str) -> (&str, &str) {
    report.split_once("&detail=").unwrap_or((report, ""))
}

fn passed(status: &str, handler_calls: usize) -> bool {
    status == "NotAllowedError" && handler_calls > 0
}

fn first_request_line(stream: &mut TcpStream) -> std::io::Result<Vec<u8>> {
    stream.set_read_timeout(Some(Duration::from_secs(5)))?;
    let mut line = Vec::with_capacity(128);
    while line.len() < 2048 {
        let mut byte = [0_u8; 1];
        if stream.read(&mut byte)? == 0 {
            break;
        }
        line.push(byte[0]);
        if line.ends_with(b"\r\n") {
            break;
        }
    }
    Ok(line)
}

fn serve(mut stream: TcpStream, sender: &mpsc::Sender<String>) -> std::io::Result<()> {
    let request = first_request_line(&mut stream)?;
    let (status, body) = if request == b"GET / HTTP/1.1\r\n" {
        ("200 OK", PAGE)
    } else if let Some(value) = request
        .strip_prefix(b"GET /report?status=")
        .and_then(|tail| tail.strip_suffix(b" HTTP/1.1\r\n"))
    {
        let value = String::from_utf8_lossy(value).into_owned();
        let _ = sender.send(value);
        ("200 OK", "Recorded")
    } else {
        ("404 Not Found", "Not found")
    };
    let response = format!(
        "HTTP/1.1 {status}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nCache-Control: no-store\r\nX-Content-Type-Options: nosniff\r\nContent-Security-Policy: default-src 'none'; script-src 'unsafe-inline'; connect-src 'self'\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    stream.write_all(response.as_bytes())
}

fn main() -> Result<(), Box<dyn Error>> {
    let mut args = std::env::args().skip(1);
    if args.next().as_deref() != Some("--request-camera") || args.next().is_some() {
        return Err("camera probe requires explicit --request-camera flag".into());
    }

    let listener = TcpListener::bind("127.0.0.1:0")?;
    let address = listener.local_addr()?;
    let (sender, receiver) = mpsc::channel();
    thread::spawn(move || {
        for stream in listener.incoming() {
            match stream {
                Ok(stream) => {
                    if let Err(error) = serve(stream, &sender) {
                        eprintln!("Local test page request failed: {error}");
                    }
                }
                Err(error) => eprintln!("Local listener failed: {error}"),
            }
        }
    });

    let event_loop = EventLoop::new();
    let window = WindowBuilder::new()
        .with_title("Search: Wry permission deny test")
        .build(&event_loop)?;
    let seen_camera = Arc::new(AtomicUsize::new(0));
    let counter = seen_camera.clone();
    let _webview = WebViewBuilder::new()
        .with_url(format!("http://{address}/"))
        .with_incognito(true)
        .with_permission_handler(move |kind| {
            if kind == PermissionKind::Camera {
                counter.fetch_add(1, Ordering::SeqCst);
            }
            PermissionResponse::Deny
        })
        .build(&window)?;
    let started = Instant::now();

    event_loop.run(move |event, _, control_flow| {
        *control_flow = ControlFlow::WaitUntil(Instant::now() + Duration::from_millis(100));
        if let Ok(report) = receiver.try_recv() {
            let (status, detail) = parse_report(&report);
            let count = seen_camera.load(Ordering::SeqCst);
            println!("Page result: {status}; detail: {detail}; camera handler calls: {count}");
            std::process::exit(if passed(status, count) { 0 } else { 1 });
        }
        if started.elapsed() > Duration::from_secs(20) {
            eprintln!(
                "Timed out; camera handler calls: {}",
                seen_camera.load(Ordering::SeqCst)
            );
            std::process::exit(1);
        }
        if let Event::WindowEvent {
            event: WindowEvent::CloseRequested,
            ..
        } = event
        {
            eprintln!("Window closed before a result was received");
            std::process::exit(1);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn report_requires_webkit_callback() {
        let (status, detail) = parse_report("NotAllowedError&detail=secure%3Dtrue");
        assert_eq!(status, "NotAllowedError");
        assert_eq!(detail, "secure%3Dtrue");
        assert!(!passed(status, 0));
        assert!(passed(status, 1));
        assert!(!passed("granted", 1));
    }
}
