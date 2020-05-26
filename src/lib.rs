//! Easy parallel closures.
//!
//! This is a simple primitive for spawning threads in bulk and waiting for them to complete.
//! Threads are allowed to borrow local variables from the main thread.
//!
//! # Examples
//!
//! Run two threads that increment a number:
//!
//! ```
//! use easy_parallel::Parallel;
//! use std::sync::Mutex;
//!
//! let mut m = Mutex::new(0);
//!
//! Parallel::new()
//!     .add(|| *m.lock().unwrap() += 1)
//!     .add(|| *m.lock().unwrap() += 1)
//!     .run();
//!
//! assert_eq!(*m.get_mut().unwrap(), 2);
//! ```
//!
//! Print each number in a vector on a different thread:
//!
//! ```
//! use easy_parallel::Parallel;
//!
//! let v = vec![10, 20, 30];
//!
//! Parallel::new()
//!     .each(0..v.len(), |i| println!("{}", v[i]))
//!     .run();
//! ```

#![warn(missing_docs, missing_debug_implementations, rust_2018_idioms)]

use std::fmt;
use std::marker::PhantomData;
use std::mem;
use std::panic;
use std::process;
use std::thread;

/// A builder that runs closures in parallel.
#[derive(Default)]
#[must_use]
pub struct Parallel<'a> {
    /// Closures to run.
    closures: Vec<Box<dyn FnOnce() + Send + 'a>>,

    /// Makes the lifetime invariant.
    _marker: PhantomData<&'a mut &'a ()>,
}

impl<'a> Parallel<'a> {
    /// Creates a builder for running closures in parallel.
    ///
    /// # Examples
    ///
    /// ```
    /// use easy_parallel::Parallel;
    ///
    /// let p = Parallel::new();
    /// ```
    pub fn new() -> Parallel<'a> {
        Parallel {
            closures: Vec::new(),
            _marker: PhantomData,
        }
    }

    /// Adds a closure to the list.
    ///
    /// # Examples
    ///
    /// ```
    /// use easy_parallel::Parallel;
    ///
    /// Parallel::new()
    ///     .add(|| println!("hello from a thread"))
    ///     .run();
    /// ```
    pub fn add<F>(mut self, f: F) -> Parallel<'a>
    where
        F: FnOnce() + Send + 'a,
    {
        self.closures.push(Box::new(f));
        self
    }

    /// Adds a cloned closure for each item in an iterator.
    ///
    /// Each clone of the closure takes an item as an argument.
    ///
    /// # Examples
    ///
    /// ```
    /// use easy_parallel::Parallel;
    ///
    /// Parallel::new()
    ///     .each(0..5, |i| println!("thread #{}", i))
    ///     .run();
    /// ```
    pub fn each<T, I, F>(mut self, iter: I, f: F) -> Parallel<'a>
    where
        I: IntoIterator<Item = T>,
        F: FnOnce(T) + Clone + Send + 'a,
        T: Send + 'a,
    {
        for t in iter.into_iter() {
            let f = f.clone();
            self.closures.push(Box::new(|| f(t)));
        }
        self
    }

    /// Spawns a thread for each closure and waits from them to complete.
    ///
    /// If a closure panics, panicking will resume in the main thread after all threads are joined.
    ///
    /// # Examples
    ///
    /// ```
    /// use easy_parallel::Parallel;
    ///
    /// Parallel::new()
    ///     .add(|| println!("thread #1"))
    ///     .add(|| println!("thread #2"))
    ///     .run();
    /// ```
    pub fn run(self) {
        // Set up a guard that aborts on a panic.
        let guard = NoPanic;

        // Join handles for spawned threads.
        let mut handles = Vec::new();

        // Spawn a thread for each closure.
        for f in self.closures {
            // Convert the `FnOnce()` closure into a `FnMut()` closure.
            let mut f = Some(f);
            let f = move || (f.take().unwrap())();

            // Erase the `'a` lifetime.
            let f: Box<dyn FnMut() + Send + 'a> = Box::new(f);
            let f: Box<dyn FnMut() + Send + 'static> = unsafe { mem::transmute(f) };

            // Spawn a thread for the closure.
            handles.push(thread::spawn(f));
        }

        let mut last_err = None;

        // Join threads and save the last panic if there was one.
        for h in handles {
            if let Err(err) = h.join() {
                last_err = Some(err);
            }
        }

        // Drop the guard because we may resume a panic now.
        drop(guard);

        // If a closure panicked, resume the last panic.
        if let Some(err) = last_err {
            panic::resume_unwind(err);
        }
    }
}

impl fmt::Debug for Parallel<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Parallel")
            .field("len", &self.closures.len())
            .finish()
    }
}

/// Aborts the process if dropped while panicking.
struct NoPanic;

impl Drop for NoPanic {
    fn drop(&mut self) {
        if thread::panicking() {
            process::abort();
        }
    }
}
