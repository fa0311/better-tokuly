//! The bounded queue of frames waiting to be written to one client.
//!
//! Fan-out never waits for a client: [`Outbox::push`] only takes a short lock.
//! What happens when a client reads too slowly depends on the frame:
//!
//! - droppable frames (friend positions) push out the oldest droppable frame;
//! - reliable frames must not be lost, so a client whose backlog exceeds the
//!   limits is disconnected and resynchronizes when it reconnects.
use std::collections::VecDeque;

use bytes::Bytes;
use parking_lot::Mutex;
use tokio::sync::Notify;
use tokio_util::sync::CancellationToken;

use crate::frames::Delivery;

/// Why the gateway ends a connection. The code is used as the WebTransport
/// session close code and the WebSocket close code (4000–4999 is the range
/// reserved for applications); the label travels as the close reason.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CloseReason {
    /// The client went away or closed the connection itself.
    PeerClosed,
    /// The gateway is shutting down; reconnect (to another instance).
    ShuttingDown,
    /// A fatal `Error` frame was sent just before; it says what was wrong.
    Fatal,
    /// No frame arrived within the idle timeout.
    Idle,
    /// The client did not read fast enough.
    SlowConsumer,
}

impl CloseReason {
    pub(crate) fn code(self) -> u16 {
        match self {
            CloseReason::PeerClosed => 4000,
            CloseReason::ShuttingDown => 4001,
            CloseReason::Fatal => 4002,
            CloseReason::Idle => 4003,
            CloseReason::SlowConsumer => 4004,
        }
    }

    pub(crate) fn label(self) -> &'static str {
        match self {
            CloseReason::PeerClosed => "closed",
            CloseReason::ShuttingDown => "shutting down",
            CloseReason::Fatal => "fatal error",
            CloseReason::Idle => "idle timeout",
            CloseReason::SlowConsumer => "slow consumer",
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct OutboxLimits {
    pub reliable_frames: usize,
    pub reliable_bytes: usize,
    pub droppable_frames: usize,
}

/// What became of a pushed frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Pushed {
    Queued,
    /// Queued, at the price of the oldest droppable frame.
    ReplacedOldest,
    /// The reliable backlog is over its limit: the connection is now closing.
    Overflowed,
    /// The connection is already closing; the frame was discarded.
    Closed,
}

/// What the writer should do next.
#[derive(Debug)]
pub(crate) enum Next {
    Frame(Delivery, Bytes),
    /// Nothing more will come: close the transport.
    Close(CloseReason),
}

#[derive(Default)]
struct State {
    reliable: VecDeque<Bytes>,
    reliable_bytes: usize,
    droppable: VecDeque<Bytes>,
    closing: Option<CloseReason>,
}

pub(crate) struct Outbox {
    limits: OutboxLimits,
    state: Mutex<State>,
    /// Wakes the single writer.
    wake: Notify,
    /// Cancelled once the connection is closing, for everyone else to observe.
    closed: CancellationToken,
    /// Cancelled when it is closing without flushing (abort or overflow):
    /// even a write in flight is not worth finishing.
    aborted: CancellationToken,
}

impl Outbox {
    pub(crate) fn new(limits: OutboxLimits) -> Self {
        Self {
            limits,
            state: Mutex::new(State::default()),
            wake: Notify::new(),
            closed: CancellationToken::new(),
            aborted: CancellationToken::new(),
        }
    }

    /// Queues a frame without ever blocking.
    pub(crate) fn push(&self, delivery: Delivery, frame: Bytes) -> Pushed {
        let mut state = self.state.lock();
        if state.closing.is_some() {
            return Pushed::Closed;
        }
        let outcome = match delivery {
            Delivery::Droppable => {
                state.droppable.push_back(frame);
                if state.droppable.len() > self.limits.droppable_frames {
                    state.droppable.pop_front();
                    Pushed::ReplacedOldest
                } else {
                    Pushed::Queued
                }
            }
            Delivery::Reliable => {
                state.reliable_bytes += frame.len();
                state.reliable.push_back(frame);
                if state.reliable.len() > self.limits.reliable_frames || state.reliable_bytes > self.limits.reliable_bytes {
                    // The client is hopelessly behind: nothing queued is worth sending anymore.
                    Self::discard(&mut state);
                    state.closing = Some(CloseReason::SlowConsumer);
                    Pushed::Overflowed
                } else {
                    Pushed::Queued
                }
            }
        };
        drop(state);
        if outcome == Pushed::Overflowed {
            self.aborted.cancel();
            self.closed.cancel();
        }
        self.wake.notify_one();
        outcome
    }

    /// Starts closing: reliable frames already queued (such as the `Error`
    /// explaining why) are still written, then the writer closes the
    /// transport. The first reason given wins.
    pub(crate) fn close(&self, reason: CloseReason) {
        {
            let mut state = self.state.lock();
            if state.closing.is_none() {
                state.closing = Some(reason);
                state.droppable.clear();
            }
        }
        self.closed.cancel();
        self.wake.notify_one();
    }

    /// Closes at once, discarding everything queued (the client is gone or
    /// not reading, so there is nobody to flush to).
    pub(crate) fn abort(&self, reason: CloseReason) {
        {
            let mut state = self.state.lock();
            Self::discard(&mut state);
            state.closing.get_or_insert(reason);
        }
        self.aborted.cancel();
        self.closed.cancel();
        self.wake.notify_one();
    }

    fn discard(state: &mut State) {
        state.reliable.clear();
        state.reliable_bytes = 0;
        state.droppable.clear();
    }

    /// Resolves once the connection is closing, whoever decided so.
    pub(crate) async fn closed(&self) {
        self.closed.cancelled().await;
    }

