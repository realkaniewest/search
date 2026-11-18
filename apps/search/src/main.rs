// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

use serde::{Deserialize, Serialize};
use std::rc::Rc;
use std::{
    env, process,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
    },
    thread,
    time::Duration,
};
use tao::{
    dpi::{LogicalPosition, LogicalSize},
    event::{Event, WindowEvent},
    event_loop::{ControlFlow, EventLoopBuilder},
    window::{Window, WindowBuilder},
};
use url::Url;
use wry::{
    NewWindowResponse, PageLoadEvent, PermissionKind, PermissionResponse, Rect, WebView,
    WebViewBuilder,
};

#[cfg(target_os = "linux")]
use gtk::prelude::*;
#[cfg(target_os = "linux")]
use tao::platform::unix::WindowExtUnix;
#[cfg(target_os = "linux")]
use wry::WebViewBuilderExtUnix;
#[cfg(target_os = "windows")]
use wry::WebViewBuilderExtWindows;

const CHROME_HEIGHT: f64 = 50.0;
const LOCAL_ORIGIN_PORT: u16 = 43127;
const MAX_TABS: usize = 12;

#[derive(Clone, Copy, Eq, PartialEq)]
enum Mode {
    Normal,
    Smoke,
    SecuritySmoke,
}

#[derive(Default)]
struct SecurityCounts {
    fixture_mode: bool,
    blocked_navigation: AtomicUsize,
    blocked_popup: AtomicUsize,
    blocked_download: AtomicUsize,
    denied_permission: AtomicUsize,
}

impl SecurityCounts {
    fn snapshot(&self) -> (usize, usize, usize, usize) {
        (
            self.blocked_navigation.load(Ordering::SeqCst),
            self.blocked_popup.load(Ordering::SeqCst),
            self.blocked_download.load(Ordering::SeqCst),
            self.denied_permission.load(Ordering::SeqCst),
        )
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase", deny_unknown_fields)]
enum Command {
    New,
    Select {
        index: usize,
    },
    Close {
        index: usize,
    },
    Open {
        value: String,
    },
    #[serde(rename = "cancel-address")]
    CancelAddress,
    Back,
    Forward,
    Reload,
}

#[derive(Debug)]
enum AppEvent {
    ChromeReady,
    StartLink { id: u64, url: String },
    ContentStarted { id: u64, url: String },
    ContentLoaded { id: u64, url: String },
    ContentTitle { id: u64, title: String },
    SecurityCheck,
    SecurityProgress,
    TimedOut,
    Command(Command),
}

#[derive(Clone, Copy, Debug)]
enum Page {
    Start,
    Rust,
    Wikipedia,
    Excalidraw,
    LocalFixture,
}

impl Page {
    fn label(self) -> &'static str {
        match self {
            Self::Start => "Новая вкладка",
            Self::Rust => "Rust",
            Self::Wikipedia => "Wikipedia",
            Self::Excalidraw => "Excalidraw",
            Self::LocalFixture => "Локальная страница",
        }
    }

    fn url(self) -> &'static str {
        match self {
            Self::Start => "search:start",
            Self::Rust => "https://rust-lang.org/",
            Self::Wikipedia => "https://www.wikipedia.org/",
            Self::Excalidraw => "https://excalidraw.com/",
            Self::LocalFixture => "http://127.0.0.1:43127/",
        }
    }
}

fn validated_url(raw: &str) -> Option<(Page, Url)> {
    // Accept one URL spelling for validation, navigation and the address bar.
    if raw.chars().any(|ch| ch.is_control() || ch == '\\') {
        return None;
    }
    let url = Url::parse(raw).ok()?;
    if !url.username().is_empty() || url.password().is_some() {
        return None;
    }
    let page = match (url.scheme(), url.host_str(), url.port_or_known_default()) {
        ("https", Some("rust-lang.org" | "www.rust-lang.org"), Some(443)) => Some(Page::Rust),
        (
            "https",
            Some("www.wikipedia.org" | "en.wikipedia.org" | "ru.wikipedia.org"),
            Some(443),
        ) => Some(Page::Wikipedia),
        ("https", Some("excalidraw.com"), Some(443)) => Some(Page::Excalidraw),
        ("http", Some("127.0.0.1"), Some(LOCAL_ORIGIN_PORT))
            if url.port() == Some(LOCAL_ORIGIN_PORT) =>
        {
            Some(Page::LocalFixture)
        }
        _ => None,
    }?;
    Some((page, url))
}

