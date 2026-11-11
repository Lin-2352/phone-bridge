//! Clipboard authority is per peer; an explicit text action may bypass the global sync switch.

pub struct Access {
    pub sharing: bool,
    pub peer: bool,
    pub paused: bool,
    pub images: bool,
}

impl Access {
    pub fn text(&self, explicit: bool) -> bool {
        !self.paused && self.peer && (self.sharing || explicit)
    }
    pub fn image(&self) -> bool {
        self.text(false) && self.images
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn disabled_peer_cannot_send_receive_or_use_explicit_text() {
        let a = Access {
            sharing: true,
            peer: false,
            paused: false,
            images: true,
        };
        assert!(!a.text(false));
        assert!(!a.text(true));
        assert!(!a.image());
    }
    #[test]
    fn text_images_and_manual_override_obey_their_independent_switches() {
        for sharing in [false, true] {
            for peer in [false, true] {
                for paused in [false, true] {
                    for images in [false, true] {
                        let a = Access {
                            sharing,
                            peer,
                            paused,
                            images,
                        };
                        assert_eq!(a.text(false), sharing && peer && !paused);
                        assert_eq!(a.text(true), peer && !paused);
                        assert_eq!(a.image(), sharing && peer && !paused && images);
                    }
                }
            }
        }
    }
    #[test]
    fn a_queued_copy_rechecks_revoked_permission_before_writing() {
        let mut pending = crate::clip_retry::PendingClipboard::new();
        pending.replace(1, "remote");
        let mut a = Access {
            sharing: true,
            peer: true,
            paused: false,
            images: true,
        };
        assert!(pending.try_write(|_| Err::<(), _>("busy")).is_err());
        a.peer = false;
        let mut writes = 0;
        let result = pending.try_write(|_| {
            if !a.text(false) {
                return Err("revoked");
            }
            writes += 1;
            Ok(())
        });
        assert_eq!(result, Err("revoked"));
        assert_eq!(writes, 0);
        pending.clear();
        assert!(!pending.is_pending());
    }
}
