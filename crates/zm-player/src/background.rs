//! Send-only resource work belongs on Tokio's workers, not the AVM/UI executor.
//! Dropping the local waiter cancels its worker as well (including on logout).
use std::future::Future;
use tokio::task::{JoinError, JoinHandle};

struct CancelOnDrop<T>(JoinHandle<T>);

impl<T> Drop for CancelOnDrop<T> {
    fn drop(&mut self) {
        self.0.abort();
    }
}

pub(crate) async fn run<T: Send + 'static>(
    work: impl Future<Output = T> + Send + 'static,
) -> Result<T, JoinError> {
    let mut task = CancelOnDrop(tokio::spawn(work));
    (&mut task.0).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{pin::pin, task::Poll};
    use tokio::sync::oneshot;

    #[tokio::test(flavor = "multi_thread", worker_threads = 1)]
    async fn work_runs_off_the_calling_thread() {
        let caller = std::thread::current().id();
        let worker = run(async { std::thread::current().id() }).await.unwrap();
        assert_ne!(caller, worker);
    }

    #[tokio::test]
    async fn dropping_waiter_cancels_pending_work() {
        struct NotifyOnDrop(Option<oneshot::Sender<()>>);
        impl Drop for NotifyOnDrop {
            fn drop(&mut self) {
                let _ = self.0.take().unwrap().send(());
            }
        }
        let (started_tx, started_rx) = oneshot::channel();
        let (dropped_tx, dropped_rx) = oneshot::channel();
        {
            let mut work = pin!(run(async move {
                let _guard = NotifyOnDrop(Some(dropped_tx));
                started_tx.send(()).unwrap();
                std::future::pending::<()>().await;
            }));
            std::future::poll_fn(|cx| {
                assert!(work.as_mut().poll(cx).is_pending());
                Poll::Ready(())
            })
            .await;
            started_rx.await.unwrap();
        }
        tokio::time::timeout(std::time::Duration::from_secs(2), dropped_rx)
            .await
            .unwrap()
            .unwrap();
    }

    #[tokio::test]
    async fn worker_errors_are_returned_to_the_waiter() {
        assert_eq!(
            run(async { Err::<(), _>("resource unavailable") })
                .await
                .unwrap(),
            Err("resource unavailable")
        );
    }
}
