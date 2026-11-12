// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

#[cfg(not(target_os = "linux"))]
compile_error!("This research probe runs only on Linux.");

use std::{
    error::Error,
    ffi::{CStr, c_char, c_int, c_void},
    fs::File,
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    ptr,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
        mpsc,
    },
    thread,
    time::{Duration, Instant},
};

use glib::object::ObjectType;
use tao::{
    event::{Event, WindowEvent},
    event_loop::{ControlFlow, EventLoopBuilder},
    platform::unix::WindowExtUnix,
    window::WindowBuilder,
};
use wry::{PermissionResponse, WebViewBuilder, WebViewBuilderExtUnix, WebViewExtUnix};

const TIMEOUT: Duration = Duration::from_secs(25);

// The Rust webkit2gtk binding used by Wry 0.57.1 omits this signal and its
// boxed query type. Keep this small C ABI bridge in the research probe only.
#[link(name = "gobject-2.0")]
unsafe extern "C" {
    fn g_signal_connect_data(
        instance: *mut c_void,
        detailed_signal: *const c_char,
        handler: Option<unsafe extern "C" fn()>,
        data: *mut c_void,
        destroy_data: Option<unsafe extern "C" fn(*mut c_void, *mut c_void)>,
        flags: u32,
    ) -> u64;
}

#[link(name = "webkit2gtk-4.1")]
unsafe extern "C" {
    fn webkit_permission_state_query_get_name(query: *mut c_void) -> *const c_char;
    fn webkit_permission_state_query_get_security_origin(query: *mut c_void) -> *mut c_void;
    fn webkit_security_origin_get_protocol(origin: *mut c_void) -> *const c_char;
    fn webkit_security_origin_get_host(origin: *mut c_void) -> *const c_char;
    fn webkit_security_origin_get_port(origin: *mut c_void) -> u16;
}

#[derive(Debug, Clone)]
struct Query {
    name: String,
    origin: String,
}

unsafe fn copy_string(value: *const c_char) -> String {
    if value.is_null() {
        return "(null)".into();
    }
    // SAFETY: WebKit returns a NUL-terminated string valid for this callback.
    unsafe { CStr::from_ptr(value) }
        .to_string_lossy()
        .into_owned()
}

unsafe extern "C" fn query_callback(
    _webview: *mut c_void,
    query: *mut c_void,
    data: *mut c_void,
) -> c_int {
    if query.is_null() || data.is_null() {
        return 0;
    }
    // SAFETY: WebKit owns query during the signal. The boxed Arc lives until
    // g_signal_connect_data calls destroy_query_data on signal disconnection.
    let name = unsafe { copy_string(webkit_permission_state_query_get_name(query)) };
    let origin = unsafe { webkit_permission_state_query_get_security_origin(query) };
    let formatted = if origin.is_null() {
        "(null)".into()
    } else {
        let scheme = unsafe { copy_string(webkit_security_origin_get_protocol(origin)) };
        let host = unsafe { copy_string(webkit_security_origin_get_host(origin)) };
        let port = unsafe { webkit_security_origin_get_port(origin) };
        format!("{scheme}://{host}:{port}")
    };
    let records = unsafe { &*(data.cast::<Arc<Mutex<Vec<Query>>>>()) };
    if let Ok(mut entries) = records.lock() {
        entries.push(Query {
            name,
            origin: formatted,
        });
    }
    // FALSE lets WebKit continue with its default PROMPT state. The separate
    // Wry permission handler below always denies actual device access.
    0
}

unsafe extern "C" fn destroy_query_data(data: *mut c_void, _closure: *mut c_void) {
    if !data.is_null() {
        // SAFETY: This is the single Box allocated for g_signal_connect_data.
        drop(unsafe { Box::from_raw(data.cast::<Arc<Mutex<Vec<Query>>>>()) });
    }
}

