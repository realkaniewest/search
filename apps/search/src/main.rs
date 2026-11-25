// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

mod bookmarks;
mod history;
#[cfg(target_os = "macos")]
mod macos_download;
#[cfg(target_os = "macos")]
mod macos_shortcuts;
#[cfg(target_os = "macos")]
mod macos_site_data;
#[cfg(test)]
mod permission_policy_probe;

use bookmarks::{Bookmarks, MAX_BOOKMARK_BYTES};
use history::{History, MAX_HISTORY_BYTES};
use serde::{Deserialize, Serialize};
use std::rc::Rc;
use std::{
    env, fs,
    io::{self, Read, Write},
    path::{Path, PathBuf},
    process,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
    },
    thread,
    time::{Duration, Instant},
};
use tao::{
    dpi::{LogicalPosition, LogicalSize},
    event::{Event, WindowEvent},
    event_loop::{ControlFlow, EventLoopBuilder},
    window::{Window, WindowBuilder},
};
use url::{Host, Url};
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
#[cfg(target_os = "macos")]
use wry::WebViewExtMacOS;

const CHROME_HEIGHT: f64 = 50.0;
const LOCAL_ORIGIN_PORT: u16 = 43127;
const MEDIA_FRAME_ORIGIN_PORT: u16 = 43128;
const MAX_TABS: usize = 12;
const PAGE_LOAD_TIMEOUT: Duration = Duration::from_secs(15);
const HAS_HTTPS_PREVIEW: bool = cfg!(all(target_os = "macos", debug_assertions))
    && option_env!("SEARCH_BUILD_HTTPS_PREVIEW").is_some();

#[derive(Clone, Copy, Eq, PartialEq)]
enum Mode {
    Normal,
    Smoke,
    SecuritySmoke,
    HttpsPreview,
    PrivatePreview,
    DeveloperPreview,
}

#[derive(Default)]
struct SecurityCounts {
    fixture_mode: bool,
    https_preview: bool,
    persistent_site_data: bool,
    developer_preview: bool,
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
    Composer {
        visible: bool,
    },
    Reopen,
    Select {
        id: u64,
    },
    Close {
        id: u64,
    },
    Open {
        value: String,
        #[serde(default)]
        engine: SearchEngine,
    },
    #[cfg(target_os = "macos")]
    #[serde(rename = "download-address")]
    DownloadAddress {
        value: String,
    },
    #[cfg(target_os = "macos")]
    #[serde(rename = "cancel-download")]
    CancelDownload,
    #[serde(rename = "developer-window")]
    DeveloperWindow,
    #[serde(rename = "inspect-page")]
    InspectPage,
    #[serde(rename = "private-window")]
    PrivateWindow,
    #[serde(rename = "bookmarks-list")]
    BookmarksList,
    #[serde(rename = "bookmark-toggle")]
    BookmarkToggle,
    #[serde(rename = "bookmark-remove")]
    BookmarkRemove {
        value: String,
    },
    #[serde(rename = "history-list")]
    HistoryList,
    #[serde(rename = "history-clear")]
    HistoryClear,
    #[serde(rename = "history-remove")]
    HistoryRemove {
        value: String,
    },
    #[serde(rename = "set-search-engine")]
    SetSearchEngine {
        engine: SearchEngine,
    },
    #[serde(rename = "save-settings")]
    SaveSettings {
        settings: Settings,
    },
    #[cfg(target_os = "macos")]
    #[serde(rename = "site-data-list")]
    SiteDataList,
    #[cfg(target_os = "macos")]
    #[serde(rename = "site-data-remove")]
    SiteDataRemove {
        name: String,
    },
    #[serde(rename = "cancel-address")]
    CancelAddress,
    Back,
    Forward,
    Reload,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
enum SearchEngine {
    #[default]
    Google,
    Yandex,
    DuckDuckGo,
    Bing,
}

impl SearchEngine {
    fn search(self, query: &str) -> Option<Url> {
        let (endpoint, parameter) = match self {
            Self::Google => ("https://www.google.com/search", "q"),
            Self::Yandex => ("https://ya.ru/search/", "text"),
            Self::DuckDuckGo => ("https://duckduckgo.com/", "q"),
            Self::Bing => ("https://www.bing.com/search", "q"),
        };
        let mut url = Url::parse(endpoint).ok()?;
        url.query_pairs_mut().append_pair(parameter, query);
        Some(url)
    }
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
enum Palette {
    #[default]
    Graphite,
    Midnight,
    Paper,
    System,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
enum PageZoom {
    #[serde(rename = "75")]
    Small,
    #[serde(rename = "90")]
    Reduced,
    #[default]
    #[serde(rename = "100")]
    Standard,
    #[serde(rename = "110")]
    Enlarged,
    #[serde(rename = "125")]
    Large,
    #[serde(rename = "150")]
    ExtraLarge,
    #[serde(rename = "200")]
    Double,
}

impl PageZoom {
    fn apply(self, tab: &Tab) {
        let factor = if matches!(tab.page, Page::Start) {
            1.0
        } else {
            match self {
                Self::Small => 0.75,
                Self::Reduced => 0.9,
                Self::Standard => 1.0,
                Self::Enlarged => 1.1,
                Self::Large => 1.25,
                Self::ExtraLarge => 1.5,
                Self::Double => 2.0,
            }
        };
        let _ = tab.view.zoom(factor);
    }
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
enum DownloadFolder {
    #[default]
    Recent,
    Downloads,
    Desktop,
    Documents,
}

#[cfg(target_os = "macos")]
impl DownloadFolder {
    fn directory(self) -> Option<PathBuf> {
        let name = match self {
            Self::Recent => return None,
            Self::Downloads => "Downloads",
            Self::Desktop => "Desktop",
            Self::Documents => "Documents",
        };
        let home = PathBuf::from(env::var_os("HOME")?);
        home.is_absolute().then(|| home.join(name))
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
struct Settings {
    engine: SearchEngine,
    private_engine: SearchEngine,
    palette: Palette,
    compact_tabs: bool,
    reduced_motion: bool,
    opaque_toolbar: bool,
    restore_tabs: bool,
    record_history: bool,
    page_zoom: PageZoom,
    download_folder: DownloadFolder,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            engine: SearchEngine::Google,
            private_engine: SearchEngine::Google,
            palette: Palette::Graphite,
            compact_tabs: false,
            reduced_motion: false,
            opaque_toolbar: false,
            restore_tabs: true,
            record_history: true,
            page_zoom: PageZoom::Standard,
            download_folder: DownloadFolder::Recent,
        }
    }
}

fn load_settings(path: &Path, legacy_engine: &Path) -> Settings {
    read_profile_data(path, 4096)
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_else(|| Settings {
            engine: load_search_engine(legacy_engine),
            ..Settings::default()
        })
}

fn update_settings(
    path: &Path,
    legacy_engine: &Path,
    update: impl FnOnce(Settings) -> io::Result<Settings>,
) -> io::Result<Settings> {
    let parent = path.parent().ok_or(io::ErrorKind::InvalidInput)?;
    fs::create_dir_all(parent)?;
    let lock = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(path.with_extension("lock"))?;
    lock.lock()?;
    let updated = update(load_settings(path, legacy_engine))?;
    save_preference(path, &updated)?;
    Ok(updated)
}

fn show_settings(chrome: &WebView, settings: Settings, private: bool, saved: Option<&str>) {
    if let Ok(json) = serde_json::to_string(&(settings, private, saved)) {
        let _ = chrome.evaluate_script(&format!("window.renderSettings(...{json})"));
    }
}

fn preference_path() -> Option<PathBuf> {
    let home = PathBuf::from(env::var_os("HOME")?);
    home.is_absolute()
        .then(|| home.join("Library/Application Support/Search Preview/search-engine.json"))
}

fn read_profile_data(path: &Path, limit: usize) -> Option<Vec<u8>> {
    let mut bytes = Vec::new();
    fs::File::open(path)
        .ok()?
        .take(limit as u64 + 1)
        .read_to_end(&mut bytes)
        .ok()?;
    (bytes.len() <= limit).then_some(bytes)
}

fn load_search_engine(path: &Path) -> SearchEngine {
    let mut value = Vec::new();
    fs::File::open(path)
        .and_then(|file| file.take(64).read_to_end(&mut value))
        .ok()
        .and_then(|_| serde_json::from_slice(&value).ok())
        .unwrap_or_default()
}

fn save_preference(path: &Path, value: &impl Serialize) -> io::Result<()> {
    let parent = path.parent().ok_or(io::ErrorKind::InvalidInput)?;
    fs::create_dir_all(parent)?;
    let name = path.file_name().ok_or(io::ErrorKind::InvalidInput)?;
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let temp = parent.join(format!(
        ".{}-{}-{stamp}.tmp",
        name.to_string_lossy(),
        process::id()
    ));
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(&temp)?;
    let result = (|| {
        serde_json::to_writer(&mut file, value)?;
        file.flush()?;
        fs::rename(&temp, path)
    })();
    if result.is_err() {
        let _ = fs::remove_file(temp);
    }
    result
}

#[derive(Deserialize, Serialize)]
struct SavedSession {
    active: usize,
    tabs: Vec<String>,
}

#[cfg(target_os = "macos")]
fn load_session(path: &Path) -> Option<SavedSession> {
    const MAX_SESSION_BYTES: u64 = 256 * 1024;
    let mut bytes = Vec::new();
    fs::File::open(path)
        .ok()?
        .take(MAX_SESSION_BYTES + 1)
        .read_to_end(&mut bytes)
        .ok()?;
    if bytes.len() as u64 > MAX_SESSION_BYTES {
        return None;
    }
    let mut session: SavedSession = serde_json::from_slice(&bytes).ok()?;
    session.tabs.truncate(MAX_TABS);
    if session.tabs.is_empty() {
        return None;
    }
    session.active = session.active.min(session.tabs.len() - 1);
    Some(session)
}

#[cfg(target_os = "macos")]
#[derive(Clone, Copy)]
enum FixtureSheetMode {
    Dismiss,
    Navigate,
    DeferredGeolocation,
    DeferredGeolocationNavigate,
}

#[cfg(target_os = "macos")]
impl FixtureSheetMode {
    fn from_env() -> Option<Self> {
        Some(
            match env::var("SEARCH_SMOKE_PERMISSION_SHEET").ok()?.as_str() {
                "1" => Self::Dismiss,
                "navigation" => Self::Navigate,
                "deferred-geolocation" => Self::DeferredGeolocation,
                "deferred-geolocation-navigation" => Self::DeferredGeolocationNavigate,
                _ => return None,
            },
        )
    }