    /// Resolves once the connection is closing without a flush.
    pub(crate) async fn aborted(&self) {
        self.aborted.cancelled().await;
    }

    pub(crate) fn is_aborted(&self) -> bool {
        self.aborted.is_cancelled()
    }

    pub(crate) fn close_reason(&self) -> Option<CloseReason> {
        self.state.lock().closing
    }

    /// The next frame to write: reliable frames first, since they are the
    /// ones whose backlog gets a client disconnected. Meant for one writer.
    pub(crate) async fn next(&self) -> Next {
        loop {
            {
                let mut state = self.state.lock();
                if let Some(frame) = state.reliable.pop_front() {
                    state.reliable_bytes -= frame.len();
                    return Next::Frame(Delivery::Reliable, frame);
                }
                if let Some(reason) = state.closing {
                    return Next::Close(reason);
                }
                if let Some(frame) = state.droppable.pop_front() {
                    return Next::Frame(Delivery::Droppable, frame);
                }
            }
            // `notify_one` stores a permit, so a push between the check above
            // and this await is not missed.
            self.wake.notified().await;
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    fn outbox(reliable_frames: usize, reliable_bytes: usize, droppable_frames: usize) -> Outbox {
        Outbox::new(OutboxLimits { reliable_frames, reliable_bytes, droppable_frames })
    }

    fn frame(text: &'static str) -> Bytes {
        Bytes::from_static(text.as_bytes())
    }

    async fn next_frame(outbox: &Outbox) -> (Delivery, Bytes) {
        match outbox.next().await {
            Next::Frame(delivery, bytes) => (delivery, bytes),
            Next::Close(reason) => panic!("unexpected close: {reason:?}"),
        }
    }

    #[tokio::test]
    async fn reliable_frames_keep_their_order_and_go_first() {
        let outbox = outbox(8, 1024, 8);
        assert_eq!(outbox.push(Delivery::Droppable, frame("pos")), Pushed::Queued);
        assert_eq!(outbox.push(Delivery::Reliable, frame("a")), Pushed::Queued);
        assert_eq!(outbox.push(Delivery::Reliable, frame("b")), Pushed::Queued);
        assert_eq!(next_frame(&outbox).await, (Delivery::Reliable, frame("a")));
        assert_eq!(next_frame(&outbox).await, (Delivery::Reliable, frame("b")));
        assert_eq!(next_frame(&outbox).await, (Delivery::Droppable, frame("pos")));
    }

    #[tokio::test]
    async fn droppable_overflow_discards_the_oldest() {
        let outbox = outbox(8, 1024, 2);
        assert_eq!(outbox.push(Delivery::Droppable, frame("1")), Pushed::Queued);
        assert_eq!(outbox.push(Delivery::Droppable, frame("2")), Pushed::Queued);
        assert_eq!(outbox.push(Delivery::Droppable, frame("3")), Pushed::ReplacedOldest);
        assert_eq!(next_frame(&outbox).await.1, frame("2"));
        assert_eq!(next_frame(&outbox).await.1, frame("3"));
        assert!(outbox.close_reason().is_none());
    }

    #[tokio::test]
    async fn reliable_overflow_closes_the_connection() {
        let by_count = outbox(2, 1024, 8);
        by_count.push(Delivery::Reliable, frame("a"));
        by_count.push(Delivery::Reliable, frame("b"));
        assert_eq!(by_count.push(Delivery::Reliable, frame("c")), Pushed::Overflowed);
        assert_eq!(by_count.close_reason(), Some(CloseReason::SlowConsumer));
        by_count.closed().await;
        by_count.aborted().await;
        // Nothing queued is delivered anymore, and later pushes are refused.
        assert!(matches!(by_count.next().await, Next::Close(CloseReason::SlowConsumer)));
        assert_eq!(by_count.push(Delivery::Reliable, frame("d")), Pushed::Closed);

        let by_bytes = outbox(100, 5, 8);
        assert_eq!(by_bytes.push(Delivery::Reliable, frame("abc")), Pushed::Queued);
        assert_eq!(by_bytes.push(Delivery::Reliable, frame("def")), Pushed::Overflowed);
    }

    #[tokio::test]
    async fn graceful_close_flushes_reliable_frames_only() {
        let outbox = outbox(8, 1024, 8);
        outbox.push(Delivery::Droppable, frame("pos"));
        outbox.push(Delivery::Reliable, frame("goodbye"));
        outbox.close(CloseReason::Fatal);
        outbox.close(CloseReason::Idle);
        outbox.closed().await;
        assert!(!outbox.is_aborted());
        assert_eq!(next_frame(&outbox).await.1, frame("goodbye"));
        assert!(matches!(outbox.next().await, Next::Close(CloseReason::Fatal)));
    }

    #[tokio::test]
    async fn abort_discards_everything() {
        let outbox = outbox(8, 1024, 8);
        outbox.push(Delivery::Reliable, frame("never sent"));
        outbox.abort(CloseReason::PeerClosed);
        assert!(outbox.is_aborted());
        assert!(matches!(outbox.next().await, Next::Close(CloseReason::PeerClosed)));
    }

    #[tokio::test]
    async fn a_waiting_writer_is_woken_by_a_push() {
        let outbox = std::sync::Arc::new(outbox(8, 1024, 8));
        let writer = tokio::spawn({
            let outbox = outbox.clone();
            async move { next_frame(&outbox).await.1 }
        });
        tokio::time::sleep(Duration::from_millis(20)).await;
        outbox.push(Delivery::Reliable, frame("late"));
        assert_eq!(tokio::time::timeout(Duration::from_secs(1), writer).await.unwrap().unwrap(), frame("late"));
    }
}