fn html(context: &str, cross_port: u16, token: &str) -> String {
    let iframes = if context == "top" {
        format!(
            "<iframe id='same' src='/same' allow='camera; microphone'></iframe>\
             <iframe id='cross' src='http://127.0.0.1:{cross_port}/' allow='camera; microphone'></iframe>"
        )
    } else {
        String::new()
    };
    format!(
        "<!doctype html><meta charset='utf-8'><title>Search origin probe</title>\
         <h1>Search origin probe: {context}</h1>{iframes}<script>\
         const label = '{context}';\
         async function report(kind, value) {{\
           await fetch('/report/{token}/' + label + '-' + kind + '/' + encodeURIComponent(value + '|' + location.origin));\
         }}\
         async function run() {{\
           for (let attempt = 1; attempt <= 2; attempt++) {{\
             try {{ const result = await navigator.permissions.query({{name:'camera'}});\
               await report('query' + attempt, result.state); }}\
             catch (e) {{ await report('query' + attempt, e.name); }}\
           }}\
           try {{ const stream = await navigator.mediaDevices.getUserMedia({{video:true}});\
             stream.getTracks().forEach(track => track.stop()); await report('gum', 'resolved'); }}\
           catch (e) {{ await report('gum', e.name); }}\
         }}\
         run();</script>"
    )
}

fn serve(
    mut stream: TcpStream,
    page: &str,
    same_page: Option<&str>,
    sender: &mpsc::Sender<String>,
    cross_port: u16,
    server_port: u16,
    token: &str,
) {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(3)));
    let mut bytes = Vec::with_capacity(512);
    while bytes.len() < 8192 {
        let mut byte = [0_u8; 1];
        let Ok(length) = stream.read(&mut byte) else {
            return;
        };
        if length == 0 {
            return;
        }
        bytes.push(byte[0]);
        if bytes.ends_with(b"\r\n\r\n") {
            break;
        }
    }
    let request = String::from_utf8_lossy(&bytes);
    let host_valid = request
        .lines()
        .any(|line| line.eq_ignore_ascii_case(&format!("Host: 127.0.0.1:{server_port}")));
    let origin_valid = request.lines().all(|line| {
        !line.to_ascii_lowercase().starts_with("origin:")
            || line.eq_ignore_ascii_case(&format!("Origin: http://127.0.0.1:{server_port}"))
    });
    let path = request
        .lines()
        .next()
        .and_then(|line| line.strip_prefix("GET "))
        .and_then(|line| line.split_once(' '))
        .map(|(path, _)| path)
        .unwrap_or("");
    let (status, body) = if !host_valid || !origin_valid {
        ("403 Forbidden", "invalid local origin")
    } else if path == "/" {
        ("200 OK", page)
    } else if path == "/same" {
        same_page.map_or(("404 Not Found", "not found"), |page| ("200 OK", page))
    } else if let Some(report) = path.strip_prefix(&format!("/report/{token}/")) {
        if report.len() <= 100 && report.is_ascii() {
            let _ = sender.send(report.to_owned());
            ("200 OK", "recorded")
        } else {
            ("400 Bad Request", "invalid report")
        }
    } else {
        ("404 Not Found", "not found")
    };
    let response = format!(
        "HTTP/1.1 {status}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nCache-Control: no-store\r\nX-Content-Type-Options: nosniff\r\nPermissions-Policy: camera=(self \"http://127.0.0.1:{cross_port}\"), microphone=(self \"http://127.0.0.1:{cross_port}\")\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    let _ = stream.write_all(response.as_bytes());
}

fn start_server(
    listener: TcpListener,
    page: String,
    same_page: Option<String>,
    sender: mpsc::Sender<String>,
    cross_port: u16,
    server_port: u16,
    token: String,
) {
    thread::spawn(move || {
        for stream in listener.incoming() {
            match stream {
                Ok(stream) => serve(
                    stream,
                    &page,
                    same_page.as_deref(),
                    &sender,
                    cross_port,
                    server_port,
                    &token,
                ),
                Err(error) => eprintln!("LOCAL_SERVER_ERROR {error}"),
            }
        }
    });
}