    fn defers_geolocation(self) -> bool {
        matches!(
            self,
            Self::DeferredGeolocation | Self::DeferredGeolocationNavigate
        )
    }

    fn navigates(self) -> bool {
        matches!(self, Self::Navigate | Self::DeferredGeolocationNavigate)
    }
}

#[cfg(target_os = "macos")]
type PendingFixtureDenial = (u64, Box<dyn FnOnce()>);

#[cfg(target_os = "macos")]
thread_local! {
    static FIXTURE_GEOLOCATION_DENIAL: std::cell::RefCell<Option<PendingFixtureDenial>> = const {
        std::cell::RefCell::new(None)
    };
}

#[cfg(target_os = "macos")]
fn finish_fixture_geolocation(owner: Option<u64>) {
    let pending = FIXTURE_GEOLOCATION_DENIAL.with(|pending| {
        let mut pending = pending.borrow_mut();
        if owner.is_none() || pending.as_ref().is_some_and(|(id, _)| Some(*id) == owner) {
            pending.take()
        } else {
            None
        }
    });
    if let Some((_, deny)) = pending {
        deny();
        println!("SECURITY: deferred_geolocation_denied");
    }
}

#[derive(Debug)]
enum AppEvent {
    #[cfg(target_os = "macos")]
    NativeShortcut(macos_shortcuts::Shortcut),
    #[cfg(target_os = "macos")]
    DownloadDestination(Url, Option<PathBuf>),
    #[cfg(target_os = "macos")]
    DownloadFinished(macos_download::Outcome),
    ChromeReady,
    StartLink {
        id: u64,
        url: String,
    },
    ContentStarted {
        id: u64,
        url: String,
    },
    ContentLoaded {
        id: u64,
        url: String,
    },
    ContentTitle {
        id: u64,
        title: String,
    },
    #[cfg(target_os = "macos")]
    SiteDataResult {
        names: Vec<String>,
        removed: Option<bool>,
    },
    NavigationTick,
    SecurityProgress(u64),
    #[cfg(target_os = "macos")]
    FixturePermissionRequest(u64, &'static str),
    #[cfg(target_os = "macos")]
    FixturePermissionDismissed(bool),
    SecurityTick,
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
    Web,
}

impl Page {
    fn label(self) -> &'static str {
        match self {
            Self::Start => "Новая вкладка",
            Self::Rust => "Rust",
            Self::Wikipedia => "Wikipedia",
            Self::Excalidraw => "Excalidraw",
            Self::LocalFixture => "Локальная страница",
            Self::Web => "HTTPS",
        }
    }

    fn url(self) -> &'static str {
        match self {
            Self::Start => "search:start",
            Self::Rust => "https://rust-lang.org/",
            Self::Wikipedia => "https://www.wikipedia.org/",
            Self::Excalidraw => "https://excalidraw.com/",
            Self::LocalFixture => "http://127.0.0.1:43127/",
            Self::Web => "about:blank",
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

fn preview_url(raw: &str) -> Option<(Page, Url)> {
    let (scheme, rest) = raw.split_once("://")?;
    let authority = rest.split(['/', '?', '#']).next()?;
    if !scheme.eq_ignore_ascii_case("https")
        || authority.is_empty()
        || authority.contains('@')
        || raw.contains('\\')
        || raw.chars().any(|ch| ch.is_control() || ch.is_whitespace())
    {
        return None;
    }
    let url = Url::parse(raw).ok()?;
    let Host::Domain(host) = url.host()? else {
        return None;
    };
    let host = host.trim_end_matches('.');
    (url.port_or_known_default() == Some(443)
        && host.contains('.')
        && !host.ends_with(".localhost")
        && !host.ends_with(".local"))
    .then(|| {
        (
            validated_url(url.as_str()).map_or(Page::Web, |(page, _)| page),
            url,
        )
    })
}

fn preview_destination(raw: &str, engine: SearchEngine) -> Option<(Page, String)> {
    let input = raw.trim();
    if input == Page::Start.url() {
        return Some((Page::Start, Page::Start.url().to_owned()));
    }
    if let Some((page, url)) = preview_url(input) {
        return Some((page, url.into()));
    }
    if input.is_empty() || input.contains("://") || input.chars().any(char::is_control) {
        return None;
    }
    let single_token = !input.chars().any(char::is_whitespace);
    let authority = input.split(['/', '?', '#']).next()?;
    if single_token
        && (authority.contains('.')
            || authority.contains('@')
            || input.contains(':')
            || input.contains('\\')
            || authority.eq_ignore_ascii_case("localhost"))
    {
        return preview_url(&format!("https://{input}")).map(|(page, url)| (page, url.into()));
    }
    let first_token = input.split_whitespace().next()?;
    let looks_like_bare_url = first_token
        .split_once(['/', '?', '#', ':'])
        .is_some_and(|(host, _)| host.contains('.') || host.eq_ignore_ascii_case("localhost"));
    if first_token.contains('@')
        || looks_like_bare_url
        || first_token.split_once(':').is_some_and(|(scheme, rest)| {
            !rest.is_empty()
                && matches!(
                    scheme.to_ascii_lowercase().as_str(),
                    "http" | "https" | "file" | "mailto" | "javascript" | "data"
                )
        })
    {
        return None;
    }
    let url = engine.search(input)?;
    preview_url(url.as_str()).map(|(page, url)| (page, url.into()))
}

fn developer_url(raw: &str) -> Option<Url> {
    let (scheme, rest) = raw.split_once("://")?;
    let authority = rest.split(['/', '?', '#']).next()?;
    if !matches!(scheme.to_ascii_lowercase().as_str(), "http" | "https")
        || authority.is_empty()
        || authority.contains('@')
        || raw
            .chars()
            .any(|ch| ch == '\\' || ch.is_control() || ch.is_whitespace())
    {
        return None;
    }
    let mut url = Url::parse(raw).ok()?;
    match url.host()? {
        Host::Domain("localhost") => url.set_host(Some("127.0.0.1")).ok()?,
        Host::Ipv4(ip) if ip.octets() == [127, 0, 0, 1] => {}
        Host::Ipv6(ip) if ip.is_loopback() => {}
        _ => return None,
    }
    Some(url)
}

fn developer_destination(raw: &str) -> Option<(Page, String)> {
    let input = raw.trim();
    if input == Page::Start.url() {
        return Some((Page::Start, input.to_owned()));
    }
    let url = if input.contains("://") {
        developer_url(input)
    } else {
        developer_url(&format!("http://{input}"))
    }?;
    Some((Page::Web, url.into()))
}

fn start_document(developer: bool) -> &'static str {
    if developer {
        include_str!("developer.html")
    } else {
        include_str!("start.html")
    }
}

fn navigation_url(raw: &str, preview: bool) -> Option<(Page, Url)> {
    if preview {
        preview_url(raw)
    } else {
        validated_url(raw)
    }
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
    if matches!(&command, Command::Open { value, .. } if value.len() > 1024) {
        return None;
    }
    Some(command)
}

fn deny_permission(_: PermissionKind) -> PermissionResponse {
    PermissionResponse::Deny
}

fn full_window_bounds(window: &Window) -> Rect {
    let size = window.inner_size().to_logical::<f64>(window.scale_factor());
    Rect {
        position: LogicalPosition::new(0.0, 0.0).into(),
        size: size.into(),
    }
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
    load_watch: LoadWatch,
    has_start_entry: bool,
    pending_start_return: bool,
    start_forward_url: Option<String>,
    view: Rc<WebView>,
    local_navigation: Arc<AtomicBool>,
}

#[derive(Default)]
struct LoadWatch {
    deadline: Option<Instant>,
    timed_out: bool,
}

impl LoadWatch {
    fn start(&mut self, now: Instant) {
        self.deadline = Some(now + PAGE_LOAD_TIMEOUT);
        self.timed_out = false;
    }

