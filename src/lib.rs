//! A crate providing a future similar to
//! [future's join](https://docs.rs/futures/latest/futures/future/fn.join.html) but with a limited
//! amount of concurrency.
//!
//! # Example
//!
//! ```rust
//! # tokio_test::block_on(async {
//! # use limited_join::*;
//! # mod download {
//! #     pub async fn download_file(url: String) {}
//! # }
//! # let files_to_download: Vec<String> = vec![];
//! // Pretend we have a ton of files we want to download, but don't want to
//! // overwhelm the server.
//! let futures = files_to_download.into_iter().map(download::download_file);
//!
//! // Let's limit the number of concurrent downloads to 4, and wait for all
//! // the files to download.
//! limited_join::join(futures, 4).await;
//!
//! # });
//! ```
use std::{
    future::Future,
    pin::Pin,
    task::{Context, Poll},
};

/// The [`Future`] behind the [`join`] function.
pub struct LimitedJoin<Fut>
where
    Fut: Future,
{
    inner: Pin<Box<[MaybeCompleted<Fut>]>>,
    /// How many futures can be concurrently pending.
    concurrency: usize,
}

/// Returns a future that acts as a [join](https://docs.rs/futures/latest/futures/future/fn.join.html)
/// of multiple futures, but with a limit on how many futures can be running at once.
///
/// # Example
/// ```rust
/// # tokio_test::block_on(async {
/// use std::time::{Duration, Instant};
/// use tokio::time::sleep;
///
/// let then = Instant::now();
/// let futures = std::iter::repeat(Duration::from_millis(100))
///     .map(|duration| async move { sleep(duration).await })
///     .take(4);
///
/// limited_join::join(futures, 2).await;
///
/// // Ensure all futures completed in roughly 200ms as we're processing only 2 at a time.
/// assert!(then.elapsed().as_millis() - 200 < 10);
/// # });
/// ```
pub fn join<Fut>(futures: impl IntoIterator<Item = Fut>, concurrency: usize) -> LimitedJoin<Fut>
where
    Fut: Future,
{
    let futures = futures
        .into_iter()
        .map(MaybeCompleted::InProgress)
        .collect::<Vec<_>>()
        .into_boxed_slice();
    LimitedJoin {
        inner: futures.into(),
        concurrency,
    }
}

impl<Fut> Future for LimitedJoin<Fut>
where
    Fut: Future,
{
    type Output = Vec<Fut::Output>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        // SAFETY: This is safe because we never move the inner futures.
        let this = unsafe { Pin::get_unchecked_mut(self) };
        // SAFETY: This is safe because we never move any of the futures.
        let states = unsafe { Pin::get_unchecked_mut(this.inner.as_mut()) };

        let mut remaining = states.iter().filter(|state| state.is_in_progress()).count();
        let mut to_poll = this.concurrency.min(remaining);

        let mut polled = 0;
        let mut index = 0;

        while polled < to_poll && index < states.len() {
            let state = &mut states[index];

            // Ensure that the future is ready to be polled, either by being new or by having been
            // woken up.
            if !state.is_in_progress() {
                index += 1;
                continue;
            }

            // SAFETY: This is all behind a Pin and can't be unpinned so we know it's in the same
            // location in memory.
            let res = unsafe { Pin::new_unchecked(state).poll(cx) };

            if let Poll::Ready(output) = res {
                states[index] = MaybeCompleted::Completed(output);
                remaining -= 1;

                // We've completed a future, so we can poll another one.
                to_poll += 1;
            }

            polled += 1;
            index += 1;
        }

        if remaining == 0 {
            Poll::Ready(states.iter_mut().map(|state| state.take()).collect())
        } else {
            Poll::Pending
        }
    }
}

enum MaybeCompleted<Fut: Future> {
    InProgress(Fut),
    Completed(Fut::Output),
    Drained,
}

impl<Fut: Future> MaybeCompleted<Fut> {
    fn is_in_progress(&self) -> bool {
        matches!(self, Self::InProgress { .. })
    }

    fn take(&mut self) -> Fut::Output {
        match std::mem::replace(self, MaybeCompleted::Drained) {
            Self::Completed(output) => output,
            Self::InProgress(_) => panic!("attempt to get output of incomplete future"),
            Self::Drained => panic!("attempt to get output of drained future"),
        }
    }

    unsafe fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Fut::Output> {
        let this = self.as_mut();
        let this = this.get_unchecked_mut();
        match this {
            Self::InProgress(future) => Pin::new_unchecked(future).poll(cx),
            _ => unreachable!("attempted to poll a complete or drained future"),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{
        sync::{
            atomic::{AtomicBool, Ordering},
            Arc,
        },
        time::Duration,
    };

    use tokio::time::sleep;

    use super::*;

    #[tokio::test]
    async fn test_not_above_limit() {
        let joined = join(
            [
                sleep(Duration::from_millis(10)),
                sleep(Duration::from_millis(20)),
            ],
            10,
        );

        let timeout = tokio::time::timeout(Duration::from_millis(30), joined);
        timeout.await.expect("future timed out before completion");
    }

    #[tokio::test]
    async fn test_above_limit_no_concurrency() {
        let completed = Arc::new(AtomicBool::new(false));
        let run = |expected: bool| {
            let completed = completed.clone();
            async move {
                let loaded = completed.load(Ordering::SeqCst);
                assert_eq!(loaded, expected);
                sleep(Duration::from_millis(10)).await;
                completed.store(true, Ordering::SeqCst);
            }
        };

        join([run(false), run(true)], 1).await;
    }

    #[tokio::test]
    async fn test_above_limit() {
        let (tx, rx) = std::sync::mpsc::channel();
        let record = |id: usize, millis: u64| {
            let tx = tx.clone();
            async move {
                tx.send(format!("s{id}")).unwrap();
                sleep(Duration::from_millis(millis)).await;
                tx.send(format!("e{id}")).unwrap();
            }
        };

        join(
            [record(0, 10), record(1, 25), record(2, 50), record(3, 50)],
            2,
        )
        .await;

        let mut order = rx.into_iter();

        // First two futures are polled concurrently.
        assert_eq!("s0", order.next().unwrap());
        assert_eq!("s1", order.next().unwrap());

        // Next the first future resolves, causing use to poll the third.
        assert_eq!("e0", order.next().unwrap());
        assert_eq!("s2", order.next().unwrap());

        // Our second future has now resolved, so we can start the last future.
        assert_eq!("e1", order.next().unwrap());
        assert_eq!("s3", order.next().unwrap());

        // Finally, we wait for the last futures to resolve.
        assert_eq!("e2", order.next().unwrap());
        assert_eq!("e3", order.next().unwrap());
    }
}
