// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

#[cfg(target_os = "macos")]
fn main() {
    use objc2::{
        AllocAnyThread, MainThreadMarker, msg_send,
        rc::{Allocated, Retained},
        runtime::{AnyClass, AnyObject, Bool},
    };
    use objc2_foundation::{NSError, NSSet, NSString, NSURL, NSUUID};
    use objc2_web_kit::{WKWebViewConfiguration, WKWebsiteDataStore};
    use std::{
        cell::RefCell,
        rc::Rc,
        time::{Duration, Instant},
    };
    use tao::{
        event::Event,
        event_loop::{ControlFlow, EventLoopBuilder},
        window::WindowBuilder,
    };
    use wry::{PageLoadEvent, WebView, WebViewBuilder, WebViewBuilderExtMacos, WebViewExtMacOS};

    enum Message {
        ExtensionReady(bool),
        Ready(usize),
        Checked(usize, bool),
        UnexpectedChromeIpc,
    }

    #[derive(Clone, Copy, Debug)]
    enum Step {
        Initial,
        GrantedHost,
        DeniedHost,
        AlternatePort,
        Disabled,
        Reenabled,
    }

    let args: Vec<_> = std::env::args().skip(1).collect();
    assert_eq!(
        args.len(),
        3,
        "expected loopback URL, profile UUID and alternate port"
    );
    let address = url::Url::parse(&args[0]).expect("valid fixture URL");
    assert_eq!(address.scheme(), "http");
    assert_eq!(address.host_str(), Some("localhost"));
    let mut denied_host = address.clone();
    denied_host
        .set_host(Some("127.0.0.1"))
        .expect("valid fixture host");
    let mut alternate_port = address.clone();
    alternate_port
        .set_port(Some(args[2].parse().expect("valid alternate port")))
        .expect("valid HTTP port");
    assert_ne!(alternate_port.port(), address.port());
    let fixture = tempfile::tempdir().expect("create extension fixture directory");
    std::fs::write(
        fixture.path().join("manifest.json"),
        r#"{
        "manifest_version":3,"name":"Search fixture","version":"1.0",
        "permissions":["storage"],
        "content_scripts":[{"matches":["http://localhost/*","http://127.0.0.1/*"],"js":["marker.js"]}]
    }"#,
    )
    .expect("write extension manifest");
    std::fs::write(
        fixture.path().join("marker.js"),
        "
        if (typeof window.ipc === 'undefined')
          document.documentElement.dataset.searchExtensionProbe = 'ran';
        else window.ipc.postMessage('extension-boundary-fixture');
    ",
    )
    .expect("write extension script");
    let mut fixture = Some(fixture);
    let mtm = MainThreadMarker::new().expect("main thread");
    let loop_ = EventLoopBuilder::<Message>::with_user_event().build();
    let window = WindowBuilder::new()
        .with_title("Search · Extension boundary smoke")
        .build(&loop_)
        .unwrap();
    let proxy = loop_.create_proxy();
    let class = |name| AnyClass::get(name).expect("requires WebKit extensions on macOS 15.4+");

    // The selectors below are the public Objective-C API in the macOS 15.4+ SDK.
    let (controller, config, fixture_url, resource_url) = unsafe {
        let uuid = NSUUID::initWithUUIDString(NSUUID::alloc(), &NSString::from_str(&args[1]))
            .expect("fixture UUID");
        let store = WKWebsiteDataStore::dataStoreForIdentifier(&uuid, mtm);
        let settings: Retained<AnyObject> = msg_send![
            class(c"WKWebExtensionControllerConfiguration"),
            nonPersistentConfiguration
        ];
        let _: () = msg_send![&*settings, setDefaultWebsiteDataStore: &*store];
        let allocated: Allocated<AnyObject> = msg_send![class(c"WKWebExtensionController"), alloc];
        let controller: Retained<AnyObject> =
            msg_send![allocated, initWithConfiguration: &*settings];
        let config = WKWebViewConfiguration::new(mtm);
        config.setWebsiteDataStore(&store);
        let _: () = msg_send![&*config, setWebExtensionController: &*controller];
        let fixture_url = NSURL::URLWithString(&NSString::from_str(address.as_str())).unwrap();
        let resource_url = NSURL::fileURLWithPath_isDirectory(
            &NSString::from_str(fixture.as_ref().unwrap().path().to_str().unwrap()),
            true,
        );
        (controller, config, fixture_url, resource_url)
    };
    let loading_class = class(c"WKWebExtension");
    let context_class = class(c"WKWebExtensionContext");
    let completion_proxy = proxy.clone();
    let controller_for_callback = controller.clone();
    let extension_context = Rc::new(RefCell::new(None));
    let context_for_callback = extension_context.clone();
    let completion = block2::RcBlock::new(
        move |extension: *mut AnyObject, _error: *mut NSError| {
            let loaded = if extension.is_null() {
                false
            } else {
                // The block is called on the main actor and WebKit owns the extension argument.
                unsafe {
                    let name: Option<Retained<NSString>> = msg_send![&*extension, displayName];
                    let version: Option<Retained<NSString>> =
                        msg_send![&*extension, displayVersion];
                    let permissions: Retained<NSSet<NSString>> =
                        msg_send![&*extension, requestedPermissions];
                    let patterns: Retained<NSSet<AnyObject>> =
                        msg_send![&*extension, allRequestedMatchPatterns];
                    let mut hosts: Vec<_> = patterns
                        .iter()
                        .map(|pattern| {
                            let pattern: Retained<NSString> = msg_send![&*pattern, string];
                            pattern.to_string()
                        })
                        .collect();
                    hosts.sort_unstable();
                    if name.as_deref().map(ToString::to_string).as_deref() != Some("Search fixture")
                        || version.as_deref().map(ToString::to_string).as_deref() != Some("1.0")
                        || permissions
                            .iter()
                            .map(|value| value.to_string())
                            .collect::<Vec<_>>()
                            != ["storage"]
                        || hosts != ["http://127.0.0.1/*", "http://localhost/*"]
                    {
                        eprintln!("FAIL: unexpected extension metadata for review");
                        let _ = completion_proxy.send_event(Message::ExtensionReady(false));
                        return;
                    }
                    let allocated: Allocated<AnyObject> = msg_send![context_class, alloc];
                    let context: Retained<AnyObject> =
                        msg_send![allocated, initForExtension: &*extension];
                    let mut error: *mut NSError = std::ptr::null_mut();
                    let loaded: Bool = msg_send![&*controller_for_callback, loadExtensionContext: &*context, error: &mut error];
                    if loaded.as_bool() {
                        *context_for_callback.borrow_mut() = Some(context);
                    }
                    loaded.as_bool()
                }
            };
            let _ = completion_proxy.send_event(Message::ExtensionReady(loaded));
        },
    );
    let mut start_loading = Some((resource_url, completion));
    let mut views: Vec<WebView> = Vec::new();
    let mut seen = [false; 3];
    let mut step = Step::Initial;
    let deadline = Instant::now() + Duration::from_secs(20);
    loop_.run(move |event, _, flow| {
        if matches!(event, Event::LoopDestroyed) {
            fixture.take();
            return;
        }
        *flow = ControlFlow::WaitUntil(deadline);
        if Instant::now() >= deadline {
            eprintln!("FAIL: extension boundary test timed out");
            *flow = ControlFlow::ExitWithCode(1);
            return;
        }
        match event {
            Event::MainEventsCleared => {
                if let Some((resource, completion)) = start_loading.take() {
                    // The public asynchronous factory copies the completion block.
                    let _: () = unsafe {
                        msg_send![loading_class, extensionWithResourceBaseURL: &*resource, completionHandler: &*completion]
                    };
                }
            }
            Event::UserEvent(Message::ExtensionReady(true)) => {
                for index in 0..3 {
                    let ready = proxy.clone();
                    let builder = WebViewBuilder::new().with_on_page_load_handler(move |event, _| {
                        if matches!(event, PageLoadEvent::Finished) {
                            let _ = ready.send_event(Message::Ready(index));
                        }
                    });
                    let builder = match index {
                        0 => builder.with_webview_configuration(config.clone()).with_url(address.as_str()),
                        1 => {
                            let chrome_proxy = proxy.clone();
                            builder.with_incognito(true)
                                .with_html(include_str!("../src/chrome.html"))
                                .with_ipc_handler(move |_| {
                                    let _ = chrome_proxy.send_event(Message::UnexpectedChromeIpc);
                                })
                        },
                        _ => builder.with_incognito(true).with_url(address.as_str()),
                    };
                    let view = builder.build_as_child(&window).expect("build Wry fixture view");
                    // Inspect the configuration copied by WKWebView rather than the builder input.
                    let (attached, persistent) = unsafe {
                        let effective: Retained<WKWebViewConfiguration> = msg_send![&*view.webview(), configuration];
                        let attached: Option<Retained<AnyObject>> = msg_send![&*effective, webExtensionController];
                        (attached, effective.websiteDataStore().isPersistent())
                    };
                    let correct_controller = match (index, attached) {
                        (0, Some(attached)) => std::ptr::eq(&*attached, &*controller),
                        (1 | 2, None) => true,
                        _ => false,
                    };
                    if !correct_controller || persistent != (index == 0) {
                        eprintln!("FAIL: controller or profile mismatch in WebView {index}");
                        *flow = ControlFlow::ExitWithCode(1);
                        return;
                    }
                    views.push(view);
                }
            }
            Event::UserEvent(Message::Ready(index)) => {
                let responses = proxy.clone();
                let expected_origin = match (index, step) {
                    (0, Step::DeniedHost) => denied_host.origin().ascii_serialization(),
                    (0, Step::AlternatePort) => alternate_port.origin().ascii_serialization(),
                    (1, _) => "null".into(),
                    _ => address.origin().ascii_serialization(),
                };
                let marker = match index {
                    0 if matches!(step, Step::Initial | Step::DeniedHost | Step::Disabled) =>
                        "document.documentElement.dataset.searchExtensionProbe === undefined && typeof window.ipc === 'undefined'",
                    0 => "document.documentElement.dataset.searchExtensionProbe === 'ran' && typeof window.ipc === 'undefined'",
                    1 => "document.documentElement.dataset.searchExtensionProbe === undefined && typeof window.ipc?.postMessage === 'function'",
                    _ => "document.documentElement.dataset.searchExtensionProbe === undefined && typeof window.ipc === 'undefined'",
                };
                let script = format!("location.origin === {} && ({marker})",
                                     serde_json::to_string(&expected_origin).unwrap());
                views[index].evaluate_script_with_callback(
                    &script,
                    move |value| { let _ = responses.send_event(Message::Checked(index, value == "true")); },
                ).expect("read fixture marker");
            }
            Event::UserEvent(Message::Checked(index, true)) => {
                if matches!(step, Step::Initial) {
                    seen[index] = true;
                    if !seen.iter().all(|checked| *checked) {
                        return;
                    }
                } else if index != 0 {
                    return;
                }
                let url = match step {
                    Step::Initial => {
                        let context = extension_context.borrow();
                        let context = context.as_ref().expect("loaded extension context");
                        // WKWebExtensionContextPermissionStatusGrantedExplicitly.
                        unsafe {
                            let _: () = msg_send![&**context, setPermissionStatus: 3isize, forURL: &*fixture_url];
                        }
                        step = Step::GrantedHost;
                        &address
                    }
                    Step::GrantedHost => {
                        step = Step::DeniedHost;
                        &denied_host
                    }
                    Step::DeniedHost => {
                        step = Step::AlternatePort;
                        &alternate_port
                    }
                    Step::AlternatePort | Step::Disabled => {
                        let context = extension_context.borrow();
                        let context = context.as_ref().expect("loaded extension context");
                        let disabling = matches!(step, Step::AlternatePort);
                        // load/unload are the public selectors for the same retained context.
                        let changed: Bool = unsafe {
                            let mut error: *mut NSError = std::ptr::null_mut();
                            if disabling {
                                msg_send![&*controller, unloadExtensionContext: &**context, error: &mut error]
                            } else {
                                msg_send![&*controller, loadExtensionContext: &**context, error: &mut error]
                            }
                        };
                        if !changed.as_bool() {
                            eprintln!("FAIL: could not change activation in {step:?}");
                            *flow = ControlFlow::ExitWithCode(1);
                            return;
                        }
                        step = if disabling { Step::Disabled } else { Step::Reenabled };
                        &address
                    }
                    Step::Reenabled => {
                        println!("PASS: Wry verified review metadata, default denial, chrome and profile isolation, host and port scope, disable and re-enable");
                        *flow = ControlFlow::ExitWithCode(0);
                        return;
                    }
                };
                views[0].load_url(url.as_str()).expect("navigate ordinary fixture");
            }
            Event::UserEvent(Message::Checked(index, false)) => {
                eprintln!("FAIL: unexpected extension state in view {index} during {step:?}");
                *flow = ControlFlow::ExitWithCode(1);
            }
            Event::UserEvent(Message::UnexpectedChromeIpc) => {
                eprintln!("FAIL: unexpected message reached the trusted panel handler");
                *flow = ControlFlow::ExitWithCode(1);
            }
            Event::UserEvent(Message::ExtensionReady(false)) => {
                eprintln!("FAIL: WebKit could not load the extension fixture");
                *flow = ControlFlow::ExitWithCode(1);
            }
            _ => {}
        }
    });
}

#[cfg(not(target_os = "macos"))]
fn main() {
    eprintln!("This smoke test requires macOS");
    std::process::exit(2);
}