    fn clear(&mut self) {
        *self = Self::default();
    }

    fn expire(&mut self, now: Instant) -> bool {
        if self.deadline.is_some_and(|deadline| now >= deadline) {
            self.deadline = None;
            self.timed_out = true;
            return true;
        }
        false
    }
}

fn only_fragment_changed(before: &str, after: &str) -> bool {
    let (Ok(mut before), Ok(mut after)) = (Url::parse(before), Url::parse(after)) else {
        return false;
    };
    if before.fragment() == after.fragment() {
        return false;
    }
    before.set_fragment(None);
    after.set_fragment(None);
    before == after
}

fn load_watch_for_page(page: Page) -> LoadWatch {
    let mut watch = LoadWatch::default();
    if !matches!(page, Page::Start) {
        watch.start(Instant::now());
    }
    watch
}

impl Tab {
    fn has_virtual_start_back(&self) -> bool {
        self.has_start_entry && !matches!(self.page, Page::Start)
    }
}

fn loaded_page(
    url: &str,
    pending_start_return: bool,
    preview: bool,
    developer: bool,
) -> Option<(Page, String)> {
    if url == "about:blank" && pending_start_return {
        return Some((Page::Start, Page::Start.url().to_owned()));
    }
    if preview && url == "about:blank" {
        return Some((Page::Web, url.to_owned()));
    }
    if developer {
        return developer_url(url).map(|url| (Page::Web, url.into()));
    }
    navigation_url(url, preview).map(|(page, url)| (page, url.into()))
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
    can_back: bool,
    can_forward: bool,
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
    let tab = &tabs[active];
    let native_back = tab.view.can_go_back();
    let native_forward = tab.view.can_go_forward();
    let virtual_back = matches!(native_back, Ok(false))
        && matches!(native_forward, Ok(false))
        && tab.has_virtual_start_back();
    let state = ChromeState {
        active,
        can_back: tab.start_forward_url.is_none()
            && (matches!(native_back, Ok(true)) || virtual_back),
        can_forward: !tab.loading
            && (tab.start_forward_url.is_some() || matches!(native_forward, Ok(true))),
        tabs: tabs
            .iter()
            .map(|tab| TabInfo {
                id: tab.id,
                label: &tab.title,
                url: &tab.display_url,
                loading: tab.loading,
            })
            .collect(),
        notice: notice
            .or(tab.load_watch.timed_out.then_some(
                "Страница пока не загрузилась. Проверь соединение или повтори переход.",
            )),
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

fn macos_major_version() -> Option<u32> {
    let result = process::Command::new("/usr/bin/sw_vers")
        .arg("-productVersion")
        .output()
        .ok()?;
    if !result.status.success() {
        return None;
    }
    String::from_utf8_lossy(&result.stdout)
        .split('.')
        .next()?
        .trim()
        .parse()
        .ok()
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
    let persistent =
        counts.persistent_site_data && !matches!(page, Page::Start | Page::LocalFixture);
    let developer = counts.developer_preview;
    let developer_origin = developer
        .then(|| developer_url(initial_url))
        .flatten()
        .map(|url| url.origin());
    let navigation_counts = counts;
    let fixture_proxy = proxy.clone();
    #[cfg(target_os = "macos")]
    let permission_proxy = proxy.clone();
    #[cfg(target_os = "macos")]
    let denial_proxy = proxy.clone();
    let security_fixture = navigation_counts.fixture_mode && matches!(page, Page::LocalFixture);
    let builder = WebViewBuilder::new()
        .with_bounds(rect)
        .with_incognito(!persistent)
        .with_devtools(developer)
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
            let allowed = if developer {
                target == "about:blank"
                    || developer_url(&target).is_some_and(|url| {
                        developer_origin.as_ref() == Some(&url.origin())
                            && Url::parse(&target)
                                .ok()
                                .is_some_and(|candidate| candidate.host() == url.host())
                    })
            } else if navigation_counts.https_preview {
                target == "about:blank" || preview_url(&target).is_some()
            } else {
                permits_content_navigation(&target, &policy)
                    || (security_fixture
                        && policy.load(Ordering::SeqCst)
                        && target
                            == format!("http://127.0.0.1:{MEDIA_FRAME_ORIGIN_PORT}/media-frame"))
            };
            if env::var_os("SEARCH_DEBUG_NAV").is_some() {
                eprintln!("content navigation allowed={allowed}");
            }
            if !allowed {
                navigation_counts
                    .blocked_navigation
                    .fetch_add(1, Ordering::SeqCst);
                // The fixture attempts this URL only after posting its report.
                if security_fixture && target == "https://blocked.invalid/" {
                    let _ = fixture_proxy.send_event(AppEvent::SecurityProgress(id));
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
        builder.with_html(start_document(developer))
    } else {
        builder.with_url(initial_url)
    };
    #[cfg(target_os = "macos")]
    let builder = if security_fixture {
        use wry::{MacosPermissionRequest, WebViewBuilderExtMacos};
        let show_fixture_sheet =
            FixtureSheetMode::from_env().is_some_and(|mode| !mode.defers_geolocation());
        builder.with_native_permission_observer(move |origin, frame, request| {
            // WebKit supplies these objects only for the duration of this callback.
            let source = |origin: &objc2_web_kit::WKSecurityOrigin| {
                let (local_host, port) = unsafe {
                    (
                        origin.protocol().to_string() == "http"
                            && origin.host().to_string() == "127.0.0.1",
                        origin.port(),
                    )
                };
                match (local_host, port) {
                    (true, port) if port == LOCAL_ORIGIN_PORT as isize => "primary",
                    (true, port) if port == MEDIA_FRAME_ORIGIN_PORT as isize => "alternate",
                    _ => "unexpected",
                }
            };
            let callback_source = source(origin);
            let frame_source = source(&*unsafe { frame.securityOrigin() });
            let frame_kind = if unsafe { frame.isMainFrame() } { "main" } else { "child" };
            let request_kind = match request {
                MacosPermissionRequest::MediaCapture(_) => "media",
                MacosPermissionRequest::Geolocation => "geolocation",
            };
            println!("SECURITY: {request_kind}_origin={callback_source} frame={frame_kind} frame_origin={frame_source}");
            if show_fixture_sheet && frame_kind == "main"
                && callback_source == "primary" && frame_source == "primary"
            {
                let _ = permission_proxy.send_event(AppEvent::FixturePermissionRequest(id, request_kind));
            }
        })
    } else {
        builder
    };
    #[cfg(target_os = "macos")]
    let builder = if security_fixture
        && id == 1
        && FixtureSheetMode::from_env().is_some_and(FixtureSheetMode::defers_geolocation)
    {
        use wry::WebViewBuilderExtMacos;
        let deferred_once = std::cell::Cell::new(false);
        builder.with_geolocation_denial_handler(move |origin, frame, deny| {
            let is_primary = |origin: &objc2_web_kit::WKSecurityOrigin| unsafe {
                origin.protocol().to_string() == "http"
                    && origin.host().to_string() == "127.0.0.1"
                    && origin.port() == LOCAL_ORIGIN_PORT as isize
            };
            if deferred_once.get()
                || !unsafe { frame.isMainFrame() }
                || !is_primary(origin)
                || !is_primary(&*unsafe { frame.securityOrigin() })
            {
                return;
            }
            let stored = FIXTURE_GEOLOCATION_DENIAL.with(|pending| {
                let mut pending = pending.borrow_mut();
                if pending.is_some() {
                    return false;
                }
                *pending = Some((id, deny));
                true
            });
            if stored {
                deferred_once.set(true);
            }
            if stored
                && denial_proxy
                    .send_event(AppEvent::FixturePermissionRequest(id, "geolocation"))
                    .is_err()
            {
                finish_fixture_geolocation(Some(id));
            }
        })
    } else {
        builder
    };
    #[cfg(target_os = "macos")]
    let builder = if persistent {
        use wry::WebViewBuilderExtDarwin;
        builder.with_data_store_identifier(macos_site_data::PROFILE_ID)
    } else {
        builder
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

fn main() {
    let args = env::args().skip(1).collect::<Vec<_>>();
    let return_to_search = args == ["--developer-preview", "--from-search"];
    let mode = match args.as_slice() {
        [] => Mode::Normal,
        [flag] if flag == "--smoke" => Mode::Smoke,
        [flag] if flag == "--security-smoke" => Mode::SecuritySmoke,
        [flag] if flag == "--https-preview" && HAS_HTTPS_PREVIEW => Mode::HttpsPreview,
        [flag] if flag == "--developer-preview" && HAS_HTTPS_PREVIEW => Mode::DeveloperPreview,
        _ if return_to_search && HAS_HTTPS_PREVIEW => Mode::DeveloperPreview,
        [flag] if flag == "--private-preview" && HAS_HTTPS_PREVIEW => Mode::PrivatePreview,
        _ => {
            eprintln!("Usage: search-browser [--smoke|--security-smoke]");
            process::exit(2);
        }
    };
    let developer = mode == Mode::DeveloperPreview;
    let private = mode == Mode::PrivatePreview || developer;
    let preview = mode == Mode::HttpsPreview || private;
    let macos_version = (cfg!(target_os = "macos") && preview)
        .then(macos_major_version)
        .flatten()
        .unwrap_or_default();
    let supports_page_zoom = macos_version >= 11;
    let legacy_engine_path = preview.then(preference_path).flatten();
    let settings_path = legacy_engine_path
        .as_ref()
        .map(|path| path.with_file_name("settings.json"));
    let mut settings = settings_path
        .as_deref()
        .zip(legacy_engine_path.as_deref())
        .map_or_else(Settings::default, |(path, legacy)| {
            load_settings(path, legacy)
        });
    let profile_path = legacy_engine_path.as_ref().filter(|_| !private);
    let session_path = profile_path
        .as_ref()
        .map(|path| path.with_file_name("session.json"));
    let history_path = profile_path
        .as_ref()
        .map(|path| path.with_file_name("history.json"));
    let mut history = history_path
        .as_deref()
        .and_then(|path| read_profile_data(path, MAX_HISTORY_BYTES))
        .and_then(|bytes| History::from_json(&bytes))
        .unwrap_or_default();
    let bookmarks_path = profile_path
        .as_ref()
        .map(|path| path.with_file_name("bookmarks.json"));
    let mut bookmarks = bookmarks_path
        .as_deref()
        .and_then(|path| read_profile_data(path, MAX_BOOKMARK_BYTES))
        .and_then(|bytes| Bookmarks::from_json(&bytes))
        .unwrap_or_default();
    let event_loop = EventLoopBuilder::<AppEvent>::with_user_event().build();
    let window_builder = WindowBuilder::new()
        .with_title(if developer {
            "Search · Разработка"
        } else if private {
            "Search · Временное окно"
        } else {
            "Search"
        })
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
        let timer_proxy = proxy.clone();
        thread::spawn(move || {
            let deadline = Instant::now() + Duration::from_secs(50);
            while Instant::now() < deadline {
                thread::sleep(Duration::from_secs(1));
                if timer_proxy.send_event(AppEvent::SecurityTick).is_err() {
                    return;
                }
            }
            let _ = timer_proxy.send_event(AppEvent::TimedOut);
        });
    }
    let security_counts = Arc::new(SecurityCounts {
        fixture_mode: mode == Mode::SecuritySmoke,
        https_preview: preview,
        developer_preview: developer,
        persistent_site_data: preview && !private && macos_version >= 14,
        ..SecurityCounts::default()
    });
    let ready_proxy = proxy.clone();
    let chrome_builder = WebViewBuilder::new()
        .with_bounds(chrome_rect)
        .with_html(include_str!("chrome.html"))
        .with_initialization_script(if return_to_search {
            "document.addEventListener('DOMContentLoaded', () => document.documentElement.classList.add('mac-toolbar', 'https-preview', 'private-preview', 'developer-preview', 'developer-from-search'));"
        } else if developer {
            "document.addEventListener('DOMContentLoaded', () => document.documentElement.classList.add('mac-toolbar', 'https-preview', 'private-preview', 'developer-preview'));"
        } else if private {
            "document.addEventListener('DOMContentLoaded', () => document.documentElement.classList.add('mac-toolbar', 'https-preview', 'private-preview'));"
        } else if preview {
            "document.addEventListener('DOMContentLoaded', () => document.documentElement.classList.add('mac-toolbar', 'https-preview'));"
        } else if cfg!(target_os = "macos") {
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
    #[cfg(target_os = "macos")]
    let fixture_navigation_test = mode == Mode::SecuritySmoke
        && FixtureSheetMode::from_env().is_some_and(FixtureSheetMode::navigates);
    #[cfg(not(target_os = "macos"))]
    let fixture_navigation_test = false;
    let initial_url = if fixture_navigation_test {
        "http://127.0.0.1:43127/permission-bootstrap"
    } else {
        initial_page.url()
    };
    let start_selection = (mode != Mode::SecuritySmoke).then(|| Arc::new(AtomicU64::new(1)));
    #[cfg(target_os = "linux")]
    let (first, local_navigation) = create_content(
        &window,
        &fixed,
        1,
        event_loop.create_proxy(),
        security_counts.clone(),
        (initial_page, initial_url, start_selection.clone()),
        content_rect,
    )
    .expect("Search start page failed");
    #[cfg(not(target_os = "linux"))]
    let (first, local_navigation) = create_content(
        &window,
        1,
        event_loop.create_proxy(),
        security_counts.clone(),
        (initial_page, initial_url, start_selection.clone()),
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
        load_watch: LoadWatch::default(),
        has_start_entry: matches!(initial_page, Page::Start),
        pending_start_return: false,
        start_forward_url: None,
        view: first,
        local_navigation,
    }];
    let mut active = 0usize;
    #[cfg(target_os = "macos")]
    let require_fixture_permission_sheet =
        mode == Mode::SecuritySmoke && FixtureSheetMode::from_env().is_some();
    #[cfg(not(target_os = "macos"))]
    let require_fixture_permission_sheet = false;
    #[cfg(target_os = "macos")]
    let mut fixture_permission_sheet: Option<(
        u64,
        objc2::rc::Retained<objc2_app_kit::NSAlert>,
    )> = None;
    #[cfg(target_os = "macos")]
    let mut fixture_permission_sheet_passed = false;
    #[cfg(not(target_os = "macos"))]
    let fixture_permission_sheet_passed = false;
    let mut composer_visible = false;
    #[cfg(target_os = "macos")]
    let mut site_data_pending = false;
    #[cfg(target_os = "macos")]
    let mut choosing_download_destination = false;
    #[cfg(target_os = "macos")]
    let mut download_job: Option<(
        Arc<macos_download::DownloadControl>,
        thread::JoinHandle<()>,
    )> = None;
    let mut last_closed = None::<(Page, String)>;
    let mut next_id = 2u64;
    let mut chrome_ready = false;
    let mut content_ready = false;
    let mut security_reports = [false; 2];
    let mut second_fixture_requested = false;
    let mut notice = None;
    let mut attempted_address: Option<String> = None;
    let content_proxy = event_loop.create_proxy();
    #[cfg(target_os = "macos")]
    let shortcuts_enabled = Rc::new(std::cell::Cell::new(false));
    #[cfg(target_os = "macos")]
    let mut shortcut_monitor = if preview {
        macos_shortcuts::Monitor::install(
            &window,
            event_loop.create_proxy(),
            shortcuts_enabled.clone(),
        )
    } else {
        None
    };

    #[cfg(target_os = "macos")]
    if let Some(session) = session_path
        .as_deref()
        .filter(|_| settings.restore_tabs)
        .and_then(load_session)
    {
        let mut restored = Vec::with_capacity(session.tabs.len());
        let shared = shared_start
            .as_ref()
            .expect("preview has a shared start view");
        let _ = shared.0.set_visible(false);
        for address in &session.tabs {
            let website = preview_url(address);
            let destination = website.as_ref().and_then(|(page, url)| {
                create_content(
                    &window,
                    next_id,
                    content_proxy.clone(),
                    security_counts.clone(),
                    (*page, url.as_str(), None),
                    content_rect,
                )
                .ok()
                .map(|(view, policy)| (Rc::new(view), policy, *page, url.to_string()))
            });
            let (view, local_navigation, page, display_url) = destination.unwrap_or_else(|| {
                (
                    shared.0.clone(),
                    shared.1.clone(),
                    Page::Start,
                    Page::Start.url().to_owned(),
                )
            });
            let _ = view.set_visible(false);
            restored.push(Tab {
                id: next_id,
                page,
                display_url,
                title: page.label().to_owned(),
                loading: !matches!(page, Page::Start),
                load_watch: load_watch_for_page(page),
                has_start_entry: matches!(page, Page::Start),
                pending_start_return: false,
                start_forward_url: None,
                view,
                local_navigation,
            });
            next_id += 1;
        }
        active = session.active;
        let _ = restored[active].view.set_visible(true);
        tabs = restored;
        if let Some(selection) = &start_selection {
            selection.store(tabs[active].id, Ordering::Relaxed);
        }
    }

    if mode == Mode::Smoke {
        // Native WebView creation may block before the event loop can consume
        // readiness callbacks. The wrapper separately bounds total runtime.
        let timer_proxy = event_loop.create_proxy();
        thread::spawn(move || {
            thread::sleep(Duration::from_secs(25));
            let _ = timer_proxy.send_event(AppEvent::TimedOut);
        });
    }

    if preview {
        let timer = event_loop.create_proxy();
        thread::spawn(move || {
            loop {
                thread::sleep(Duration::from_secs(1));
                if timer.send_event(AppEvent::NavigationTick).is_err() {
                    break;
                }
            }
        });
    }

    event_loop.run(move |event, _, control_flow| {
        let _keep_window_alive = &window;
        *control_flow = ControlFlow::Wait;
        let command_from_start = match event {
            Event::UserEvent(AppEvent::StartLink { id, url }) => {
                if id == tabs[active].id && shared_start.as_ref().is_some_and(|(view, _)| Rc::ptr_eq(view, &tabs[active].view)) {
                    Event::UserEvent(AppEvent::Command(Command::Open { value: url, engine: SearchEngine::default() }))
                } else {
                    return;
                }
            }
            event => event,
        };
        match command_from_start {
            #[cfg(target_os = "macos")]
            Event::UserEvent(AppEvent::NativeShortcut(shortcut)) => {
                if chrome_ready && !composer_visible && let Ok(json) = serde_json::to_string(&shortcut) {
                    if shortcut.focuses_address() { let _ = chrome.focus(); }
                    let _ = chrome.evaluate_script(&format!("window.searchNativeShortcut({json})"));
                }
            }
            #[cfg(target_os = "macos")]
            Event::UserEvent(AppEvent::DownloadDestination(url, destination))
                if choosing_download_destination && download_job.is_none() =>
            {
                choosing_download_destination = false;
                if let Some(destination) = destination {
                    let cancel = Arc::new(macos_download::DownloadControl::default());
                    let worker = Arc::clone(&cancel);
                    let proxy = content_proxy.clone();
                    let _ = chrome.evaluate_script("window.setDownloadActive(true)");
                    notice = Some("Загрузка файла…");
                    let worker = thread::spawn(move || {
                        let outcome = macos_download::fetch(&url, &destination, &worker);
                        let _ = proxy.send_event(AppEvent::DownloadFinished(outcome));
                    });
                    download_job = Some((cancel, worker));
                    sync_chrome(&chrome, &tabs, active, notice, attempted_address.as_deref());
                }
            }
            #[cfg(target_os = "macos")]
            Event::UserEvent(AppEvent::DownloadFinished(outcome)) => {
                if let Some((_, worker)) = download_job.take() { let _ = worker.join(); }
                use macos_download::Outcome;
                notice = Some(match outcome {
                    Outcome::Saved => "Файл сохранён.",
                    Outcome::Cancelled => "Загрузка отменена.",
                    Outcome::DestinationExists => "Файл с таким именем уже есть. Выбери другое имя.",
                    Outcome::Failed => "Не удалось скачать файл по этому адресу.",
                });
                let _ = chrome.evaluate_script("window.setDownloadActive(false)");
                sync_chrome(&chrome, &tabs, active, notice, attempted_address.as_deref());
            }
            Event::UserEvent(AppEvent::ChromeReady) => {
                if preview {
                    show_settings(&chrome, settings, private, None);
                    if security_counts.persistent_site_data {
                        let _ = chrome.evaluate_script("document.getElementById('site-data-group').dataset.unavailable = ''");
                    }
                    if !supports_page_zoom {
                        let _ = chrome.evaluate_script("document.getElementById('page-zoom-group').dataset.unavailable = 'true'");
                    }
                }
                if preview && !private && let Ok(json) = serde_json::to_string(bookmarks.entries()) {
                    let _ = chrome.evaluate_script(&format!("window.renderBookmarks({json})"));
                }
                if mode == Mode::Smoke { println!("SMOKE: chrome loaded"); }
                chrome_ready = true;
                #[cfg(target_os = "macos")]
                shortcuts_enabled.set(!composer_visible);
                sync_chrome(&chrome, &tabs, active, notice, attempted_address.as_deref());
                if mode == Mode::Smoke && content_ready {
                    println!("PASS: Search chrome and start content loaded");
                    *control_flow = ControlFlow::ExitWithCode(0);
                }
            }
            Event::UserEvent(AppEvent::NavigationTick) if preview => {
                let now = Instant::now();
                let mut changed = false;
                for tab in &mut tabs {
                    if tab.load_watch.expire(now) {
                        tab.loading = false;
                        changed = true;
                    }
                }
                if chrome_ready && changed {
                    sync_chrome(&chrome, &tabs, active, notice, attempted_address.as_deref());
                }
            }
            Event::UserEvent(AppEvent::ContentStarted { id, url }) => {
                #[cfg(target_os = "macos")]
                if let Some((owner, alert)) = fixture_permission_sheet.as_ref()
                    && *owner == id
                {
                    let parent = chrome.ns_window();
                    let sheet = alert.window();
                    if parent.attachedSheet().is_some_and(|attached| std::ptr::eq(&*attached, &*sheet)) {
                        parent.endSheet_returnCode(&sheet, objc2_app_kit::NSModalResponseAbort);
                    }
                }
                if let Some(tab) = tabs.iter_mut().find(|tab| tab.id == id)
                    && (url == "about:blank" || if developer { developer_url(&url).is_some() } else { navigation_url(&url, preview).is_some() })
                {
                    tab.loading = true;
                    if preview && url != "about:blank" && tab.load_watch.deadline.is_none() {
                        tab.load_watch.start(Instant::now());
                    }
                    if let Some((page, display_url)) = loaded_page(&url, tab.pending_start_return || matches!(tab.page, Page::Start), preview, developer) {
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
                    let finished_start = url == "about:blank" && tab.pending_start_return;
                    if url != "about:blank" || finished_start || tab.load_watch.deadline.is_none() {
                        tab.loading = false;
                        if url != "about:blank" || finished_start {
                            tab.load_watch.clear();
                        }
                    }
                    if let Some((page, display_url)) = loaded_page(&url, tab.pending_start_return || matches!(tab.page, Page::Start), preview, developer) {
                        tab.page = page;
                        tab.display_url = display_url;
                        tab.pending_start_return = false;
                        if matches!(page, Page::Start) {
                            tab.title = page.label().to_owned();
                        }
                    }
                    if supports_page_zoom && !developer {
                        settings.page_zoom.apply(tab);
                    }
                    #[cfg(target_os = "macos")]
                    if mode == Mode::SecuritySmoke && id == 1
                        && matches!(tab.page, Page::LocalFixture)
                        && macos_major_version().is_some_and(|version| version >= 11)
                    {
                        let zoom = |view: &WebView| -> f64 {
                            // pageZoom is public on macOS 11 and newer.
                            unsafe { objc2::msg_send![&*view.webview(), pageZoom] }
                        };
                        PageZoom::ExtraLarge.apply(tab);
                        let enlarged = zoom(&tab.view);
                        PageZoom::Standard.apply(tab);
                        if enlarged != 1.5 || zoom(&tab.view) != 1.0 || zoom(&chrome) != 1.0 {
                            eprintln!("FAIL: WebKit page zoom did not apply and reset independently of chrome");
                            *control_flow = ControlFlow::ExitWithCode(1);
                            return;
                        }
                        println!("SMOKE: WebKit page zoom applied 150% and reset to 100%");
                    }
                    if settings.record_history && let Some(path) = &history_path
                        && let Some((_, visited)) = preview_url(&url)
                    {
                        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)
                            .map_or(0, |elapsed| elapsed.as_secs());
                        if history.record(&visited, &tab.title, now)
                            && let Err(error) = save_preference(path, &history)
                        {
                            eprintln!("Search could not save history: {error}");
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
            Event::UserEvent(AppEvent::SecurityProgress(id)) if mode == Mode::SecuritySmoke => {
                if let 1..=2 = id {
                    security_reports[id as usize - 1] = true;
                }
            }
            #[cfg(target_os = "macos")]
            Event::UserEvent(AppEvent::FixturePermissionRequest(id, kind))
                if require_fixture_permission_sheet
                    && !fixture_permission_sheet_passed
                    && fixture_permission_sheet.is_none()
                    && tabs[active].id == id
                    && matches!(tabs[active].page, Page::LocalFixture) =>
            {
                use objc2::MainThreadMarker;
                use objc2_app_kit::{NSAlert, NSAlertFirstButtonReturn, NSModalResponseAbort};
                use objc2_foundation::NSString;
                let window = chrome.ns_window();
                if window.attachedSheet().is_some() {
                    finish_fixture_geolocation(Some(id));
                    return;
                }
                if let Some(main_thread) = MainThreadMarker::new() {
                    let alert = NSAlert::new(main_thread);
                    let explanation = if kind == "geolocation" {
                        "Тестовый запрос к геолокации отклонён"
                    } else {
                        "Тестовый запрос к камере или микрофону отклонён"
                    };
                    alert.setMessageText(&NSString::from_str(explanation));
                    alert.setInformativeText(&NSString::from_str(&format!("http://127.0.0.1:{LOCAL_ORIGIN_PORT}")));
                    alert.addButtonWithTitle(&NSString::from_str("Закрыть"));
                    let sheet_proxy = content_proxy.clone();
                    let completion = block2::RcBlock::new(move |response| {
                        let expected = if fixture_navigation_test { NSModalResponseAbort } else { NSAlertFirstButtonReturn };
                        let _ = sheet_proxy.send_event(AppEvent::FixturePermissionDismissed(response == expected));
                    });
                    alert.beginSheetModalForWindow_completionHandler(&window, Some(&completion));
                    fixture_permission_sheet = Some((id, alert));
                    if fixture_navigation_test {
                        let _ = tabs[active].view.load_url(Page::LocalFixture.url());
                    }
                }
            }
            #[cfg(target_os = "macos")]
            Event::UserEvent(AppEvent::FixturePermissionRequest(id, _)) => {
                finish_fixture_geolocation(Some(id));
            }
            #[cfg(target_os = "macos")]
            Event::UserEvent(AppEvent::FixturePermissionDismissed(dismissed))
                if require_fixture_permission_sheet =>
            {
                fixture_permission_sheet = None;
                finish_fixture_geolocation(None);
                fixture_permission_sheet_passed = dismissed;
                println!("SECURITY: permission_sheet_dismissed={dismissed} navigation={fixture_navigation_test}");
            }
            Event::UserEvent(AppEvent::SecurityTick) if mode == Mode::SecuritySmoke => {
                #[cfg(target_os = "macos")]
                if let Some((_, alert)) = fixture_permission_sheet.as_ref()
                    && !fixture_navigation_test
                {
                    let sheet = alert.window();
                    let parent = chrome.ns_window();
                    if parent.attachedSheet().is_some_and(|attached| std::ptr::eq(&*attached, &*sheet)) {
                        parent.endSheet_returnCode(&sheet, objc2_app_kit::NSAlertFirstButtonReturn);
                    }
                }
                let (nav, popup, download, permission) = security_counts.snapshot();
                let first_ready = content_ready && chrome_ready && tabs.first().is_some_and(
                    |tab| tab.title == "Search security fixture ready"
                );
                // Wait for both the page report and the asynchronous WebView callbacks.
                if (!require_fixture_permission_sheet || fixture_permission_sheet_passed)
                    && first_ready && nav > 0 && download > 0 && security_reports[0]
                    && !second_fixture_requested
                {
                    second_fixture_requested = true;
                    let _ = content_proxy.send_event(AppEvent::Command(Command::New));
                }
                if first_ready && security_reports == [true; 2] && nav >= 2 && download >= 2
                    && tabs.get(1).is_some_and(|tab| tab.title == tabs[0].title)
                {
                    println!("SECURITY: blocked_navigation={nav} blocked_popup={popup} blocked_download={download} denied_permission={permission}");
                    println!("PASS: Search security boundary checks");
                    *control_flow = ControlFlow::ExitWithCode(0);
                }
            }
            Event::UserEvent(AppEvent::TimedOut) => {
                if mode == Mode::SecuritySmoke {
                    eprintln!("SECURITY: reports={security_reports:?} chrome_ready={chrome_ready} content_ready={content_ready} counts={:?}", security_counts.snapshot());
                }
                eprintln!("FAIL: Search first window did not load before the test deadline");
                *control_flow = ControlFlow::ExitWithCode(1);
            }
            #[cfg(target_os = "macos")]
            Event::UserEvent(AppEvent::SiteDataResult { names, removed }) => {
                site_data_pending = false;
                if let Ok(json) = serde_json::to_string(&(names, removed)) {
                    let _ = chrome.evaluate_script(&format!("window.renderSiteData(...{json})"));
                }
            }
            Event::UserEvent(AppEvent::Command(Command::Composer { visible })) => {
                if preview && visible != composer_visible {
                    let rect = if visible { full_window_bounds(&window) } else { bounds(&window).0 };
                    if chrome.set_bounds(rect).is_ok() {
                        composer_visible = visible;
                        #[cfg(target_os = "macos")]
                        shortcuts_enabled.set(chrome_ready && !visible);
                        let _ = tabs[active].view.set_visible(!visible);
                        if visible {
                            let _ = chrome.focus();
                        }
                    }
                }
            }
            Event::UserEvent(AppEvent::Command(command)) => {
                notice = None;
                attempted_address = None;
                match command {
                    Command::Reopen if last_closed.is_none() => {
                        notice = Some("Нет недавно закрытой вкладки.");
                    }
                    Command::New | Command::Reopen if tabs.len() < MAX_TABS => {
                        let (_, rect) = bounds(&window);
                        let page = if mode == Mode::SecuritySmoke {
                            Page::LocalFixture
                        } else {
                            Page::Start
                        };
                        let restoring = matches!(command, Command::Reopen);
                        let (page, url) = if restoring {
                            last_closed.as_ref().map_or((page, page.url()), |(page, url)| (*page, url.as_str()))
                        } else {
                            (page, page.url())
                        };
                        let shared = shared_start.as_ref().filter(|_| matches!(page, Page::Start));
                        let new_view = if let Some((view, policy)) = shared {
                            Ok((view.clone(), policy.clone()))
                        } else {
                        #[cfg(target_os = "linux")]
                        let new_view = create_content(
                            &window,
                            &fixed,
                            next_id,
                            content_proxy.clone(),
                            security_counts.clone(),
                            (page, url, None),
                            rect,
                        );
                        #[cfg(not(target_os = "linux"))]
                        let new_view = create_content(
                            &window,
                            next_id,
                            content_proxy.clone(),
                            security_counts.clone(),
                            (page, url, None),
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
                                display_url: url.to_owned(),
                                title: page.label().to_owned(),
                                loading: restoring && !matches!(page, Page::Start),
                                load_watch: if restoring && preview { load_watch_for_page(page) } else { LoadWatch::default() },
                                has_start_entry: matches!(page, Page::Start),
                                pending_start_return: false,
                                start_forward_url: None,
                                view,
                                local_navigation,
                            });
                            active = tabs.len() - 1;
                            next_id += 1;
                            if restoring {
                                last_closed = None;
                            }
                            if mode == Mode::SecuritySmoke {
                                eprintln!("SECURITY: second WebView active");
                            }
                        } else {
                            notice = Some("Не удалось открыть вкладку.");
                        }
                    }
                    Command::New | Command::Reopen => {
                        notice = Some("Открыто 12 вкладок. Закрой одну, чтобы открыть новую.");
                    }
                    Command::Select { id } => {
                        if let Some(index) = tabs.iter().position(|tab| tab.id == id)
                            && index != active
                        {
                            if !Rc::ptr_eq(&tabs[active].view, &tabs[index].view) {
                                let _ = tabs[active].view.set_visible(false);
                                let _ = tabs[index].view.set_visible(true);
                            }
                            active = index;
                        }
                    }
                    Command::Close { id } => {
                        if tabs.len() > 1
                            && let Some(index) = tabs.iter().position(|tab| tab.id == id)
                        {
                            let removed = tabs.remove(index);
                            if !matches!(removed.page, Page::LocalFixture) {
                                last_closed = Some((removed.page, removed.display_url.clone()));
                            }
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
                    Command::Open { value, engine } => {
                        let destination = if developer {
                            developer_destination(&value)
                        } else if preview {
                            preview_destination(&value, engine)
                        } else {
                            page_from_input(&value)
                        };
                        if let Some((page, url)) = destination {
                            let is_shared = shared_start.as_ref().is_some_and(|(view, _)| Rc::ptr_eq(view, &tabs[active].view));
                            let changes_local_origin = developer && !matches!(page, Page::Start)
                                && developer_url(&tabs[active].display_url).map(|url| url.origin())
                                    != developer_url(&url).map(|url| url.origin());
                            let result = if is_shared && matches!(page, Page::Start) {
                                Ok(())
                            } else if is_shared || changes_local_origin || matches!(page, Page::LocalFixture) {
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
                                    tab.view.load_html(start_document(developer))
                                } else {
                                    tab.view.load_url(&url)
                                }
                            };
                            if result.is_ok() {
                                let waits_for_commit = preview && !matches!(page, Page::Start);
                                if waits_for_commit && !only_fragment_changed(&tabs[active].display_url, &url) {
                                    tabs[active].loading = true;
                                    tabs[active].load_watch.start(Instant::now());
                                } else {
                                    tabs[active].load_watch.clear();
                                }
                                if waits_for_commit && (is_shared || changes_local_origin) {
                                    tabs[active].page = Page::Web;
                                    tabs[active].display_url = "about:blank".to_owned();
                                    tabs[active].title = Page::Web.label().to_owned();
                                } else if !waits_for_commit {
                                    tabs[active].page = page;
                                    tabs[active].display_url = url;
                                    tabs[active].title = page.label().to_owned();
                                    tabs[active].loading = !matches!(page, Page::Start);
                                }
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
                            notice = Some(if developer {
                                "Введи адрес локального сервера, например localhost:3000."
                            } else if preview {
                                "Этот адрес недоступен. Введи публичный HTTPS-адрес или поисковый запрос."
                            } else {
                                "Пока доступны Rust, Wikipedia, Excalidraw и local."
                            });
                            attempted_address = Some(value);
                        }
                    }
                    #[cfg(target_os = "macos")]
                    Command::DownloadAddress { value } if preview && !private => {
                        if download_job.is_none() && !choosing_download_destination {
                            if let Some((_, url)) = preview_url(value.trim()) {

                                choosing_download_destination = true;
                                let proxy = content_proxy.clone();
                                let request_url = url.clone();
                                macos_download::choose_destination(&chrome.ns_window(), &url, settings.download_folder.directory().as_deref(), move |destination| {
                                    let _ = proxy.send_event(AppEvent::DownloadDestination(request_url.clone(), destination));
                                });
                            } else {
                                notice = Some("Введи прямой публичный HTTPS-адрес файла.");
                            }
                        }
                    }
                    #[cfg(target_os = "macos")]
                    Command::CancelDownload if preview => {
                        if let Some((cancel, _)) = &download_job {
                            notice = Some(if cancel.cancel() {
                                "Отменяю загрузку…"
                            } else {
                                "Файл уже сохраняется."
                            });
                        }
                    }
                    Command::DeveloperWindow if preview && !private => {
                        let opened = env::current_exe().and_then(|binary| {
                            process::Command::new(binary).args(["--developer-preview", "--from-search"]).spawn()
                        });
                        if opened.is_err() {
                            notice = Some("Не удалось открыть окно разработки.");
                        }
                    }
                    #[cfg(all(target_os = "macos", debug_assertions))]
                    Command::InspectPage if developer && developer_url(&tabs[active].display_url).is_some() => {
                        tabs[active].view.open_devtools();
                    }
                    Command::PrivateWindow if preview && !private => {
                        let opened = env::current_exe().and_then(|binary| {
                            process::Command::new(binary).arg("--private-preview").spawn()
                        });
                        if opened.is_err() {
                            notice = Some("Не удалось открыть временное окно.");
                        }
                    }
                    Command::BookmarksList | Command::BookmarkToggle | Command::BookmarkRemove { .. } if preview && !private => {
                        let mut updated = bookmarks.clone();
                        let changed = match command {
                            Command::BookmarkToggle => {
                                let current = &tabs[active];
                                let result = preview_url(&current.display_url)
                                    .and_then(|(_, url)| updated.toggle(&url, &current.title));
                                if result.is_none() {
                                    notice = Some("Не удалось добавить закладку. Проверь адрес или число сохранённых закладок.");
                                }
                                result.is_some()
                            }
                            Command::BookmarkRemove { value } => preview_url(&value)
                                .is_some_and(|(_, url)| updated.remove(&url)),
                            _ => false,
                        };
                        if changed {
                            if bookmarks_path.as_ref().is_some_and(|path| save_preference(path, &updated).is_ok()) {
                                bookmarks = updated;
                            } else {
                                notice = Some("Не удалось сохранить изменения в закладках.");
                            }
                        }
                        if let Ok(json) = serde_json::to_string(bookmarks.entries()) {
                            let _ = chrome.evaluate_script(&format!("window.renderBookmarks({json})"));
                        }
                    }
                    Command::HistoryList | Command::HistoryClear | Command::HistoryRemove { .. } if preview && !private => {
                        let mut updated = history.clone();
                        let changed = match command {
                            Command::HistoryClear => updated.clear(),
                            Command::HistoryRemove { value } => preview_url(&value)
                                .is_some_and(|(_, url)| updated.remove(&url)),
                            _ => false,
                        };
                        if changed {
                            if history_path.as_ref().is_some_and(|path| save_preference(path, &updated).is_ok()) {
                                history = updated;
                            } else {
                                notice = Some("Не удалось сохранить изменения в истории.");
                            }
                        }
                        if let Ok(json) = serde_json::to_string(history.entries()) {
                            let _ = chrome.evaluate_script(&format!("window.renderHistory({json})"));
                        }
                    }
                    Command::SetSearchEngine { engine } if preview && !private => {
                        let saved = settings_path.as_deref().zip(legacy_engine_path.as_deref())
                            .and_then(|(path, legacy)| update_settings(path, legacy, |current| {
                                Ok(Settings { engine, ..current })
                            }).ok());
                        if let Some(updated) = saved {
                            if supports_page_zoom && updated.page_zoom != settings.page_zoom {
                                for tab in &tabs {
                                    updated.page_zoom.apply(tab);
                                }
                            }
                            settings = updated;
                        } else {
                            notice = Some("Не удалось сохранить поисковик.");
                        }
                        show_settings(&chrome, settings, false, None);
                    }
                    Command::SaveSettings { settings: updated } if preview && !private => {
                        let previous_zoom = settings.page_zoom;
                        let result = settings_path.as_deref().zip(legacy_engine_path.as_deref())
                            .ok_or(io::ErrorKind::NotFound.into())
                            .and_then(|(path, legacy)| update_settings(path, legacy, |current| {
                                if current == settings {
                                    Ok(updated)
                                } else {
                                    Err(io::ErrorKind::AlreadyExists.into())
                                }
                            }));
                        let outcome = match result {
                            Ok(updated) => {
                                settings = updated;
                                "saved"
                            }
                            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                                if let Some((path, legacy)) = settings_path.as_deref().zip(legacy_engine_path.as_deref()) {
                                    settings = load_settings(path, legacy);
                                }
                                "conflict"
                            }
                            Err(_) => "failed",
                        };
                        if supports_page_zoom && settings.page_zoom != previous_zoom {
                            for tab in &tabs {
                                settings.page_zoom.apply(tab);
                            }
                        }
                        show_settings(&chrome, settings, false, Some(outcome));
                    }
                    #[cfg(target_os = "macos")]
                    Command::SiteDataList | Command::SiteDataRemove { .. }
                        if security_counts.persistent_site_data && composer_visible && !site_data_pending =>
                    {
                        let remove = match command {
                            Command::SiteDataRemove { name } if !name.is_empty() => Some(name),
                            Command::SiteDataRemove { .. } => return,
                            _ => None,
                        };
                        site_data_pending = true;
                        let result_proxy = content_proxy.clone();
                        macos_site_data::manage(remove, move |names, removed| {
                            let _ = result_proxy.send_event(AppEvent::SiteDataResult { names, removed });
                        });
                    }
                    #[cfg(target_os = "macos")]
                    Command::SiteDataList | Command::SiteDataRemove { .. } => {
                        let _ = chrome.evaluate_script("window.siteDataUnavailable()");
                    }
                    Command::CancelAddress => {}
                    Command::Back => {
                        let tab = &mut tabs[active];
                        if return_to_search && matches!(tab.page, Page::Start)
                            && (tab.start_forward_url.is_some() || !matches!(tab.view.can_go_back(), Ok(true)))
                        {
                            *control_flow = ControlFlow::Exit;
                            return;
                        }
                        if tab.start_forward_url.is_none() {
                            match tab.view.can_go_back() {
                                Ok(false)
                                    if matches!(tab.view.can_go_forward(), Ok(false))
                                        && tab.has_virtual_start_back() =>
                                {
                                    // Some engines skip load_html in their native history.
                                    if tab.view.load_html(start_document(developer)).is_ok() {
                                        tab.local_navigation.store(false, Ordering::SeqCst);
                                        tab.start_forward_url = Some(std::mem::replace(
                                            &mut tab.display_url,
                                            Page::Start.url().to_owned(),
                                        ));
                                        tab.page = Page::Start;
                                        tab.title = Page::Start.label().to_owned();
                                        tab.loading = false;
                                        tab.load_watch.clear();
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
                    Command::Reload if !matches!(tabs[active].page, Page::Start)
                        && tabs[active].display_url != "about:blank" =>
                    {
                        if tabs[active].view.reload().is_err() {
                            notice = Some("Не удалось обновить страницу.");
                        } else if preview {
                            tabs[active].load_watch.start(Instant::now());
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
                                if preview { tab.load_watch.start(Instant::now()); }
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
                let _ = chrome.set_bounds(if composer_visible { full_window_bounds(&window) } else { chrome_rect });
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
            Event::LoopDestroyed => {
                #[cfg(target_os = "macos")]
                finish_fixture_geolocation(None);
                #[cfg(target_os = "macos")]
                drop(shortcut_monitor.take());
                #[cfg(target_os = "macos")]
                if let Some((cancel, worker)) = download_job.take() {
                    cancel.cancel();
                    let _ = worker.join();
                }
                if let Some(path) = &session_path {
                    let session = SavedSession {
                        active,
                        tabs: tabs.iter().map(|tab| {
                            (tab.display_url.len() <= 16 * 1024).then(|| preview_url(&tab.display_url)).flatten()
                                .map_or_else(|| Page::Start.url().to_owned(), |(_, url)| url.into())
                        }).collect(),
                    };
                    let session = if settings.restore_tabs { session } else {
                        SavedSession { active: 0, tabs: vec![Page::Start.url().to_owned()] }
                    };
                    if let Err(error) = save_preference(path, &session) {
                        eprintln!("Search could not save its session: {error}");
                    }
                }
            }
            _ => {}
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preview_distinguishes_domain_search_and_unsupported_addresses() {
        assert_eq!(
            preview_destination("github.com", SearchEngine::Google)
                .unwrap()
                .1,
            "https://github.com/"
        );
        for (engine, host, key) in [
            (SearchEngine::Google, "www.google.com", "q"),
            (SearchEngine::Yandex, "ya.ru", "text"),
            (SearchEngine::DuckDuckGo, "duckduckgo.com", "q"),
            (SearchEngine::Bing, "www.bing.com", "q"),
        ] {
            let query = "Rust & async #тест";
            let (_, address) = preview_destination(query, engine).unwrap();
            let address = Url::parse(&address).unwrap();
            assert_eq!(address.host_str(), Some(host));
            assert_eq!(
                address.query_pairs().collect::<Vec<_>>(),
                vec![(key.into(), query.into())]
            );
        }
        for blocked in [
            "http://github.com/",
            "localhost",
            "127.0.0.1",
            "file:///tmp/test",
            "mailto:test@example.org trailing",
            "github.com/settings?token=example trailing",
        ] {
            assert!(
                preview_destination(blocked, SearchEngine::Google).is_none(),
                "{blocked}"
            );
        }
    }

    #[test]
    fn search_engine_preference_survives_reopen_and_handles_invalid_data() {
        let root = env::temp_dir().join(format!("search-engine-test-{}", process::id()));
        fs::create_dir(&root).unwrap();
        let path = root.join("search-engine.json");
        assert_eq!(load_search_engine(&path), SearchEngine::Google);
        save_preference(&path, &SearchEngine::DuckDuckGo).unwrap();
        assert_eq!(load_search_engine(&path), SearchEngine::DuckDuckGo);
        save_preference(&path, &SearchEngine::Yandex).unwrap();
        assert_eq!(load_search_engine(&path), SearchEngine::Yandex);
        fs::write(&path, br#""unknown""#).unwrap();
        assert_eq!(load_search_engine(&path), SearchEngine::Google);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn settings_migrate_the_search_engine_and_round_trip() {
        let root = env::temp_dir().join(format!("search-settings-test-{}", process::id()));
        fs::create_dir(&root).unwrap();
        let legacy = root.join("search-engine.json");
        let path = root.join("settings.json");
        save_preference(&legacy, &SearchEngine::Yandex).unwrap();
        assert_eq!(load_settings(&path, &legacy).engine, SearchEngine::Yandex);
        let settings = Settings {
            engine: SearchEngine::Bing,
            private_engine: SearchEngine::DuckDuckGo,
            palette: Palette::Paper,
            compact_tabs: true,
            reduced_motion: true,
            opaque_toolbar: true,
            restore_tabs: false,
            record_history: false,
            page_zoom: PageZoom::Large,
            download_folder: DownloadFolder::Downloads,
        };
        fs::write(
            root.join(format!(".settings.json-{}.tmp", process::id())),
            b"old",
        )
        .unwrap();
        update_settings(&path, &legacy, |_| Ok(settings)).unwrap();
        assert_eq!(load_settings(&path, &legacy), settings);
        let rejected =
            update_settings(&path, &legacy, |_| Err(io::ErrorKind::AlreadyExists.into()));
        assert_eq!(rejected.unwrap_err().kind(), io::ErrorKind::AlreadyExists);
        assert_eq!(load_settings(&path, &legacy), settings);
        fs::write(&path, vec![b' '; 4097]).unwrap();
        assert_eq!(load_settings(&path, &legacy).engine, SearchEngine::Yandex);
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn session_load_limits_tabs_and_ignores_broken_files() {
        let root = env::temp_dir().join(format!("search-session-test-{}", process::id()));
        fs::create_dir(&root).unwrap();
        let path = root.join("session.json");
        let saved = SavedSession {
            active: 100,
            tabs: vec!["https://example.org/".to_owned(); MAX_TABS + 2],
        };
        save_preference(&path, &saved).unwrap();
        let restored = load_session(&path).unwrap();
        assert_eq!(restored.tabs.len(), MAX_TABS);
        assert_eq!(restored.active, MAX_TABS - 1);
        fs::write(&path, b"{").unwrap();
        assert!(load_session(&path).is_none());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn load_timeout_is_reported_once_and_recovers() {
        assert!(only_fragment_changed(
            "https://example.org/#one",
            "https://example.org/#two"
        ));
        assert!(!only_fragment_changed(
            "https://example.org/a#one",
            "https://example.org/b#two"
        ));
        let start = Instant::now();
        let mut watch = LoadWatch::default();
        watch.start(start);
        assert!(!watch.expire(start + Duration::from_secs(1)));
        assert!(watch.expire(start + PAGE_LOAD_TIMEOUT));
        assert!(watch.timed_out);
        assert!(!watch.expire(start + PAGE_LOAD_TIMEOUT));
        watch.start(start);
        assert!(!watch.timed_out);
        watch.clear();
        assert!(!watch.expire(start + PAGE_LOAD_TIMEOUT));
    }

    #[test]
    fn developer_addresses_use_only_explicit_loopback_hosts() {
        assert_eq!(
            developer_destination("localhost:3000/docs").unwrap().1,
            "http://127.0.0.1:3000/docs"
        );
        assert_eq!(
            developer_destination("http://[::1]:8080/").unwrap().1,
            "http://[::1]:8080/"
        );
        for url in [
            "http://192.168.0.1:3000/",
            "https://example.org/",
            "http://@localhost:3000/",
            "file:///tmp/test",
        ] {
            assert!(developer_destination(url).is_none(), "{url}");
        }
        assert!(preview_url("http://127.0.0.1:3000/").is_none());
    }

    #[test]
    fn https_preview_accepts_new_sites_without_widening_normal_navigation() {
        let site = "https://developer.mozilla.org/ru/docs/Web/HTML";
        assert!(navigation_url(site, false).is_none());
        assert!(matches!(navigation_url(site, true), Some((Page::Web, _))));
        for denied in [
            "http://developer.mozilla.org/",
            "https://developer.mozilla.org:8443/",
            "https://@developer.mozilla.org/",
            "https://127.0.0.1/",
            "https://app.localhost./",
            "http://127.0.0.1:43127/",
            "file:///tmp/search-preview.html",
            "mailto:search@example.org",
        ] {
            assert!(navigation_url(denied, true).is_none(), "{denied}");
        }
    }

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
        assert!(parse_command(r#"{"type":"select","id":-1}"#).is_none());
        assert!(parse_command(r#"{"type":"close","index":0}"#).is_none());
        assert!(matches!(
            parse_command(r#"{"type":"close","id":42}"#),
            Some(Command::Close { id: 42 })
        ));
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
        let (page, url) = loaded_page("https://rust-lang.org/", false, false, false).unwrap();
        assert!(matches!(page, Page::Rust));
        assert_eq!(url, "https://rust-lang.org/");
        let (page, url) = loaded_page("about:blank", true, false, false).unwrap();
        assert!(matches!(page, Page::Start));
        assert_eq!(url, "search:start");
        assert!(loaded_page("about:blank", false, false, false).is_none());
        let (page, url) = loaded_page("https://rust-lang.org/learn", true, false, false).unwrap();
        assert!(matches!(page, Page::Rust));
        assert_eq!(url, "https://rust-lang.org/learn");
    }
}

#[cfg(test)]
mod address_input_probe;
