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
    use objc2_foundation::{NSError, NSString, NSURL, NSUUID};
    use objc2_web_kit::{WKWebViewConfiguration, WKWebsiteDataStore};
    use std::time::{Duration, Instant};
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

    let args: Vec<_> = std::env::args().skip(1).collect();
    assert_eq!(args.len(), 2, "expected loopback URL and profile UUID");
    let address = url::Url::parse(&args[0]).expect("valid fixture URL");
    assert_eq!(address.scheme(), "http");
    assert_eq!(address.host_str(), Some("localhost"));
    let fixture = tempfile::tempdir().expect("create extension fixture directory");
    std::fs::write(
        fixture.path().join("manifest.json"),
        r#"{
        "manifest_version":3,"name":"Search fixture","version":"1.0",
        "content_scripts":[{"matches":["http://localhost/*"],"js":["marker.js"]}]
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
    let completion = block2::RcBlock::new(
        move |extension: *mut AnyObject, _error: *mut NSError| {
            let loaded = if extension.is_null() {
                false
            } else {
                // The block is called on the main actor and WebKit owns the extension argument.
                unsafe {
                    let allocated: Allocated<AnyObject> = msg_send![context_class, alloc];
                    let context: Retained<AnyObject> =
                        msg_send![allocated, initForExtension: &*extension];
                    let mut error: *mut NSError = std::ptr::null_mut();
                    let loaded: Bool = msg_send![&*controller_for_callback, loadExtensionContext: &*context, error: &mut error];
                    if loaded.as_bool() {
                        // WKWebExtensionContextPermissionStatusGrantedExplicitly.
                        let _: () = msg_send![&*context, setPermissionStatus: 3isize, forURL: &*fixture_url];
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
                views[index].evaluate_script_with_callback(
                    match index {
                        0 => "document.documentElement.dataset.searchExtensionProbe === 'ran' && typeof window.ipc === 'undefined'",
                        1 => "document.documentElement.dataset.searchExtensionProbe === undefined && typeof window.ipc?.postMessage === 'function'",
                        _ => "document.documentElement.dataset.searchExtensionProbe === undefined && typeof window.ipc === 'undefined'",
                    },
                    move |value| { let _ = responses.send_event(Message::Checked(index, value == "true")); },
                ).expect("read fixture marker");
            }
            Event::UserEvent(Message::Checked(index, true)) => {
                seen[index] = true;
                if seen.iter().all(|checked| *checked) {
                    println!("PASS: extension ran in ordinary Wry content only; trusted chrome and temporary content stayed isolated");
                    *flow = ControlFlow::ExitWithCode(0);
                }
            }
            Event::UserEvent(Message::Checked(index, marked)) => {
                eprintln!("FAIL: unexpected extension marker in view {index}: {marked}");
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
