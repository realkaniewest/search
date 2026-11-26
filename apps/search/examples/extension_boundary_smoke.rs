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
    use objc2_app_kit::{NSAlert, NSAlertFirstButtonReturn, NSAlertSecondButtonReturn, NSWindow};
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
        platform::macos::WindowExtMacOS,
        window::WindowBuilder,
    };
    use wry::{PageLoadEvent, WebView, WebViewBuilder, WebViewBuilderExtMacos, WebViewExtMacOS};

    enum Message {
        ReviewReady(Option<String>),
        DriveReview,
        ReviewDecision(bool),
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
        RevokedHost,
        RevokedAlternatePort,
    }

    let args: Vec<_> = std::env::args().skip(1).collect();
    assert_eq!(
        args.len(),
        4,
        "expected loopback URL, profile UUID, alternate port and staged fixture"
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
        let resource_url = NSURL::fileURLWithPath_isDirectory(&NSString::from_str(&args[3]), true);
        (controller, config, fixture_url, resource_url)
    };
    let loading_class = class(c"WKWebExtension");
    let context_class = class(c"WKWebExtensionContext");
    let completion_proxy = proxy.clone();
    let cancel_review = std::env::var_os("SEARCH_EXTENSION_CANCEL_REVIEW").is_some();
    let reviewed_extension = Rc::new(RefCell::new(None));
    let reviewed_for_callback = reviewed_extension.clone();
    let extension_context = RefCell::new(None);
    let completion = block2::RcBlock::new(
        move |extension: *mut AnyObject, _error: *mut NSError| {
            let review = if extension.is_null() {
                None
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
                    let optional_permissions: Retained<NSSet<NSString>> =
                        msg_send![&*extension, optionalPermissions];
                    let optional_patterns: Retained<NSSet<AnyObject>> =
                        msg_send![&*extension, optionalPermissionMatchPatterns];
                    let read_hosts = |patterns: &NSSet<AnyObject>| {
                        let mut hosts: Vec<_> = patterns
                            .iter()
                            .map(|pattern| {
                                let pattern: Retained<NSString> = msg_send![&*pattern, string];
                                pattern.to_string()
                            })
                            .collect();
                        hosts.sort_unstable();
                        hosts
                    };
                    if name.as_deref().map(ToString::to_string).as_deref() != Some("Search fixture")
                        || version.as_deref().map(ToString::to_string).as_deref() != Some("1.0")
                        || permissions
                            .iter()
                            .map(|value| value.to_string())
                            .collect::<Vec<_>>()
                            != ["storage"]
                        || read_hosts(&patterns) != ["http://127.0.0.1/*", "http://localhost/*"]
                        || optional_permissions
                            .iter()
                            .map(|value| value.to_string())
                            .collect::<Vec<_>>()
                            != ["tabs"]
                        || read_hosts(&optional_patterns) != ["https://example.test/*"]
                    {
                        eprintln!("FAIL: unexpected extension metadata for review");
                        let _ = completion_proxy.send_event(Message::ReviewReady(None));
                        return;
                    }
                    *reviewed_for_callback.borrow_mut() = Retained::retain(extension);
                    reviewed_for_callback.borrow().as_ref().map(|_| {
                        format!(
                            "{} {}\nОбязательные возможности: {}\nСайты: {}\nДополнительные возможности: {}\nДополнительные сайты: {}",
                            name.as_deref().unwrap(), version.as_deref().unwrap(),
                            permissions.iter().map(|value| value.to_string()).collect::<Vec<_>>().join(", "),
                            read_hosts(&patterns).join(", "),
                            optional_permissions.iter().map(|value| value.to_string()).collect::<Vec<_>>().join(", "),
                            read_hosts(&optional_patterns).join(", "),
                        )
                    })
                }
            };
            let _ = completion_proxy.send_event(Message::ReviewReady(review));
        },
    );
    let mut start_loading = Some((resource_url, completion));
    let mut review_alert = None;
    let mut views: Vec<WebView> = Vec::new();
    let mut seen = [false; 3];
    let mut step = Step::Initial;
    let deadline = Instant::now() + Duration::from_secs(20);
    loop_.run(move |event, _, flow| {
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
            Event::UserEvent(Message::ReviewReady(Some(summary))) => {
                let alert = NSAlert::new(mtm);
                alert.setMessageText(&NSString::from_str("Установить расширение?"));
                alert.setInformativeText(&NSString::from_str(&summary));
                alert.addButtonWithTitle(&NSString::from_str("Отмена"));
                alert.addButtonWithTitle(&NSString::from_str("Установить"));
                let reply = proxy.clone();
                let completion = block2::RcBlock::new(move |response| {
                    let _ = reply.send_event(Message::ReviewDecision(response == NSAlertSecondButtonReturn));
                });
                // Tao owns this NSWindow for the lifetime of the event loop.
                let parent = unsafe { &*window.ns_window().cast::<NSWindow>() };
                alert.beginSheetModalForWindow_completionHandler(parent, Some(&completion));
                review_alert = Some(alert);
                let _ = proxy.send_event(Message::DriveReview);
            }
            Event::UserEvent(Message::DriveReview) => {
                let contexts: Retained<NSSet<AnyObject>> = unsafe { msg_send![&*controller, extensionContexts] };
                let parent = unsafe { &*window.ns_window().cast::<NSWindow>() };
                let Some(sheet) = parent.attachedSheet().filter(|_| contexts.is_empty()) else {
                    eprintln!("FAIL: expected a review sheet with no loaded extension");
                    *flow = ControlFlow::ExitWithCode(1);
                    return;
                };
                let response = if cancel_review { NSAlertFirstButtonReturn } else { NSAlertSecondButtonReturn };
                parent.endSheet_returnCode(&sheet, response);
            }
            Event::UserEvent(Message::ReviewDecision(approved)) => {
                review_alert.take();
                let extension = reviewed_extension.borrow_mut().take().expect("reviewed package");
                if !approved {
                    let contexts: Retained<NSSet<AnyObject>> = unsafe { msg_send![&*controller, extensionContexts] };
                    if !cancel_review || !contexts.is_empty() {
                        eprintln!("FAIL: unexpected extension review cancellation");
                        *flow = ControlFlow::ExitWithCode(1);
                        return;
                    }
                    println!("PASS: cancelling extension review left no loaded context");
                    *flow = ControlFlow::ExitWithCode(0);
                    return;
                }
                if cancel_review {
                    eprintln!("FAIL: extension review approved despite cancellation");
                    *flow = ControlFlow::ExitWithCode(1);
                    return;
                }
                let context: Retained<AnyObject> = unsafe {
                    let allocated: Allocated<AnyObject> = msg_send![context_class, alloc];
                    msg_send![allocated, initForExtension: &*extension]
                };
                let mut error: *mut NSError = std::ptr::null_mut();
                let loaded: Bool = unsafe {
                    msg_send![&*controller, loadExtensionContext: &*context, error: &mut error]
                };
                if !loaded.as_bool() {
                    eprintln!("FAIL: could not load the confirmed extension");
                    *flow = ControlFlow::ExitWithCode(1);
                    return;
                }
                *extension_context.borrow_mut() = Some(context);
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
                    (0, Step::AlternatePort | Step::RevokedAlternatePort) => alternate_port.origin().ascii_serialization(),
                    (1, _) => "null".into(),
                    _ => address.origin().ascii_serialization(),
                };
                let marker = match index {
                    0 if matches!(step, Step::Initial | Step::DeniedHost | Step::Disabled | Step::RevokedHost | Step::RevokedAlternatePort) =>
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
                        let context = extension_context.borrow();
                        let context = context.as_ref().expect("loaded extension context");
                        // WKWebExtensionContextPermissionStatusUnknown removes the explicit grant.
                        unsafe {
                            let _: () = msg_send![&**context, setPermissionStatus: 0isize, forURL: &*fixture_url];
                        }
                        step = Step::RevokedHost;
                        &address
                    }
                    Step::RevokedHost => {
                        step = Step::RevokedAlternatePort;
                        &alternate_port
                    }
                    Step::RevokedAlternatePort => {
                        println!("PASS: Wry verified review metadata, default denial, chrome and profile isolation, host and port scope, disable, re-enable and revoke");
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
            Event::UserEvent(Message::ReviewReady(None)) => {
                eprintln!("FAIL: WebKit could not prepare the extension fixture for review");
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
