// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Gaëtan Dezeiraud, Louis Pinaud

use std::sync::{Arc, Mutex};

/// Progress value published by a worker and sampled by a UI frontend.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Progress {
    pub done: u64,
    pub total: u64,
    pub name: String,
}

/// Latest value published by a worker thread and sampled by a UI frontend.
///
/// Rendering polls snapshots at its own pace, independently from the worker.
#[derive(Clone)]
pub struct AsyncValue<T>(Arc<Mutex<T>>);

impl<T: Default> Default for AsyncValue<T> {
    fn default() -> Self {
        Self(Arc::new(Mutex::new(T::default())))
    }
}

impl<T> AsyncValue<T> {
    pub fn call(&self, value: T) {
        if let Ok(mut slot) = self.0.lock() {
            *slot = value;
        }
    }
}

impl<T: Default> AsyncValue<T> {
    /// Atomically consume a one-shot value without racing a concurrent writer.
    pub fn take(&self) -> T {
        self.0
            .lock()
            .map(|mut value| std::mem::take(&mut *value))
            .unwrap_or_default()
    }
}

impl<T: Clone> AsyncValue<T> {
    pub fn get(&self) -> T {
        self.0
            .lock()
            .map(|value| value.clone())
            .unwrap_or_else(|error| error.into_inner().clone())
    }
}

#[cfg(test)]
mod tests {
    use super::AsyncValue;

    #[test]
    fn worker_and_ui_share_the_latest_value() {
        let value = AsyncValue::<String>::default();
        let worker = value.clone();
        std::thread::spawn(move || worker.call("done".to_owned()))
            .join()
            .unwrap();

        assert_eq!(value.get(), "done");
    }

    #[test]
    fn take_consumes_a_one_shot_value() {
        let value = AsyncValue::<String>::default();
        value.call("ready".to_owned());

        assert_eq!(value.take(), "ready");
        assert!(value.take().is_empty());
    }
}
