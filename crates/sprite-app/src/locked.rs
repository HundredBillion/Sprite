//! One rule for a poisoned lock: take the guard anyway.
//!
//! Every mutex in this crate guards state that a panicking holder cannot
//! leave half-written in a way the next holder cannot survive, so recovery is
//! always the right answer and no site should decide it alone.

use std::sync::{Mutex, MutexGuard, PoisonError};

/// Locks `mutex`, recovering the guard if a previous holder panicked.
pub(crate) fn locked<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

#[cfg(test)]
mod tests {
    use super::locked;
    use std::sync::{Arc, Mutex};

    #[test]
    fn a_poisoned_lock_still_hands_out_its_guard() {
        let mutex = Arc::new(Mutex::new(1));
        let poisoner = Arc::clone(&mutex);
        let panicked = std::thread::spawn(move || {
            let _guard = poisoner.lock().unwrap();
            panic!("poison the lock");
        })
        .join();
        assert!(panicked.is_err());
        assert!(mutex.is_poisoned());
        assert_eq!(*locked(&mutex), 1);
    }
}
