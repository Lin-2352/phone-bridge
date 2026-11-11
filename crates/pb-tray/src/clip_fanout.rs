//! Independent clipboard recipients; waiting for one must not delay another.

use std::future::Future;
use tokio::task::JoinError;

pub async fn send_all<T, F, Fut>(targets: Vec<T>, mut send: F) -> Vec<JoinError>
where
    T: Send + 'static,
    F: FnMut(T) -> Fut + Send,
    Fut: Future<Output = ()> + Send + 'static,
{
    let mut tasks = tokio::task::JoinSet::new();
    for target in targets {
        tasks.spawn(send(target));
    }
    let mut errors = Vec::new();
    while let Some(result) = tasks.join_next().await {
        if let Err(error) = result {
            errors.push(error);
        }
    }
    errors
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    };
    use std::time::Duration;
    use tokio::sync::{mpsc, Notify};

    #[tokio::test]
    async fn an_unreachable_first_phone_does_not_delay_other_phones() {
        let blocked = Arc::new(Notify::new());
        let release = blocked.clone();
        let (tx, mut rx) = mpsc::unbounded_channel();
        let job = tokio::spawn(send_all(vec![0, 1, 2], move |target| {
            let blocked = blocked.clone();
            let tx = tx.clone();
            async move {
                if target == 0 {
                    blocked.notified().await;
                }
                tx.send(target).unwrap();
            }
        }));
        let before = tokio::time::timeout(Duration::from_secs(1), async {
            let mut healthy = vec![rx.recv().await.unwrap(), rx.recv().await.unwrap()];
            healthy.sort();
            healthy
        })
        .await
        .ok();
        release.notify_one();
        assert!(job.await.unwrap().is_empty());
        assert_eq!(
            before,
            Some(vec![1, 2]),
            "healthy phones must complete before the blocked phone is released"
        );
    }

    #[tokio::test]
    async fn a_panicking_sender_does_not_cancel_healthy_senders() {
        let healthy = Arc::new(AtomicUsize::new(0));
        let seen = healthy.clone();
        let job = tokio::spawn(send_all(vec![0, 1], move |target| {
            let healthy = healthy.clone();
            async move {
                if target == 0 {
                    panic!("test sender failed");
                }
                healthy.fetch_add(1, Ordering::Relaxed);
            }
        }));
        let outcome = job.await;
        assert!(outcome.is_ok(), "one sender must not panic the whole batch");
        assert_eq!(outcome.unwrap().len(), 1);
        assert_eq!(seen.load(Ordering::Relaxed), 1);
    }

    #[tokio::test]
    async fn an_empty_recipient_list_sends_nothing() {
        assert!(
            send_all(Vec::<u8>::new(), |_| async { panic!("no recipients") })
                .await
                .is_empty()
        );
    }
}
