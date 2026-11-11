//! Newest-only, in-memory clipboard delivery state; no window or clipboard calls.

pub struct PendingClipboard<T> {
    latest: Option<(u32, T)>,
}

impl<T> PendingClipboard<T> {
    pub const fn new() -> Self {
        Self { latest: None }
    }
    pub fn replace(&mut self, sequence: u32, value: T) {
        self.latest = Some((sequence, value));
    }
    pub fn is_pending(&self) -> bool {
        self.latest.is_some()
    }
    pub fn clear(&mut self) {
        self.latest = None;
    }
    pub fn observe_local_copy(&mut self, sequence: u32) -> bool {
        if self.latest.as_ref().is_some_and(|(at, _)| *at != sequence) {
            self.clear();
            true
        } else {
            false
        }
    }
    pub fn try_write<E>(
        &mut self,
        mut write: impl FnMut(&T) -> Result<(), E>,
    ) -> Result<Option<T>, E> {
        let Some((_, value)) = self.latest.as_ref() else {
            return Ok(None);
        };
        write(value)?;
        Ok(self.latest.take().map(|(_, value)| value))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_busy_clipboard_retains_the_clip_until_a_successful_retry() {
        let mut pending = PendingClipboard::new();
        pending.replace(10, "text");
        assert_eq!(pending.try_write(|_| Err("busy")), Err("busy"));
        assert!(
            pending.is_pending(),
            "a busy Windows clipboard must not drop a received clip"
        );
        assert_eq!(pending.try_write(|_| Ok::<_, &str>(())), Ok(Some("text")));
        assert!(!pending.is_pending());
    }

    #[test]
    fn only_the_newest_clip_survives_busy_retries_and_unlock() {
        let mut pending = PendingClipboard::new();
        pending.replace(10, vec![1, 2]);
        let _ = pending.try_write(|_| Err("locked"));
        pending.replace(10, vec![3, 4]);
        let _ = pending.try_write(|_| Err("busy"));
        let mut writes = Vec::new();
        assert_eq!(
            pending.try_write(|bytes| {
                writes.push(bytes.clone());
                Ok::<_, &str>(())
            }),
            Ok(Some(vec![3, 4]))
        );
        assert_eq!(writes, vec![vec![3, 4]]);
    }

    #[test]
    fn a_newer_local_copy_cancels_the_stale_pending_clip() {
        let mut pending = PendingClipboard::new();
        pending.replace(u32::MAX, "remote");
        assert!(
            !pending.observe_local_copy(u32::MAX),
            "an older queued window message is not a new copy"
        );
        assert!(
            pending.observe_local_copy(0),
            "sequence wrap is still a new copy"
        );
        assert!(!pending.is_pending());
        assert_eq!(pending.try_write(|_| Ok::<_, &str>(())), Ok(None));
    }

    #[test]
    fn clearing_a_pending_clip_never_invokes_the_writer() {
        let mut pending = PendingClipboard::new();
        pending.replace(1, "remote");
        pending.clear();
        assert_eq!(
            pending.try_write::<()>(|_| panic!("nothing to write")),
            Ok(None)
        );
    }
}