fn main() -> Result<(), Box<dyn Error>> {
    if std::env::args().skip(1).collect::<Vec<_>>() != ["--request-camera"] {
        return Err("run only with explicit --request-camera".into());
    }
    let top = TcpListener::bind("127.0.0.1:0")?;
    let frame = TcpListener::bind("127.0.0.1:0")?;
    let top_port = top.local_addr()?.port();
    let frame_port = frame.local_addr()?.port();
    let mut random = [0_u8; 16];
    File::open("/dev/urandom")?.read_exact(&mut random)?;
    let token = random
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    let (sender, receiver) = mpsc::channel();
    start_server(
        top,
        html("top", frame_port, &token),
        Some(html("same", frame_port, &token)),
        sender.clone(),
        frame_port,
        top_port,
        token.clone(),
    );
    start_server(
        frame,
        html("cross", top_port, &token),
        None,
        sender,
        frame_port,
        frame_port,
        token,
    );
    eprintln!("TOP_ORIGIN http://127.0.0.1:{top_port}");
    eprintln!("FRAME_ORIGIN http://127.0.0.1:{frame_port}");

    let event_loop = EventLoopBuilder::<()>::with_user_event().build();
    let window = WindowBuilder::new()
        .with_title("Search: Linux origin permission probe")
        .build(&event_loop)?;
    let box_widget = window.default_vbox().ok_or("Tao GTK box missing")?;
    let records = Arc::new(Mutex::new(Vec::<Query>::new()));
    let denied_count = Arc::new(AtomicUsize::new(0));
    let counter = denied_count.clone();
    let view = WebViewBuilder::new()
        .with_url("about:blank")
        .with_incognito(true)
        .with_permission_handler(move |kind| {
            counter.fetch_add(1, Ordering::SeqCst);
            eprintln!("WRY_PERMISSION_REQUEST {kind:?} DENY");
            PermissionResponse::Deny
        })
        .build_gtk(box_widget)?;
    let native_view = view.webview();
    let callback: unsafe extern "C" fn() = unsafe {
        std::mem::transmute::<
            unsafe extern "C" fn(*mut c_void, *mut c_void, *mut c_void) -> c_int,
            unsafe extern "C" fn(),
        >(query_callback)
    };
    let data = Box::into_raw(Box::new(records.clone())).cast::<c_void>();
    let signal_id = unsafe {
        g_signal_connect_data(
            native_view.as_ptr().cast::<c_void>(),
            c"query-permission-state".as_ptr(),
            Some(callback),
            data,
            Some(destroy_query_data),
            0,
        )
    };
    if signal_id == 0 {
        unsafe { destroy_query_data(data, ptr::null_mut()) };
        return Err("WebKitGTK query-permission-state signal not available".into());
    }
    view.load_url(&format!("http://127.0.0.1:{top_port}/"))?;
    let started = Instant::now();
    let mut reports = Vec::new();
    event_loop.run(move |event, _, control_flow| {
        let _keep_alive = (&window, &view, &native_view);
        *control_flow = ControlFlow::WaitUntil(Instant::now() + Duration::from_millis(100));
        while let Ok(report) = receiver.try_recv() {
            eprintln!("PAGE_REPORT {report}");
            reports.push(report);
        }
        if reports.len() >= 9 || started.elapsed() >= TIMEOUT {
            let mut top_seen = false;
            let mut frame_seen = false;
            if let Ok(entries) = records.lock() {
                for entry in entries.iter() {
                    eprintln!("QUERY name={} origin={}", entry.name, entry.origin);
                }
                top_seen = entries
                    .iter()
                    .any(|entry| entry.name == "camera" && entry.origin == format!("http://127.0.0.1:{top_port}"));
                frame_seen = entries
                    .iter()
                    .any(|entry| entry.name == "camera" && entry.origin == format!("http://127.0.0.1:{frame_port}"));
                eprintln!(
                    "DIAGNOSTIC top_origin_seen={top_seen} cross_origin_seen={frame_seen} reports={} denied_callbacks={}",
                    reports.len(), denied_count.load(Ordering::SeqCst)
                );
            }
            let unsafe_grant = reports.iter().any(|value| value.contains("-gum/resolved%7C"));
            let top_report_origin = format!("http%3A%2F%2F127.0.0.1%3A{top_port}");
            let cross_report_origin = format!("http%3A%2F%2F127.0.0.1%3A{frame_port}");
            let complete = [
                ("top-query1/", &top_report_origin),
                ("top-query2/", &top_report_origin),
                ("top-gum/", &top_report_origin),
                ("same-query1/", &top_report_origin),
                ("same-query2/", &top_report_origin),
                ("same-gum/", &top_report_origin),
                ("cross-query1/", &cross_report_origin),
                ("cross-query2/", &cross_report_origin),
                ("cross-gum/", &cross_report_origin),
            ]
            .iter()
            .all(|(prefix, origin)| {
                reports
                    .iter()
                    .any(|value| value.starts_with(*prefix) && value.ends_with(origin.as_str()))
            });
            *control_flow = ControlFlow::ExitWithCode(if unsafe_grant {
                2
            } else if complete {
                0
            } else {
                1
            });
        }
        if let Event::WindowEvent {
            event: WindowEvent::CloseRequested,
            ..
        } = event
        {
            *control_flow = ControlFlow::ExitWithCode(3);
        }
    });
}