fn allowed_url(raw: &str) -> Option<Page> {
    validated_url(raw).map(|(page, _)| page)
}

fn page_from_input(raw: &str) -> Option<(Page, String)> {
    let input = raw.trim();
    let page = match input.to_ascii_lowercase().as_str() {
        "rust" => Page::Rust,
        "wikipedia" => Page::Wikipedia,
        "excalidraw" => Page::Excalidraw,
        "local" => Page::LocalFixture,
        "search:start" => Page::Start,
        _ => return validated_url(input).map(|(page, url)| (page, url.into())),
    };
    Some((page, page.url().to_owned()))
}

fn parse_command(raw: &str) -> Option<Command> {
    if raw.len() > 2048 {
        return None;
    }
    let command = serde_json::from_str::<Command>(raw).ok()?;
    if matches!(&command, Command::Open { value } if value.len() > 1024) {
        return None;
    }
    Some(command)
}

fn deny_permission(_: PermissionKind) -> PermissionResponse {
    PermissionResponse::Deny
}

fn bounds(window: &Window) -> (Rect, Rect) {
    let size = window.inner_size().to_logical::<f64>(window.scale_factor());
    let width = size.width.max(1.0);
    let chrome = Rect {
        position: LogicalPosition::new(0.0, 0.0).into(),
        size: LogicalSize::new(width, CHROME_HEIGHT).into(),
    };
    let content = Rect {
        position: LogicalPosition::new(0.0, CHROME_HEIGHT).into(),
        size: LogicalSize::new(width, (size.height - CHROME_HEIGHT).max(1.0)).into(),
    };
    (chrome, content)
}

#[cfg(target_os = "linux")]
fn build_view(
    builder: WebViewBuilder<'_>,
    _window: &Window,
    fixed: &gtk::Fixed,
) -> wry::Result<WebView> {
    builder.build_gtk(fixed)
}

#[cfg(not(target_os = "linux"))]
fn build_view(builder: WebViewBuilder<'_>, window: &Window) -> wry::Result<WebView> {
    builder.build_as_child(window)
}

struct Tab {
    id: u64,
    page: Page,
    display_url: String,
    title: String,
    loading: bool,
    has_start_entry: bool,
    pending_start_return: bool,
    start_forward_url: Option<String>,
    view: Rc<WebView>,
    local_navigation: Arc<AtomicBool>,
}

fn loaded_page(url: &str, pending_start_return: bool) -> Option<(Page, String)> {
    if url == "about:blank" && pending_start_return {
        return Some((Page::Start, Page::Start.url().to_owned()));
    }
    validated_url(url).map(|(page, url)| (page, url.into()))
}

fn safe_title(raw: &str) -> Option<String> {
    let title: String = raw
        .chars()
        .take(160)
        .filter(|ch| {
            !ch.is_control()
                && !matches!(ch, '\u{061c}' | '\u{200e}' | '\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}')
        })
        .take(80)
        .collect();
    let title = title.trim();
    (!title.is_empty()).then(|| title.to_owned())
}

#[derive(Serialize)]
struct TabInfo<'a> {
    id: u64,
    label: &'a str,
    url: &'a str,
    loading: bool,
}

#[derive(Serialize)]
struct ChromeState<'a> {
    active: usize,
    tabs: Vec<TabInfo<'a>>,
    notice: Option<&'static str>,
    attempted_address: Option<&'a str>,
}

fn sync_chrome(
    chrome: &WebView,
    tabs: &[Tab],
    active: usize,
    notice: Option<&'static str>,
    attempted_address: Option<&str>,
) {
    let state = ChromeState {
        active,
        tabs: tabs
            .iter()
            .map(|tab| TabInfo {
                id: tab.id,
                label: &tab.title,
                url: &tab.display_url,
                loading: tab.loading,
            })
            .collect(),
        notice,
        attempted_address,
    };
    if let Ok(json) = serde_json::to_string(&state) {
        let _ = chrome.evaluate_script(&format!("window.renderSearch({json})"));
    }
}

fn permits_content_navigation(target: &str, local_navigation: &AtomicBool) -> bool {
    if target == "about:blank" {
        return true;
    }
    match allowed_url(target) {
        Some(Page::LocalFixture) => local_navigation.load(Ordering::SeqCst),
        Some(_) => {
            local_navigation.store(false, Ordering::SeqCst);
            true
        }
        None => false,
    }
}

