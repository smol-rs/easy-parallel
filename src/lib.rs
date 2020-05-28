//! Run closures in parallel.
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
//! Square each number of a vector on a different thread:
//!
//! ```
//! use easy_parallel::Parallel;
//!
//! let v = vec![10, 20, 30];
//!
//! let mut squares = Parallel::new()
//!     .each(0..v.len(), |i| v[i] * v[i])
//!     .run();
//!
//! squares.sort();
//! assert_eq!(squares, [100, 400, 900]);
//! ```
//!
//! Compute the sum of numbers in an array:
//!
//! ```
//! use easy_parallel::Parallel;
//!
//! fn par_sum(v: &[i32]) -> i32 {
//!     const THRESHOLD: usize = 2;
//!
//!     if v.len() <= THRESHOLD {
//!         v.iter().copied().sum()
//!     } else {
//!         let half = (v.len() + 1) / 2;
//!         let sums = Parallel::new().each(v.chunks(half), par_sum).run();
//!         sums.into_iter().sum()
//!     }
//! }
//!
//! let v = [1, 25, -4, 10, 8];
//! assert_eq!(par_sum(&v), 40);
//! ```

#![warn(missing_docs, missing_debug_implementations, rust_2018_idioms)]

use std::fmt;
use std::mem;
use std::panic;
use std::process;
use std::thread;

/// A builder that runs closures in parallel.
#[derive(Default)]
#[must_use]
pub struct Parallel<'a, T> {
    /// Closures to run.
    closures: Vec<Box<dyn FnOnce() -> T + Send + 'a>>,
}

impl<'a, T> Parallel<'a, T> {
    /// Creates a builder for running closures in parallel.
    ///
    /// # Examples
    ///
    /// ```
    /// use easy_parallel::Parallel;
    ///
    /// let p = Parallel::<()>::new();
    /// ```
    pub fn new() -> Parallel<'a, T> {
        Parallel {
            closures: Vec::new(),
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
    pub fn add<F>(mut self, f: F) -> Parallel<'a, T>
    where
        F: FnOnce() -> T + Send + 'a,
        T: Send + 'a,
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
    ///     .each(0..5, |i| println!("hello from thread #{}", i))
    ///     .run();
    /// ```
    pub fn each<A, I, F>(mut self, iter: I, f: F) -> Parallel<'a, T>
    where
        I: IntoIterator<Item = A>,
        F: FnOnce(A) -> T + Clone + Send + 'a,
        A: Send + 'a,
        T: Send + 'a,
    {
        for t in iter.into_iter() {
            let f = f.clone();
            self.closures.push(Box::new(|| f(t)));
        }
        self
    }

    /// Runs each closure on a separate thread and collects their results.
    ///
    /// Results are collected in the order in which closures were initially submitted.
    /// One of the closures always runs on the main thread
    /// because there is no point in spawning an extra thread for it.
    ///
    /// If a closure panics, panicking will resume in the main thread after all threads are joined.
    ///
    /// # Examples
    ///
    /// ```
    /// use easy_parallel::Parallel;
    /// use std::thread;
    /// use std::time::Duration;
    ///
    /// let res = Parallel::new()
    ///     .each(0..5, |i| {
    ///         thread::sleep(Duration::from_secs(i));
    ///         4 - i
    ///     })
    ///     .run();
    ///
    /// // Threads finish in reverse order because of how they sleep.
    /// assert_eq!(res, [4, 3, 2, 1, 0]);
    /// ```
    pub fn run(self) -> Vec<T>
    where
        T: Send + 'a,
    {
        // Vec to collect results from spawned threads.
        let mut results = self.closures.iter().map(|_| None).collect::<Vec<_>>();

        // Wrap closures to assign results into the Vec
        let wrapped_closures = self.closures.into_iter().zip(&mut results).map(|(f, result)| {
            Box::new(move || *result = Some(f())) as Box<dyn FnOnce() + Send + '_>
        });

        // This is a separate function so that `&mut results` can be borrow-checked.
        Self::run_without_return_values(wrapped_closures);

        // Collect the results.
        results.into_iter().map(Option::unwrap).collect()
    }

    fn run_without_return_values<'b>(mut closures: impl Iterator<Item=Box<dyn FnOnce() + Send + 'b>>) {
        // Get the first closure.
        let f = match closures.next() {
            None => return,
            Some(f) => f,
        };

        // Set up a guard that aborts on panic,
        // to make sure we don’t exit the `'b` lifetime without joining threads.
        let guard = NoPanic;

        // Join handles for spawned threads.
        let mut handles = Vec::new();

        // Spawn a thread for each closure.
        for f in closures {
            // Erase the `'b` lifetime.
            let f: Box<dyn FnOnce() + Send + 'b> = f;
            let f: Box<dyn FnOnce() + Send + 'static> = unsafe { mem::transmute(f) };

            // Spawn a thread for the closure.
            handles.push(thread::spawn(f));
        }

        let mut last_err = None;

        // Run the first closure on the main thread.
        match panic::catch_unwind(panic::AssertUnwindSafe(f)) {
            Ok(()) => {}
            Err(err) => last_err = Some(err),
        }

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

impl<T> fmt::Debug for Parallel<'_, T> {
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
