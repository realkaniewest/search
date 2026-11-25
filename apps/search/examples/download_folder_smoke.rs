// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

#[cfg(target_os = "macos")]
#[allow(dead_code)]
#[path = "../src/macos_download.rs"]
mod macos_download;

#[cfg(target_os = "macos")]
fn main() {
    use objc2::rc::Retained;
    use objc2_app_kit::{NSModalResponseCancel, NSOpenPanel, NSSavePanel, NSWindow};
    use std::{
        cell::RefCell,
        path::{Path, PathBuf},
        rc::Rc,
        time::{Duration, Instant},
    };
    use tao::{
        event::Event,
        event_loop::{ControlFlow, EventLoop},
        platform::macos::WindowExtMacOS,
        window::WindowBuilder,
    };

    enum Step {
        OpenPicker,
        CancelPicker,
        WaitForPickerCancel,
        CancelSave,
        WaitForCancel,
    }

    fn same_path(left: &Path, right: &Path) -> bool {
        left.canonicalize()
            .ok()
            .zip(right.canonicalize().ok())
            .is_some_and(|(left, right)| left == right)
    }

    let event_loop = EventLoop::new();
    let window = WindowBuilder::new()
        .with_title("Search · Download folder smoke test")
        .build(&event_loop)
        .expect("create fixture window");
    // The Tao window owns this NSWindow and stays alive throughout the run.
    let parent = unsafe { Retained::retain(window.ns_window().cast::<NSWindow>()) }
        .expect("retain fixture window");
    let temporary = tempfile::tempdir().expect("create test directory");
    let expected = temporary.path().join("Search downloads");
    std::fs::create_dir(&expected).expect("create selected subfolder");
    let selected: Rc<RefCell<Option<Option<PathBuf>>>> = Rc::new(RefCell::new(None));
    let mut step = Step::OpenPicker;
    let mut check_after = Instant::now();
    let deadline = Instant::now() + Duration::from_secs(12);
    let mut temporary = Some(temporary);
    event_loop.run(move |event, _, flow| {
        let _keep_window = &window;
        *flow = ControlFlow::WaitUntil(Instant::now() + Duration::from_millis(60));
        if matches!(event, Event::LoopDestroyed) {
            temporary.take();
            return;
        }
        if !matches!(event, Event::MainEventsCleared) {
            return;
        }
        if Instant::now() > deadline {
            eprintln!("FAIL: native download folder test timed out");
            *flow = ControlFlow::ExitWithCode(1);
            return;
        }
        if Instant::now() < check_after {
            return;
        }

        match step {
            Step::OpenPicker => {
                let result = selected.clone();
                macos_download::choose_folder(&parent, move |folder| *result.borrow_mut() = Some(folder));
                step = Step::CancelPicker;
            }
            Step::CancelPicker => {
                let Some(panel) = parent.attachedSheet().and_then(|sheet| sheet.downcast::<NSOpenPanel>().ok()) else {
                    return;
                };
                if !panel.canChooseDirectories() || panel.canChooseFiles() || panel.allowsMultipleSelection() {
                    eprintln!("FAIL: picker did not enforce a single folder");
                    *flow = ControlFlow::ExitWithCode(1);
                    return;
                }
                parent.endSheet_returnCode(&panel, NSModalResponseCancel);
                step = Step::WaitForPickerCancel;
            }
            Step::WaitForPickerCancel => {
                let Some(folder) = selected.borrow_mut().take() else {
                    return;
                };
                if folder.is_some() {
                    eprintln!("FAIL: cancelling the picker returned a folder");
                    *flow = ControlFlow::ExitWithCode(1);
                    return;
                }
                let destination = selected.clone();
                let url = url::Url::parse("https://example.test/fixture.txt").unwrap();
                macos_download::choose_destination(&parent, &url, Some(&expected), move |path| {
                    *destination.borrow_mut() = Some(path);
                });
                check_after = Instant::now() + Duration::from_millis(500);
                step = Step::CancelSave;
            }
            Step::CancelSave => {
                let Some(panel) = parent.attachedSheet().and_then(|sheet| sheet.downcast::<NSSavePanel>().ok()) else {
                    return;
                };
                let matches = panel.directoryURL().and_then(|url| url.path())
                    .is_some_and(|path| same_path(Path::new(&path.to_string()), &expected));
                if !matches {
                    eprintln!("FAIL: save sheet did not open in the selected folder");
                    *flow = ControlFlow::ExitWithCode(1);
                    return;
                }
                parent.endSheet_returnCode(&panel, NSModalResponseCancel);
                step = Step::WaitForCancel;
            }
            Step::WaitForCancel => {
                let Some(destination) = selected.borrow_mut().take() else {
                    return;
                };
                if destination.is_some() || expected.read_dir().unwrap().next().is_some() {
                    eprintln!("FAIL: cancelling returned a destination or created a file");
                    *flow = ControlFlow::ExitWithCode(1);
                    return;
                }
                println!("PASS: Search opened a folder-only picker and used the configured starting directory in the save sheet");
                *flow = ControlFlow::ExitWithCode(0);
            }
        }
    });
}

#[cfg(not(target_os = "macos"))]
fn main() {
    eprintln!("This smoke test requires macOS");
    std::process::exit(2);
}
