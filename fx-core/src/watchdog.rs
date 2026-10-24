//! Heartbeat watch for an out-of-process helper.
//!
//! [`Frame`] is a length-prefixed buffer. [`Watch`] decides when the helper has
//! gone too long without a beat. Nothing in this module starts a process.

use std::fmt;
use std::time::{Duration, Instant};

/// Longest accepted frame payload, in bytes.
///
/// A length prefix above this is rejected. Validation bound, not a performance
/// budget.
pub const MAX_FRAME_LEN: usize = 1_048_576;

/// One helper message: a payload without its length prefix.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Frame {
    bytes: Vec<u8>,
}

impl Frame {
    /// Builds a frame whose payload is at most [`MAX_FRAME_LEN`] bytes.
    pub fn new(bytes: impl Into<Vec<u8>>) -> Result<Self, FrameTooLarge> {
        let bytes = bytes.into();
        if bytes.len() > MAX_FRAME_LEN {
            return Err(FrameTooLarge);
        }
        Ok(Self { bytes })
    }

    /// Payload bytes. The length prefix is not included.
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    /// Little-endian `u32` payload length, then the payload.
    pub fn encode(&self) -> Vec<u8> {
        let len = u32::try_from(self.bytes.len()).expect("frame length fits in u32");
        let mut out = Vec::with_capacity(4 + self.bytes.len());
        out.extend_from_slice(&len.to_le_bytes());
        out.extend_from_slice(&self.bytes);
        out
    }

    /// Reads one frame from the front of `buffer`.
    ///
    /// [`TryDecode::Ready`] is the frame and how many bytes it occupied.
    /// [`TryDecode::Pending`] means the buffer is shorter than the frame the
    /// prefix describes. [`TryDecode::Error`] means the length is above
    /// [`MAX_FRAME_LEN`].
    pub fn decode(buffer: &[u8]) -> TryDecode {
        if buffer.len() < 4 {
            return TryDecode::Pending;
        }
        let mut prefix = [0u8; 4];
        prefix.copy_from_slice(&buffer[..4]);
        let Ok(len) = usize::try_from(u32::from_le_bytes(prefix)) else {
            return TryDecode::Error;
        };
        if len > MAX_FRAME_LEN {
            return TryDecode::Error;
        }
        let total = 4 + len;
        if buffer.len() < total {
            return TryDecode::Pending;
        }
        TryDecode::Ready {
            frame: Frame {
                bytes: buffer[4..total].to_vec(),
            },
            consumed: total,
        }
    }
}

/// Payload longer than [`MAX_FRAME_LEN`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FrameTooLarge;

impl fmt::Display for FrameTooLarge {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "frame length is above {MAX_FRAME_LEN} bytes")
    }
}

impl std::error::Error for FrameTooLarge {}

/// Outcome of [`Frame::decode`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TryDecode {
    /// A complete frame and the number of buffer bytes it used.
    Ready { frame: Frame, consumed: usize },
    /// The buffer does not yet hold a whole frame.
    Pending,
    /// The length prefix is above [`MAX_FRAME_LEN`].
    Error,
}

/// Deadline for beats from a helper.
///
/// `new` opens the first window at `started`. A helper that never beats still
/// expires when that window ends. [`Watch::beat`] moves the window to that
/// beat.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Watch {
    pub deadline: Duration,
    /// Last recorded beat. `None` until [`Watch::beat`].
    pub last_beat: Option<Instant>,
    started: Instant,
}

impl Watch {
    /// Starts the first window at `started`.
    pub fn new(deadline: Duration, started: Instant) -> Self {
        Self {
            deadline,
            last_beat: None,
            started,
        }
    }

    /// Records a beat at `now`.
    pub fn beat(&mut self, now: Instant) {
        self.last_beat = Some(now);
    }

    /// Whether the open window has elapsed.
    ///
    /// The instant `anchor + deadline` is already expired, matching the
    /// enumeration clock. The anchor is `last_beat` after a beat, and `started`
    /// when no beat has arrived.
    pub fn expired(&self, now: Instant) -> bool {
        let anchor = self.last_beat.unwrap_or(self.started);
        match anchor.checked_add(self.deadline) {
            // The deadline instant itself is expired. An instant the clock cannot
            // represent has not arrived.
            Some(deadline) => now >= deadline,
            None => false,
        }
    }

