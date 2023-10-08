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
//! let squares = Parallel::new()
//!     .each(0..v.len(), |i| v[i] * v[i])
//!     .run();
//!
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
#![forbid(unsafe_code)]
#![doc(
    html_favicon_url = "https://raw.githubusercontent.com/smol-rs/smol/master/assets/images/logo_fullsize_transparent.png"
)]
#![doc(
    html_logo_url = "https://raw.githubusercontent.com/smol-rs/smol/master/assets/images/logo_fullsize_transparent.png"
)]

use std::fmt;
use std::iter;
use std::panic;
use std::sync::mpsc;
use std::thread;

/// A builder that runs closures in parallel.
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
    #[allow(clippy::should_implement_trait)]
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
    /// Results are collected in the order in which closures were added. One of the closures always
    /// runs on the main thread because there is no point in spawning an extra thread for it.
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
    ///     .each(1..=3, |i| 10 * i)
    ///     .add(|| 100)
    ///     .collect::<Vec<_>>();
    ///
    /// assert_eq!(res, [10, 20, 30, 100]);
    /// ```
    pub fn collect<C>(mut self) -> C
    where
        T: Send + 'a,
        C: FromIterator<T> + Extend<T>,
    {
        // Get the last closure.
        let f = match self.closures.pop() {
            None => return iter::empty().collect(),
            Some(f) => f,
        };

        // Spawn threads, run the last closure on the current thread.
        let (mut results, r) = self.finish_in::<_, _, C>(f);
        results.extend(Some(r));
        results
    }

    /// Runs each closure on a separate thread and collects their results.
    ///
    /// Results are collected in the order in which closures were added. One of the closures always
    /// runs on the main thread because there is no point in spawning an extra thread for it.
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
    ///     .each(1..=3, |i| 10 * i)
    ///     .add(|| 100)
    ///     .run();
    ///
    /// assert_eq!(res, [10, 20, 30, 100]);
    /// ```
    pub fn run(self) -> Vec<T>
    where
        T: Send + 'a,
    {
        self.collect()
    }

    /// Finishes with a closure to run on the main thread, starts threads, and collects results.
    ///
    /// Results are collected in the order in which closures were added.
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
    /// let (res, ()) = Parallel::new()
    ///     .each(1..=3, |i| 10 * i)
    ///     .finish(|| println!("Waiting for results"));
    ///
    /// assert_eq!(res, [10, 20, 30]);
    /// ```
    pub fn finish<F, R>(self, f: F) -> (Vec<T>, R)
    where
        F: FnOnce() -> R,
        T: Send + 'a,
    {
        self.finish_in::<_, _, Vec<T>>(f)
    }

    /// Finishes with a closure to run on the main thread, starts threads, and collects results into an
    /// arbitrary container.
    ///
    /// Results are collected in the order in which closures were added.
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
    /// let (res, ()) = Parallel::new()
    ///     .each(1..=3, |i| 10 * i)
    ///     .finish_in::<_, _, Vec<i32>>(|| println!("Waiting for results"));
    ///
    /// assert_eq!(res, [10, 20, 30]);
    /// ```
    pub fn finish_in<F, R, C>(self, f: F) -> (C, R)
    where
        F: FnOnce() -> R,
        T: Send + 'a,
        C: FromIterator<T>,
    {
        // Set up a new thread scope.
        thread::scope(|scope| {
            // Join handles for spawned threads.
            let mut handles = Vec::new();

            // Channels to collect results from spawned threads.
            let mut receivers = Vec::new();

            for f in self.closures.into_iter() {
                // Wrap into a closure that sends the result back.
                let (sender, receiver) = mpsc::channel();
                let f = move || sender.send(f()).unwrap();

                // Spawn it on the scope.
                handles.push(scope.spawn(f));
                receivers.push(receiver);
            }

            let mut last_err = None;

            // Run the main closure on the main thread.
            let res = panic::catch_unwind(panic::AssertUnwindSafe(f));

            // Join threads and save the last panic if there was one.
            for h in handles {
                if let Err(err) = h.join() {
                    last_err = Some(err);
                }
            }

            // If a thread has panicked, resume the last collected panic.
            if let Some(err) = last_err {
                panic::resume_unwind(err);
            }

            // Collect the results from threads.
            let results = receivers.into_iter().map(|r| r.recv().unwrap()).collect();

            // If the main closure panicked, resume its panic.
            match res {
                Ok(r) => (results, r),
                Err(err) => panic::resume_unwind(err),
            }
        })
    }
}

impl<T> fmt::Debug for Parallel<'_, T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Parallel")
            .field("len", &self.closures.len())
            .finish()
    }
}

impl<T> Default for Parallel<'_, T> {
    fn default() -> Self {
        Self::new()
    }
}