fn create_content(
    window: &Window,
    #[cfg(target_os = "linux")] fixed: &gtk::Fixed,
    id: u64,
    proxy: tao::event_loop::EventLoopProxy<AppEvent>,
    counts: Arc<SecurityCounts>,
    initial: (Page, &str, Option<Arc<AtomicU64>>),
    rect: Rect,
) -> wry::Result<(WebView, Arc<AtomicBool>)> {
    let (page, initial_url, shared_start) = initial;
    let local_navigation = Arc::new(AtomicBool::new(matches!(page, Page::LocalFixture)));
    let policy = local_navigation.clone();
    let permission_counts = counts.clone();
    let download_counts = counts.clone();
    let popup_counts = counts.clone();
    let navigation_counts = counts;
    let fixture_proxy = proxy.clone();
    let download_proxy = proxy.clone();
    let second_fixture =
        navigation_counts.fixture_mode && id == 2 && matches!(page, Page::LocalFixture);
    let builder = WebViewBuilder::new()
        .with_bounds(rect)
        .with_incognito(true)
        .with_permission_handler(move |kind| {
            permission_counts
                .denied_permission
                .fetch_add(1, Ordering::SeqCst);
            deny_permission(kind)
        })
        .with_download_started_handler(move |_, _| {
            download_counts
                .blocked_download
                .fetch_add(1, Ordering::SeqCst);
            if second_fixture {
                let _ = download_proxy.send_event(AppEvent::SecurityProgress);
            }
            false
        })
        .with_new_window_req_handler(move |_, _| {
            popup_counts.blocked_popup.fetch_add(1, Ordering::SeqCst);
            NewWindowResponse::Deny
        })
        .with_navigation_handler(move |target| {
            if let Some(selection) = &shared_start {
                if [Page::Rust, Page::Wikipedia, Page::Excalidraw]
                    .iter()
                    .any(|site| site.url() == target)
                {
                    let id = selection.load(Ordering::Relaxed);
                    let _ = fixture_proxy.send_event(AppEvent::StartLink { id, url: target });
                    return false;
                }
                return target == "about:blank";
            }
            let allowed = permits_content_navigation(&target, &policy);
            if env::var_os("SEARCH_DEBUG_NAV").is_some() {
                eprintln!("content navigation allowed={allowed}");
            }
            if !allowed {
                navigation_counts
                    .blocked_navigation
                    .fetch_add(1, Ordering::SeqCst);
                // In the two-tab smoke run, the second fixture attempts this
                // navigation after its report reaches the local server.
                if second_fixture && target == "https://blocked.invalid/" {
                    let _ = fixture_proxy.send_event(AppEvent::SecurityProgress);
                }
            }
            allowed
        })
        .with_on_page_load_handler({
            let proxy = proxy.clone();
            move |event, url| {
                let app_event = match event {
                    PageLoadEvent::Started => AppEvent::ContentStarted { id, url },
                    PageLoadEvent::Finished => AppEvent::ContentLoaded { id, url },
                };
                let _ = proxy.send_event(app_event);
            }
        })
        .with_document_title_changed_handler(move |title| {
            let _ = proxy.send_event(AppEvent::ContentTitle { id, title });
        });
    let builder = if matches!(page, Page::Start) {
        builder.with_html(include_str!("start.html"))
    } else {
        builder.with_url(initial_url)
    };
    #[cfg(target_os = "windows")]
    let builder = builder
        .with_additional_browser_args("")
        .with_profile_name(format!("SearchTemporaryTab{id}"));
    #[cfg(target_os = "linux")]
    return build_view(builder, window, fixed).map(|view| (view, local_navigation));
    #[cfg(not(target_os = "linux"))]
    build_view(builder, window).map(|view| (view, local_navigation))
}

fn send_security_checkpoint(proxy: &tao::event_loop::EventLoopProxy<AppEvent>, round: u8) {
    eprintln!("SECURITY: dispatch checkpoint {round}");
    if proxy.send_event(AppEvent::SecurityCheck).is_err() {
        eprintln!("SECURITY: event loop closed before checkpoint {round}");
    }
}