    /// Whether the supervisor should stop the helper. This is [`Watch::expired`].
    pub fn should_stop(&self, now: Instant) -> bool {
        self.expired(now)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ready(buffer: &[u8]) -> (Frame, usize) {
        match Frame::decode(buffer) {
            TryDecode::Ready { frame, consumed } => (frame, consumed),
            TryDecode::Pending => panic!("buffer should hold a frame"),
            TryDecode::Error => panic!("frame length should be legal"),
        }
    }

    #[test]
    fn frame_round_trip() {
        let frame = Frame::new(b"helper-ok".as_slice()).unwrap();
        let encoded = frame.encode();
        assert_eq!(&encoded[..4], &9u32.to_le_bytes());
        assert_eq!(&encoded[4..], b"helper-ok");

        let (decoded, consumed) = ready(&encoded);
        assert_eq!(decoded, frame);
        assert_eq!(decoded.bytes(), b"helper-ok");
        assert_eq!(consumed, encoded.len());

        let mut extra = encoded.clone();
        extra.push(0xFF);
        let (_, consumed) = ready(&extra);
        assert_eq!(consumed, encoded.len());

        let empty = Frame::new(Vec::<u8>::new()).unwrap();
        let encoded = empty.encode();
        assert_eq!(encoded, [0, 0, 0, 0]);
        let (decoded, consumed) = ready(&encoded);
        assert_eq!(decoded.bytes(), b"");
        assert_eq!(consumed, 4);
    }

    #[test]
    fn rejects_an_oversized_length() {
        assert!(Frame::new(vec![0u8; MAX_FRAME_LEN]).is_ok());
        assert_eq!(
            Frame::new(vec![0u8; MAX_FRAME_LEN + 1]).unwrap_err(),
            FrameTooLarge
        );

        let max = Frame::new(vec![0u8; MAX_FRAME_LEN]).unwrap();
        let (decoded, consumed) = ready(&max.encode());
        assert_eq!(decoded.bytes().len(), MAX_FRAME_LEN);
        assert_eq!(consumed, 4 + MAX_FRAME_LEN);

        let mut too_big = u32::try_from(MAX_FRAME_LEN + 1)
            .unwrap()
            .to_le_bytes()
            .to_vec();
        assert!(matches!(Frame::decode(&too_big), TryDecode::Error));
        too_big.extend_from_slice(&[1, 2, 3, 4]);
        assert!(matches!(Frame::decode(&too_big), TryDecode::Error));
        assert!(matches!(
            Frame::decode(&u32::MAX.to_le_bytes()),
            TryDecode::Error
        ));
    }

    #[test]
    fn pending_on_a_short_buffer() {
        assert!(matches!(Frame::decode(&[]), TryDecode::Pending));
        assert!(matches!(Frame::decode(&[1, 0, 0]), TryDecode::Pending));

        let mut short = 4u32.to_le_bytes().to_vec();
        short.extend_from_slice(&[1, 2, 3]);
        assert!(matches!(Frame::decode(&short), TryDecode::Pending));

        let mut max_header = u32::try_from(MAX_FRAME_LEN).unwrap().to_le_bytes().to_vec();
        assert!(matches!(Frame::decode(&max_header), TryDecode::Pending));
        max_header.push(0);
        assert!(matches!(Frame::decode(&max_header), TryDecode::Pending));
    }

    #[test]
    fn beat_extends_the_deadline() {
        let started = Instant::now();
        let deadline = Duration::from_millis(100);
        let mut watch = Watch::new(deadline, started);
        assert_eq!(watch.last_beat, None);

        let beat_at = started + Duration::from_millis(80);
        watch.beat(beat_at);
        assert_eq!(watch.last_beat, Some(beat_at));
        assert!(!watch.expired(started + deadline));
        assert!(!watch.should_stop(beat_at + Duration::from_millis(99)));
        assert!(watch.expired(beat_at + deadline));
        assert!(watch.should_stop(beat_at + deadline));
    }

    #[test]
    fn silence_past_the_deadline_expires() {
        let started = Instant::now();
        let deadline = Duration::from_millis(100);
        let watch = Watch::new(deadline, started);
        assert_eq!(watch.last_beat, None);
        assert!(!watch.expired(started));
        assert!(!watch.should_stop(started + Duration::from_millis(99)));
        assert!(watch.expired(started + deadline));
        assert!(watch.should_stop(started + deadline));
        assert!(watch.expired(started + deadline + Duration::from_millis(1)));
    }

    #[test]
    fn beat_after_expiry_opens_a_new_window() {
        let started = Instant::now();
        let deadline = Duration::from_millis(100);
        let mut watch = Watch::new(deadline, started);
        let late = started + deadline + Duration::from_millis(40);

        assert!(watch.expired(late));
        assert!(watch.should_stop(late));
        watch.beat(late);
        assert_eq!(watch.last_beat, Some(late));
        assert!(!watch.expired(late));
        assert!(!watch.should_stop(late));
        assert!(!watch.expired(late + Duration::from_millis(99)));
        assert!(!watch.should_stop(late + deadline - Duration::from_millis(1)));
        assert!(watch.expired(late + deadline));
        assert!(watch.should_stop(late + deadline));
    }

    #[test]
    fn a_deadline_the_clock_cannot_represent_is_not_expired() {
        let started = Instant::now();
        let watch = Watch::new(Duration::MAX, started);
        assert!(!watch.expired(started));
        assert!(!watch.should_stop(started));
        let later = started + Duration::from_secs(3600);
        assert!(!watch.expired(later));
        assert!(!watch.should_stop(later));
        assert_eq!(watch.expired(later), watch.should_stop(later));
    }
}
