// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

use objc2::MainThreadMarker;
use objc2_foundation::{NSArray, NSUUID};
use objc2_web_kit::{WKWebsiteDataRecord, WKWebsiteDataStore};
use std::{collections::BTreeSet, ptr::NonNull};

pub(super) const PROFILE_ID: [u8; 16] = [
    0x51, 0x86, 0xf1, 0xe4, 0xfc, 0xa0, 0x47, 0xb3, 0xa7, 0x98, 0xc4, 0xe9, 0x1c, 0xe9, 0x13, 0x55,
];

pub(super) fn manage(
    remove: Option<String>,
    complete: impl Fn(Vec<String>, Option<bool>) + 'static,
) {
    let mtm = MainThreadMarker::new().expect("site data is managed on the main thread");
    // Only called on macOS 14+ for the ordinary Search profile.
    let store =
        unsafe { WKWebsiteDataStore::dataStoreForIdentifier(&NSUUID::from_bytes(PROFILE_ID), mtm) };
    let types = unsafe { WKWebsiteDataStore::allWebsiteDataTypes(mtm) };
    let completion = std::rc::Rc::new(complete);
    let records_store = store.clone();
    let records_types = types.clone();
    let callback = block2::RcBlock::new(move |records: NonNull<NSArray<WKWebsiteDataRecord>>| {
        // WebKit keeps the array alive for the duration of this callback.
        let records = unsafe { records.as_ref() };
        let mut names = BTreeSet::new();
        let mut selected = Vec::new();
        for record in records {
            let name = unsafe { record.displayName() }.to_string();
            if remove.as_deref() == Some(name.as_str()) {
                selected.push(record);
            } else {
                names.insert(name);
            }
        }
        let names: Vec<_> = names.into_iter().collect();
        if selected.is_empty() {
            completion(names, remove.as_ref().map(|_| false));
            return;
        }
        let completion = completion.clone();
        let refreshed_store = records_store.clone();
        let refreshed_types = records_types.clone();
        let removed = block2::RcBlock::new(move || {
            let completion = completion.clone();
            let refreshed =
                block2::RcBlock::new(move |records: NonNull<NSArray<WKWebsiteDataRecord>>| {
                    let records = unsafe { records.as_ref() };
                    let names: BTreeSet<_> = records
                        .into_iter()
                        .map(|record| unsafe { record.displayName() }.to_string())
                        .collect();
                    completion(names.into_iter().collect(), Some(true));
                });
            unsafe {
                refreshed_store
                    .fetchDataRecordsOfTypes_completionHandler(&refreshed_types, &refreshed);
            }
        });
        let selected = NSArray::from_retained_slice(&selected);
        // Acknowledgement is sent only after WebKit finishes removing those records.
        unsafe {
            records_store.removeDataOfTypes_forDataRecords_completionHandler(
                &records_types,
                &selected,
                &removed,
            );
        }
    });
    unsafe { store.fetchDataRecordsOfTypes_completionHandler(&types, &callback) };
}
