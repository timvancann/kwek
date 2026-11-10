//! How long since the person last touched the keyboard or mouse, anywhere on the machine:
//! what presence calls "at the computer".

use kwek_core::presence::IdleSource;

pub struct SystemIdle;

#[cfg(target_os = "macos")]
#[link(name = "CoreGraphics", kind = "framework")]
unsafe extern "C" {
    fn CGEventSourceSecondsSinceLastEventType(state: i32, event_type: u32) -> f64;
}

impl IdleSource for SystemIdle {
    #[cfg(target_os = "macos")]
    fn idle_secs(&self) -> u64 {
        // kCGEventSourceStateCombinedSessionState (0), kCGAnyInputEventType (~0).
        // SAFETY: a plain query with constant arguments; it reads no memory of ours.
        let secs = unsafe { CGEventSourceSecondsSinceLastEventType(0, u32::MAX) };
        if secs.is_finite() && secs >= 0.0 {
            secs as u64
        } else {
            0
        }
    }

    /// Not measured here yet: counts as always at the computer.
    #[cfg(not(target_os = "macos"))]
    fn idle_secs(&self) -> u64 {
        0
    }
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use super::*;

    #[test]
    fn reads_a_plausible_idle_time() {
        // Since boot at the most; a garbage read would be huge or the error fallback.
        assert!(SystemIdle.idle_secs() < 60 * 60 * 24 * 365);
    }
}
