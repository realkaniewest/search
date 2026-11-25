// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

use std::{
    io,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::atomic::{AtomicU8, Ordering},
    thread,
    time::Duration,
};

use objc2::MainThreadMarker;
use objc2_app_kit::{NSModalResponseOK, NSSavePanel, NSWindow};
use objc2_foundation::{NSDictionary, NSString, NSURL, NSURLQuarantinePropertiesKey};
use url::Url;

const MAX_BYTES: u64 = 100 * 1024 * 1024;

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Outcome {
    Saved,
    Cancelled,
    DestinationExists,
    Failed,
}

#[derive(Default)]
pub(crate) struct DownloadControl(AtomicU8);

impl DownloadControl {
    const RUNNING: u8 = 0;
    const CANCELLED: u8 = 1;
    const COMMITTING: u8 = 2;

    pub(crate) fn cancel(&self) -> bool {
        match self.0.compare_exchange(
            Self::RUNNING,
            Self::CANCELLED,
            Ordering::AcqRel,
            Ordering::Acquire,
        ) {
            Ok(_) | Err(Self::CANCELLED) => true,
            Err(_) => false,
        }
    }

    fn cancelled(&self) -> bool {
        self.0.load(Ordering::Acquire) == Self::CANCELLED
    }

    fn begin_commit(&self) -> bool {
        self.0
            .compare_exchange(
                Self::RUNNING,
                Self::COMMITTING,
                Ordering::AcqRel,
                Ordering::Acquire,
            )
            .is_ok()
    }
}

pub(crate) fn choose_destination(
    window: &NSWindow,
    url: &Url,
    directory: Option<&Path>,
    complete: impl Fn(Option<PathBuf>) + 'static,
) {
    let Some(main_thread) = MainThreadMarker::new() else {
        complete(None);
        return;
    };
    if window.attachedSheet().is_some() {
        complete(None);
        return;
    }
    let panel = NSSavePanel::savePanel(main_thread);
    panel.setMessage(Some(&NSString::from_str(
        "Выбери новое имя файла. Search сохраняет существующие файлы.",
    )));
    if let Some(path) = initial_directory(directory) {
        let directory_url = NSURL::fileURLWithPath_isDirectory(&NSString::from_str(path), true);
        panel.setDirectoryURL(Some(&directory_url));
    }
    panel.setNameFieldStringValue(&NSString::from_str(&suggested_name(url)));
    let selected = panel.clone();
    let callback = block2::RcBlock::new(move |response| {
        let destination = if response == NSModalResponseOK {
            selected
                .URL()
                .and_then(|url| url.path())
                .map(|path| PathBuf::from(path.to_string()))
                .filter(|path| path.is_absolute())
        } else {
            None
        };
        complete(destination);
    });
    panel.beginSheetModalForWindow_completionHandler(window, &callback);
}

fn initial_directory(directory: Option<&Path>) -> Option<&str> {
    directory
        .filter(|path| path.is_absolute() && path.is_dir())
        .and_then(Path::to_str)
}

fn suggested_name(url: &Url) -> String {
    let name = url
        .path_segments()
        .and_then(|mut parts| parts.next_back())
        .filter(|name| !name.is_empty() && !name.starts_with('.'))
        .unwrap_or("download");
    name.chars()
        .take(96)
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '.' | '-' | '_') {
                ch
            } else {
                '_'
            }
        })
        .collect()
}

#[link(name = "CoreServices", kind = "framework")]
unsafe extern "C" {
    static kLSQuarantineAgentNameKey: &'static NSString;
    static kLSQuarantineTypeKey: &'static NSString;
    static kLSQuarantineTypeWebDownload: &'static NSString;
}

fn mark_web_download(path: &Path) -> bool {
    let Some(path) = path.to_str() else {
        return false;
    };
    let file = NSURL::fileURLWithPath(&NSString::from_str(path));
    let agent = NSString::from_str("Search");
    // Launch Services exports CFString constants bridged to NSString.
    unsafe {
        let properties = NSDictionary::from_slices(
            &[kLSQuarantineAgentNameKey, kLSQuarantineTypeKey],
            &[&*agent, kLSQuarantineTypeWebDownload],
        );
        file.setResourceValue_forKey_error(Some(&properties), NSURLQuarantinePropertiesKey)
            .is_ok()
    }
}

fn finish(stage: tempfile::NamedTempFile, destination: &Path) -> Outcome {
    match stage.persist_noclobber(destination) {
        Ok(_) => Outcome::Saved,
        Err(error) if error.error.kind() == io::ErrorKind::AlreadyExists => {
            Outcome::DestinationExists
        }
        Err(_) => Outcome::Failed,
    }
}

pub(crate) fn fetch(url: &Url, destination: &Path, control: &DownloadControl) -> Outcome {
    fetch_with(destination, control, |path| {
        Command::new("/usr/bin/curl")
            .args([
                "-q",
                "--silent",
                "--fail",
                "--proto",
                "=https",
                "--connect-timeout",
                "15",
                "--max-time",
                "180",
                "--max-filesize",
                "104857600",
                "--globoff",
                "--write-out",
                "%{http_code}",
            ])
            .arg("--output")
            .arg(path)
            .arg("--url")
            .arg(url.as_str())
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
    })
}

