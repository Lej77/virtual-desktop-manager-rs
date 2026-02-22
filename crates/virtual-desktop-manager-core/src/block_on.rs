//! A very simple async executor based on the [`futures-executor`] crate.
//!
//! The implementation mostly comes from
//! <https://docs.rs/futures-executor/0.3.30/src/futures_executor/local_pool.rs.html#45-103>.
//!
//! See also this reddit thread for more info about simple executors:
//! <https://www.reddit.com/r/rust/comments/eilw8j/what_is_the_minimum_that_must_be_implemented_to/>
//!
//! [`futures-executor`]: https://crates.io/crates/futures-executor/0.3.30

use std::{
    cell::Cell,
    future::Future,
    pin::{pin, Pin},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    task::{Context, Poll, Wake, Waker},
    thread::{self, Thread},
};

thread_local!(static ENTERED: Cell<bool> = const { Cell::new(false) });

struct Enter {}
impl Enter {
    fn new() -> Self {
        ENTERED.with(|c| {
            if c.get() {
                panic!(
                    "an execution scope has already been entered: \
                    cannot execute `block_on` from within another `block_on`"
                )
            } else {
                c.set(true);

                Enter {}
            }
        })
    }
}
impl Drop for Enter {
    fn drop(&mut self) {
        ENTERED.with(|c| {
            assert!(c.get());
            c.set(false);
        });
    }
}

struct ThreadNotify {
    /// The (single) executor thread.
    thread: Thread,
    /// A flag to ensure a wakeup (i.e. `unpark()`) is not "forgotten"
    /// before the next `park()`, which may otherwise happen if the code
    /// being executed as part of the future(s) being polled makes use of
    /// park / unpark calls of its own, i.e. we cannot assume that no other
    /// code uses park / unpark on the executing `thread`.
    unparked: AtomicBool,
}

thread_local! {
    static CURRENT_THREAD_NOTIFY: Arc<ThreadNotify> = Arc::new(ThreadNotify {
        thread: thread::current(),
        unparked: AtomicBool::new(false),
    });
}

impl Wake for ThreadNotify {
    fn wake_by_ref(self: &Arc<Self>) {
        // Make sure the wakeup is remembered until the next `park()`.
        let unparked = self.unparked.swap(true, Ordering::Release);
        if !unparked {
            // If the thread has not been unparked yet, it must be done
            // now. If it was actually parked, it will run again,
            // otherwise the token made available by `unpark`
            // may be consumed before reaching `park()`, but `unparked`
            // ensures it is not forgotten.
            self.thread.unpark();
        }
    }

    fn wake(self: Arc<Self>) {
        <ThreadNotify as Wake>::wake_by_ref(&self)
    }
}

// Set up and run a basic single-threaded spawner loop, invoking `f` on each
// turn.
fn run_executor<T, F>(mut f: F) -> T
where
    F: FnMut(&mut Context<'_>) -> Poll<T>,
{
    let _enter = Enter::new();

    CURRENT_THREAD_NOTIFY.with(|thread_notify| {
        let waker = Waker::from(Arc::clone(thread_notify));
        let mut cx = Context::from_waker(&waker);
        loop {
            if let Poll::Ready(t) = f(&mut cx) {
                return t;
            }

            // Wait for a wakeup.
            while !thread_notify.unparked.swap(false, Ordering::Acquire) {
                // No wakeup occurred. It may occur now, right before parking,
                // but in that case the token made available by `unpark()`
                // is guaranteed to still be available and `park()` is a no-op.
                thread::park();
            }
        }
    })
}

/// Run a future to completion on the current thread.
///
/// This function will block the caller until the given future has completed.
pub fn block_on<F: Future>(f: F) -> F::Output {
    let mut f = pin!(f);
    run_executor(|cx| f.as_mut().poll(cx))
}

/// Create a new future that finishes when the list of futures complete.
///
/// Note: this code was not taken from any other crate.
///
/// # Panics
///
/// The returned future will delay any panic in a queued future until all
/// futures have completed in order to prevent accidental cancellation.
pub fn simple_join<'a, Fut>(futures: impl IntoIterator<Item = Fut>) -> impl Future<Output = ()> + 'a
where
    Fut: Future<Output = ()> + 'a,
{
    struct Join<'a> {
        list: Vec<Pin<Box<dyn Future<Output = ()> + 'a>>>,
        panic: Option<Box<dyn std::any::Any + Send>>,
    }
    impl Future for Join<'_> {
        type Output = ();

        fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
            let this = &mut self.get_mut();
            let list = &mut this.list;
            list.retain_mut(|item| {
                let res = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    Future::poll(item.as_mut(), cx).is_pending()
                }));
                match res {
                    Err(payload) => {
                        this.panic.get_or_insert(payload);
                        false
                    }
                    Ok(is_pending) => is_pending,
                }
            });
            if list.is_empty() {
                if let Some(payload) = this.panic.take() {
                    std::panic::resume_unwind(payload)
                } else {
                    Poll::Ready(())
                }
            } else {
                Poll::Pending
            }
        }
    }
    Join {
        list: futures
            .into_iter()
            .map(|fut| Box::pin(fut) as Pin<Box<dyn Future<Output = ()> + '_>>)
            .collect(),
        panic: None,
    }
}
