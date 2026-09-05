// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Gaëtan Dezeiraud, Louis Pinaud

//! Thread-safe progress state shared by UI frontends.
//!
//! Workers only publish the latest value. Each frontend polls a snapshot from
//! its own UI loop, so worker speed is independent from rendering speed.

use std::sync::Arc;
use winui_support::{AsyncValue, Progress as ProgressSnapshot};

pub(super) type ProgressStore = AsyncValue<ProgressSnapshot>;

pub(super) fn callback(store: &ProgressStore) -> common::ProgressFn {
    let store = store.clone();
    Arc::new(move |done, total, name| {
        store.call(ProgressSnapshot {
            done,
            total,
            name: name.to_owned(),
        });
    })
}

#[cfg(test)]
mod tests {
    use super::{ProgressSnapshot, ProgressStore, callback};

    #[test]
    fn snapshot_returns_latest_published_progress() {
        let store = ProgressStore::default();
        let publish = callback(&store);
        publish(2, 5, "Removing files");
        publish(3, 5, "Removing shortcuts");

        assert_eq!(
            store.get(),
            ProgressSnapshot {
                done: 3,
                total: 5,
                name: "Removing shortcuts".to_owned(),
            }
        );
    }

    #[test]
    fn worker_thread_and_ui_snapshot_share_the_same_store() {
        let store = ProgressStore::default();
        let publish = callback(&store);
        std::thread::spawn(move || publish(1, 1, "Done"))
            .join()
            .unwrap();

        assert_eq!(store.get().name, "Done");
    }
}