fn fetch_with(
    destination: &Path,
    control: &DownloadControl,
    start: impl FnOnce(&Path) -> io::Result<std::process::Child>,
) -> Outcome {
    if control.cancelled() {
        return Outcome::Cancelled;
    }
    if destination.symlink_metadata().is_ok() {
        return Outcome::DestinationExists;
    }
    let Some(parent) = destination.parent() else {
        return Outcome::Failed;
    };
    let Ok(staging_dir) = tempfile::Builder::new()
        .prefix(".search-download-")
        .tempdir_in(parent)
    else {
        return Outcome::Failed;
    };
    let Ok(stage) = tempfile::NamedTempFile::new_in(staging_dir.path()) else {
        return Outcome::Failed;
    };
    let started = start(stage.path());
    let Ok(mut child) = started else {
        return Outcome::Failed;
    };
    loop {
        let cancelled_now = control.cancelled();
        if cancelled_now
            || stage
                .as_file()
                .metadata()
                .is_ok_and(|meta| meta.len() > MAX_BYTES)
        {
            let _ = child.kill();
            let _ = child.wait();
            return if cancelled_now {
                Outcome::Cancelled
            } else {
                Outcome::Failed
            };
        }
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) => thread::sleep(Duration::from_millis(75)),
            Err(_) => {
                let _ = child.kill();
                let _ = child.wait();
                return Outcome::Failed;
            }
        }
    }
    let Ok(result) = child.wait_with_output() else {
        return Outcome::Failed;
    };
    if control.cancelled() {
        return Outcome::Cancelled;
    }
    if !result.status.success()
        || result.stdout != b"200"
        || !stage
            .as_file()
            .metadata()
            .is_ok_and(|meta| meta.len() <= MAX_BYTES)
    {
        return Outcome::Failed;
    }
    if !mark_web_download(stage.path()) {
        return Outcome::Failed;
    }
    if !control.begin_commit() {
        return Outcome::Cancelled;
    }
    finish(stage, destination)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn initial_directory_uses_only_existing_absolute_folders() {
        let root = tempfile::tempdir().unwrap();
        let file = root.path().join("file");
        std::fs::write(&file, b"").unwrap();
        assert_eq!(initial_directory(Some(root.path())), root.path().to_str());
        assert_eq!(initial_directory(Some(&file)), None);
        assert_eq!(initial_directory(Some(&root.path().join("missing"))), None);
        assert_eq!(initial_directory(Some(Path::new("."))), None);
        assert_eq!(initial_directory(None), None);
    }

    #[test]
    fn mid_transfer_cancellation_removes_staged_bytes() {
        let root = tempfile::tempdir().unwrap();
        let destination = root.path().join("cancelled.txt");
        let control = DownloadControl::default();
        let (staged, ready) = std::sync::mpsc::channel();
        thread::scope(|scope| {
            let job = scope.spawn(|| {
                fetch_with(&destination, &control, move |path| {
                    std::fs::write(path, b"partial")?;
                    let child = Command::new("/bin/sleep").arg("5").spawn()?;
                    staged.send(path.to_owned()).unwrap();
                    Ok(child)
                })
            });
            let path = ready.recv_timeout(Duration::from_secs(2)).unwrap();
            assert_eq!(std::fs::read(path).unwrap(), b"partial");
            assert!(control.cancel());
            assert_eq!(job.join().unwrap(), Outcome::Cancelled);
        });
        assert!(!destination.exists());
        assert_eq!(root.path().read_dir().unwrap().count(), 0);
    }

    #[test]
    fn staging_preserves_existing_files_and_promotes_completed_bytes() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("file.txt");
        std::fs::write(&path, b"existing").unwrap();
        let mut staged = tempfile::NamedTempFile::new_in(root.path()).unwrap();
        staged.write_all(b"replacement").unwrap();
        assert_eq!(finish(staged, &path), Outcome::DestinationExists);
        assert_eq!(std::fs::read(&path).unwrap(), b"existing");
        assert_eq!(root.path().read_dir().unwrap().count(), 1);

        let mut staged = tempfile::NamedTempFile::new_in(root.path()).unwrap();
        staged.write_all(b"completed").unwrap();
        let path = root.path().join("other.txt");
        assert_eq!(finish(staged, &path), Outcome::Saved);
        assert_eq!(std::fs::read(path).unwrap(), b"completed");
    }

    #[test]
    fn cancelled_job_never_writes_a_destination() {
        let root = tempfile::tempdir().unwrap();
        let url = Url::parse("https://example.org/file.txt").unwrap();
        let path = root.path().join("file.txt");
        let control = DownloadControl::default();
        assert!(control.cancel());
        assert_eq!(fetch(&url, &path, &control), Outcome::Cancelled);
        assert_eq!(root.path().read_dir().unwrap().count(), 0);
    }
}