fn main() {
    let mode = match env::args().skip(1).collect::<Vec<_>>().as_slice() {
        [] => Mode::Normal,
        [flag] if flag == "--smoke" => Mode::Smoke,
        [flag] if flag == "--security-smoke" => Mode::SecuritySmoke,
        _ => {
            eprintln!("Usage: search-browser [--smoke|--security-smoke]");
            process::exit(2);
        }
    };
    let event_loop = EventLoopBuilder::<AppEvent>::with_user_event().build();
    let window_builder = WindowBuilder::new()
        .with_title("Search")
        .with_inner_size(LogicalSize::new(1180.0, 760.0))
        .with_min_inner_size(LogicalSize::new(680.0, 420.0));
    #[cfg(target_os = "macos")]
    let window_builder = {
        use tao::platform::macos::WindowBuilderExtMacOS;
        window_builder
            .with_transparent(true)
            .with_titlebar_transparent(true)
            .with_title_hidden(true)
            .with_fullsize_content_view(true)
    };
    let window = window_builder
        .build(&event_loop)
        .expect("Search window creation failed");
    #[cfg(target_os = "linux")]
    let fixed = {
        let vbox = window.default_vbox().expect("Tao GTK container missing");
        let fixed = gtk::Fixed::new();
        vbox.pack_start(&fixed, true, true, 0);
        fixed.show();
        fixed
    };
    let (chrome_rect, content_rect) = bounds(&window);
    let proxy = event_loop.create_proxy();
    if mode == Mode::SecuritySmoke {
        for (round, seconds) in [(1, 12), (2, 24)] {
            let timer_proxy = proxy.clone();
            thread::spawn(move || {
                thread::sleep(Duration::from_secs(seconds));
                send_security_checkpoint(&timer_proxy, round);
            });
        }
    }
    if mode != Mode::Normal {
        let timer_proxy = proxy.clone();
        thread::spawn(move || {
            let seconds = if mode == Mode::SecuritySmoke { 34 } else { 25 };
            thread::sleep(Duration::from_secs(seconds));
            let _ = timer_proxy.send_event(AppEvent::TimedOut);
        });
    }
    let security_counts = Arc::new(SecurityCounts {
        fixture_mode: mode == Mode::SecuritySmoke,
        ..SecurityCounts::default()
    });
    let ready_proxy = proxy.clone();
    let chrome_builder = WebViewBuilder::new()
        .with_bounds(chrome_rect)
        .with_html(include_str!("chrome.html"))
        .with_initialization_script(if cfg!(target_os = "macos") {
            "document.documentElement?.classList.add('mac-toolbar'); document.addEventListener('DOMContentLoaded', () => document.documentElement.classList.add('mac-toolbar'));"
        } else { "" })
        .with_incognito(true)
        .with_permission_handler(deny_permission)
        .with_download_started_handler(|_, _| false)
        .with_new_window_req_handler(|_, _| NewWindowResponse::Deny)
        .with_navigation_handler(|target| target == "about:blank")
        .with_ipc_handler(move |request| {
            if let Some(command) = parse_command(request.body()) {
                let _ = proxy.send_event(AppEvent::Command(command));
            }
        })
        .with_on_page_load_handler(move |event, _| {
            if matches!(event, PageLoadEvent::Finished) {
                let _ = ready_proxy.send_event(AppEvent::ChromeReady);
            }
        });
    #[cfg(target_os = "macos")]
    let chrome_builder = chrome_builder.with_transparent(true);
    #[cfg(target_os = "windows")]
    let chrome_builder = chrome_builder.with_additional_browser_args("");
    #[cfg(target_os = "linux")]
    let chrome = build_view(chrome_builder, &window, &fixed).expect("Search chrome failed");
    #[cfg(not(target_os = "linux"))]
    let chrome = build_view(chrome_builder, &window).expect("Search chrome failed");

    let initial_page = if mode == Mode::SecuritySmoke {
        Page::LocalFixture
    } else {
        Page::Start
    };
    let start_selection = (mode != Mode::SecuritySmoke).then(|| Arc::new(AtomicU64::new(1)));
    #[cfg(target_os = "linux")]
    let (first, local_navigation) = create_content(
        &window,
        &fixed,
        1,
        event_loop.create_proxy(),
        security_counts.clone(),
        (initial_page, initial_page.url(), start_selection.clone()),
        content_rect,
    )
    .expect("Search start page failed");
    #[cfg(not(target_os = "linux"))]
    let (first, local_navigation) = create_content(
        &window,
        1,
        event_loop.create_proxy(),
        security_counts.clone(),
        (initial_page, initial_page.url(), start_selection.clone()),
        content_rect,
    )
    .expect("Search start page failed");
    let first = Rc::new(first);
    let shared_start =
        (mode != Mode::SecuritySmoke).then(|| (first.clone(), local_navigation.clone()));
    let mut tabs = vec![Tab {
        id: 1,
        page: initial_page,
        display_url: initial_page.url().to_owned(),
        title: initial_page.label().to_owned(),
        loading: false,
        has_start_entry: matches!(initial_page, Page::Start),
        pending_start_return: false,
        start_forward_url: None,
        view: first,
        local_navigation,
    }];
    let mut active = 0usize;
    let mut next_id = 2u64;
    let mut chrome_ready = false;
    let mut content_ready = false;
    let mut notice = None;
    let mut attempted_address: Option<String> = None;
    let content_proxy = event_loop.create_proxy();

    event_loop.run(move |event, _, control_flow| {
        let _keep_window_alive = &window;
        *control_flow = ControlFlow::Wait;
        let command_from_start = match event {
            Event::UserEvent(AppEvent::StartLink { id, url }) => {
                if id == tabs[active].id && shared_start.as_ref().is_some_and(|(view, _)| Rc::ptr_eq(view, &tabs[active].view)) {
                    Event::UserEvent(AppEvent::Command(Command::Open { value: url }))
                } else {
                    return;
                }
            }
            event => event,
        };
        match command_from_start {
            Event::UserEvent(AppEvent::ChromeReady) => {
                if mode == Mode::Smoke { println!("SMOKE: chrome loaded"); }
                chrome_ready = true;
                sync_chrome(&chrome, &tabs, active, notice, attempted_address.as_deref());
                if mode == Mode::Smoke && content_ready {
                    println!("PASS: Search chrome and start content loaded");
                    *control_flow = ControlFlow::ExitWithCode(0);
                }
            }
            Event::UserEvent(AppEvent::ContentStarted { id, url }) => {
                if let Some(tab) = tabs.iter_mut().find(|tab| tab.id == id)
                    && (allowed_url(&url).is_some() || url == "about:blank")
                {
                    tab.loading = true;
                    if let Some((page, display_url)) = loaded_page(&url, tab.pending_start_return) {
                        tab.page = page;
                        tab.display_url = display_url;
                        tab.title = page.label().to_owned();
                        if !matches!(page, Page::Start) {
                            tab.start_forward_url = None;
                        } else {
                            tab.local_navigation.store(false, Ordering::SeqCst);
                        }
                    }
                    if chrome_ready {
                        sync_chrome(&chrome, &tabs, active, notice, attempted_address.as_deref());
                    }
                }
            }
            Event::UserEvent(AppEvent::ContentLoaded { id, url }) => {
                if mode == Mode::Smoke { println!("SMOKE: content {id} loaded at {url}"); }
                if id == 1 {
                    content_ready = true;
                }
                if let Some(tab) = tabs.iter_mut().find(|tab| tab.id == id) {
                    tab.loading = false;
                    if let Some((page, display_url)) = loaded_page(&url, tab.pending_start_return) {
                        tab.page = page;
                        tab.display_url = display_url;
                        tab.pending_start_return = false;
                        if matches!(page, Page::Start) {
                            tab.title = page.label().to_owned();
                        }
                    }
                    if chrome_ready {
                        sync_chrome(&chrome, &tabs, active, notice, attempted_address.as_deref());
                    }
                }
                if mode == Mode::Smoke && chrome_ready && content_ready {
                    println!("PASS: Search chrome and start content loaded");
                    *control_flow = ControlFlow::ExitWithCode(0);
                }
            }
            Event::UserEvent(AppEvent::ContentTitle { id, title }) => {
                if let Some(tab) = tabs.iter_mut().find(|tab| tab.id == id)
                    && !matches!(tab.page, Page::Start)
                    && let Some(title) = safe_title(&title)
                {
                    tab.title = title;
                    if chrome_ready {
                        sync_chrome(&chrome, &tabs, active, notice, attempted_address.as_deref());
                    }
                }
            }
            Event::UserEvent(signal @ (AppEvent::SecurityCheck | AppEvent::SecurityProgress))
                if mode == Mode::SecuritySmoke =>
            {
                let checkpoint = matches!(signal, AppEvent::SecurityCheck);
                let (nav, popup, download, permission) = security_counts.snapshot();
                let title_verified = tabs
                    .first()
                    .is_some_and(|tab| tab.title == "Search security fixture ready");
                println!("SECURITY: fixture_loaded={content_ready} title_verified={title_verified} blocked_navigation={nav} blocked_popup={popup} blocked_download={download} denied_permission={permission}");
                // WebKitGTK can deny window.open before Wry's popup callback.
                // The fixture separately checks that no popup was returned.
                if content_ready && chrome_ready && title_verified && nav > 0 && download > 0 {
                    if checkpoint && tabs.len() == 1 && active == 0 {
                        let _ = content_proxy.send_event(AppEvent::Command(Command::New));
                    } else if nav >= 2
                        && download >= 2
                        && tabs.get(1).is_some_and(|tab| tab.title == tabs[0].title)
                    {
                        println!("PASS: Search security boundary checks");
                        *control_flow = ControlFlow::ExitWithCode(0);
                    }
                } else if checkpoint {
                    eprintln!("FAIL: Search security boundary checks incomplete");
                    *control_flow = ControlFlow::ExitWithCode(1);
                }
            }
            Event::UserEvent(AppEvent::TimedOut) => {
                eprintln!("FAIL: Search first window did not load before the test deadline");
                *control_flow = ControlFlow::ExitWithCode(1);
            }
            Event::UserEvent(AppEvent::Command(command)) => {
                notice = None;
                attempted_address = None;
                match command {
                    Command::New if tabs.len() < MAX_TABS => {
                        let (_, rect) = bounds(&window);
                        let page = if mode == Mode::SecuritySmoke {
                            Page::LocalFixture
                        } else {
                            Page::Start
                        };
                        let new_view = if let Some((view, policy)) = &shared_start {
                            Ok((view.clone(), policy.clone()))
                        } else {
                        #[cfg(target_os = "linux")]
                        let new_view = create_content(
                            &window,
                            &fixed,
                            next_id,
                            content_proxy.clone(),
                            security_counts.clone(),
                            (page, page.url(), None),
                            rect,
                        );
                        #[cfg(not(target_os = "linux"))]
                        let new_view = create_content(
                            &window,
                            next_id,
                            content_proxy.clone(),
                            security_counts.clone(),
                            (page, page.url(), None),
                            rect,
                        );
                        new_view.map(|(view, policy)| (Rc::new(view), policy))
                        };
                        if mode == Mode::SecuritySmoke {
                            eprintln!("SECURITY: second WebView created={}", new_view.is_ok());
                        }
                        if let Ok((view, local_navigation)) = new_view {
                            if !Rc::ptr_eq(&tabs[active].view, &view) {
                                let _ = tabs[active].view.set_visible(false);
                                let _ = view.set_visible(true);
                            }
                            tabs.push(Tab {
                                id: next_id,
                                page,
                                display_url: page.url().to_owned(),
                                title: page.label().to_owned(),
                                loading: false,
                                has_start_entry: matches!(page, Page::Start),
                                pending_start_return: false,
                                start_forward_url: None,
                                view,
                                local_navigation,
                            });
                            active = tabs.len() - 1;
                            next_id += 1;
                            if mode == Mode::SecuritySmoke {
                                eprintln!("SECURITY: second WebView active");
                            }
                        }
                    }
                    Command::New => {
                        notice = Some("Открыто 12 вкладок. Закрой одну, чтобы открыть новую.");
                    }
                    Command::Select { index } if index < tabs.len() && index != active => {
                        if !Rc::ptr_eq(&tabs[active].view, &tabs[index].view) {
                            let _ = tabs[active].view.set_visible(false);
                            let _ = tabs[index].view.set_visible(true);
                        }
                        active = index;
                    }
                    Command::Close { index } if index < tabs.len() => {
                        if tabs.len() > 1 {
                            let removed = tabs.remove(index);
                            if index < active {
                                active -= 1;
                            } else if index == active {
                                active = active.min(tabs.len() - 1);
                                if !Rc::ptr_eq(&removed.view, &tabs[active].view) {
                                    let _ = removed.view.set_visible(false);
                                    let _ = tabs[active].view.set_visible(true);
                                }
                            }
                        }
                    }
                    Command::Open { value } => {
                        if let Some((page, url)) = page_from_input(&value) {
                            let is_shared = shared_start.as_ref().is_some_and(|(view, _)| Rc::ptr_eq(view, &tabs[active].view));
                            let result = if is_shared && matches!(page, Page::Start) {
                                Ok(())
                            } else if is_shared || matches!(page, Page::LocalFixture) {
                                let (_, rect) = bounds(&window);
                                #[cfg(target_os = "linux")]
                                let local_view = create_content(
                                    &window, &fixed, next_id, content_proxy.clone(),
                                    security_counts.clone(), (page, &url, None), rect,
                                );
                                #[cfg(not(target_os = "linux"))]
                                let local_view = create_content(
                                    &window, next_id, content_proxy.clone(),
                                    security_counts.clone(), (page, &url, None), rect,
                                );
                                local_view.map(|(view, local_navigation)| {
                                    let tab = &mut tabs[active];
                                    tab.id = next_id;
                                    next_id += 1;
                                    let _ = tab.view.set_visible(false);
                                    tab.view = Rc::new(view);
                                    tab.local_navigation = local_navigation;
                                    tab.has_start_entry = is_shared && !matches!(page, Page::LocalFixture);
                                })
                            } else {
                                let tab = &tabs[active];
                                tab.local_navigation.store(false, Ordering::SeqCst);
                                if matches!(page, Page::Start) {
                                    tab.view.load_html(include_str!("start.html"))
                                } else {
                                    tab.view.load_url(&url)
                                }
                            };
                            if result.is_ok() {
                                tabs[active].page = page;
                                tabs[active].display_url = url;
                                tabs[active].title = page.label().to_owned();
                                tabs[active].loading = !matches!(page, Page::Start);
                                if matches!(page, Page::Start) {
                                    tabs[active].has_start_entry = true;
                                }
                                tabs[active].pending_start_return = false;
                                tabs[active].start_forward_url = None;
                            } else {
                                notice = Some("Не удалось открыть страницу. Попробуй ещё раз.");
                                attempted_address = Some(value);
                            }
                        } else {
                            notice = Some("Пока доступны Rust, Wikipedia, Excalidraw и local.");
                            attempted_address = Some(value);
                        }
                    }
                    Command::CancelAddress => {}
                    Command::Back => {
                        let tab = &mut tabs[active];
                        if tab.start_forward_url.is_none() {
                            match tab.view.can_go_back() {
                                Ok(false)
                                    if tab.has_start_entry
                                        && !matches!(tab.page, Page::Start)
                                        && matches!(tab.view.can_go_forward(), Ok(false)) =>
                                {
                                    // Some engines skip load_html in their native history.
                                    if tab.view.load_html(include_str!("start.html")).is_ok() {
                                        tab.local_navigation.store(false, Ordering::SeqCst);
                                        tab.start_forward_url = Some(std::mem::replace(
                                            &mut tab.display_url,
                                            Page::Start.url().to_owned(),
                                        ));
                                        tab.page = Page::Start;
                                        tab.title = Page::Start.label().to_owned();
                                        tab.loading = false;
                                        tab.pending_start_return = false;
                                    }
                                }
                                Ok(false) => {}
                                _ => {
                                    tab.pending_start_return = tab.has_start_entry;
                                    let _ = tab.view.go_back();
                                }
                            }
                        }
                    }
                    Command::Reload if !matches!(tabs[active].page, Page::Start) => {
                        if tabs[active].view.reload().is_err() {
                            notice = Some("Не удалось обновить страницу.");
                        }
                    }
                    Command::Forward if !tabs[active].loading => {
                        let tab = &mut tabs[active];
                        tab.pending_start_return = false;
                        if let Some(url) = tab.start_forward_url.take() {
                            if matches!(allowed_url(&url), Some(Page::LocalFixture)) {
                                notice = Some("Чтобы открыть локальную страницу, введи её адрес.");
                            } else if tab.view.load_url(&url).is_ok() {
                                tab.loading = true;
                            } else {
                                tab.start_forward_url = Some(url);
                                notice = Some("Не удалось открыть следующую страницу.");
                            }
                        } else {
                            let _ = tab.view.go_forward();
                        }
                    }
                    _ => {}
                }
                if let Some(selection) = &start_selection {
                    selection.store(tabs[active].id, Ordering::Relaxed);
                }
                if chrome_ready {
                    sync_chrome(&chrome, &tabs, active, notice, attempted_address.as_deref());
                }
            }
            Event::WindowEvent {
                event: WindowEvent::Resized(_),
                ..
            } => {
                let (chrome_rect, content_rect) = bounds(&window);
                let _ = chrome.set_bounds(chrome_rect);
                if let Some((view, _)) = &shared_start {
                    let _ = view.set_bounds(content_rect);
                }
                for tab in &tabs {
                    if !shared_start.as_ref().is_some_and(|(view, _)| Rc::ptr_eq(view, &tab.view)) {
                        let _ = tab.view.set_bounds(content_rect);
                    }
                }
            }
            Event::WindowEvent {
                event: WindowEvent::CloseRequested,
                ..
            } => *control_flow = ControlFlow::Exit,
            _ => {}
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn navigation_accepts_only_exact_origins() {
        for good in [
            "https://rust-lang.org/learn",
            "https://www.rust-lang.org/",
            "https://www.wikipedia.org/wiki/Rust",
            "https://en.wikipedia.org/wiki/Rust_(programming_language)",
            "https://ru.wikipedia.org/wiki/Special:Search?search=Rust",
            "https://excalidraw.com/#room=x",
            "http://127.0.0.1:43127/test",
        ] {
            assert!(allowed_url(good).is_some(), "{good}");
        }
        for bad in [
            "https://rust-lang.org.evil.test/",
            "https://ru.wikipedia.org.evil.test/",
            "https://de.wikipedia.org/",
            "https://user:pass@rust-lang.org/",
            "http://rust-lang.org/",
            "https://rust-lang.org:8443/",
            "http://127.0.0.1:43128/",
            "http://localhost:43127/",
            "file:///etc/passwd",
            "mailto:test@example.com",
            "search:start",
        ] {
            assert!(allowed_url(bad).is_none(), "{bad}");
        }
    }

    #[test]
    fn public_navigation_does_not_inherit_loopback_access() {
        let policy = AtomicBool::new(false);
        let local = "http://127.0.0.1:43127/";
        assert!(!permits_content_navigation(local, &policy));
        policy.store(true, Ordering::SeqCst);
        assert!(permits_content_navigation(local, &policy));
        assert!(permits_content_navigation(
            "https://rust-lang.org/",
            &policy
        ));
        assert!(!permits_content_navigation(local, &policy));
    }

    #[test]
    fn address_validation_rejects_ambiguous_input_and_uses_canonical_url() {
        assert!(validated_url("https://rust-\nlang.org/").is_none());
        assert!(validated_url(r"https://rust-lang.org\learn").is_none());
        let (page, url) = page_from_input("HTTPS://RUST-LANG.ORG:443/learn").unwrap();
        assert!(matches!(page, Page::Rust));
        assert_eq!(url, "https://rust-lang.org/learn");
    }

    #[test]
    fn commands_are_bounded_and_typed() {
        assert!(matches!(
            parse_command(r#"{"type":"new"}"#),
            Some(Command::New)
        ));
        assert!(parse_command(r#"{"type":"open","value":"x","extra":1}"#).is_none());
        assert!(parse_command(r#"{"type":"select","index":-1}"#).is_none());
        assert!(parse_command(&"x".repeat(2049)).is_none());
        assert!(page_from_input("https://not-allowed.example/").is_none());
    }

    #[test]
    fn input_preserves_allowed_path_and_start_page() {
        let (page, url) = page_from_input("https://www.wikipedia.org/wiki/Rust").unwrap();
        assert!(matches!(page, Page::Wikipedia));
        assert_eq!(url, "https://www.wikipedia.org/wiki/Rust");
        let (page, url) = page_from_input("search:start").unwrap();
        assert!(matches!(page, Page::Start));
        assert_eq!(url, "search:start");
    }

    #[test]
    fn document_titles_are_bounded_and_strip_direction_controls() {
        assert_eq!(safe_title("  Rust \n"), Some("Rust".to_owned()));
        assert_eq!(safe_title("\u{202e}Wiki\u{202c}"), Some("Wiki".to_owned()));
        assert_eq!(
            safe_title("A\u{200f}B\u{061c}\u{200e}"),
            Some("AB".to_owned())
        );
        assert_eq!(
            safe_title("x".repeat(200).as_str())
                .unwrap()
                .chars()
                .count(),
            80
        );
        assert_eq!(safe_title("\n\t"), None);
    }

    #[test]
    fn back_to_embedded_start_restores_display_address() {
        let (page, url) = loaded_page("https://rust-lang.org/", false).unwrap();
        assert!(matches!(page, Page::Rust));
        assert_eq!(url, "https://rust-lang.org/");
        let (page, url) = loaded_page("about:blank", true).unwrap();
        assert!(matches!(page, Page::Start));
        assert_eq!(url, "search:start");
        assert!(loaded_page("about:blank", false).is_none());
        let (page, url) = loaded_page("https://rust-lang.org/learn", true).unwrap();
        assert!(matches!(page, Page::Rust));
        assert_eq!(url, "https://rust-lang.org/learn");
    }
}
