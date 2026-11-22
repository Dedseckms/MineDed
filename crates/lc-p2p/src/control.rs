//! Bounded per-connection control mailboxes. Room snapshots replace older
//! pending snapshots without consuming the reliable message budget.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use tokio::sync::Notify;

use crate::{proto::ControlMsg, wire};

const RELIABLE_LIMIT: usize = 256;
// One voice state per speaker plus the room roster.
const SNAPSHOT_LIMIT: usize = 257;

#[derive(PartialEq, Eq)]
enum Snapshot {
    Welcome,
    Voice(Option<String>),
}

struct Line {
    text: String,
    snapshot: Option<Snapshot>,
}

#[derive(Default)]
struct Queue {
    lines: VecDeque<Line>,
    reliable: usize,
    snapshots: usize,
    closed: bool,
}

struct Shared {
    queue: Mutex<Queue>,
    ready: Arc<Notify>,
    senders: AtomicUsize,
}

pub(crate) struct Sender(Arc<Shared>);

pub(crate) struct Receiver {
    shared: Arc<Shared>,
    ready: Arc<Notify>,
}

pub(crate) fn channel() -> (Sender, Receiver) {
    let shared = Arc::new(Shared {
        queue: Mutex::new(Queue::default()),
        ready: Arc::new(Notify::new()),
        senders: AtomicUsize::new(1),
    });
    let rx = Receiver {
        shared: shared.clone(),
        ready: shared.ready.clone(),
    };
    (Sender(shared), rx)
}

impl Sender {
    /// Returns an error only when the bounded message budget is exhausted.
    /// Reliable lines are never discarded or overwritten by snapshots.
    pub(crate) fn try_send(&self, text: String) -> Result<(), ()> {
        let snapshot = match wire::decode(&text) {
            Ok(ControlMsg::Welcome { .. }) => Some(Snapshot::Welcome),
            Ok(ControlMsg::VoiceState { peer, .. }) => Some(Snapshot::Voice(peer)),
            _ => None,
        };
        let mut q = self.0.queue.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(key) = &snapshot {
            if let Some(i) = q
                .lines
                .iter()
                .position(|l| l.snapshot.as_ref() == Some(key))
            {
                if matches!(key, Snapshot::Welcome) {
                    // Keep the initial Welcome ahead of chat and voice
                    // snapshots during the guest's handshake.
                    q.lines[i].text = text;
                    return Ok(());
                }
                q.lines.remove(i);
            } else {
                if q.snapshots == SNAPSHOT_LIMIT {
                    return Err(());
                }
                q.snapshots += 1;
            }
        } else {
            if q.reliable == RELIABLE_LIMIT {
                return Err(());
            }
            q.reliable += 1;
        }
        q.lines.push_back(Line { text, snapshot });
        drop(q);
        self.0.ready.notify_one();
        Ok(())
    }
}

impl Drop for Sender {
    fn drop(&mut self) {
        if self.0.senders.fetch_sub(1, Ordering::AcqRel) == 1 {
            self.0
                .queue
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .closed = true;
            self.0.ready.notify_one();
        }
    }
}

impl Clone for Sender {
    fn clone(&self) -> Self {
        self.0.senders.fetch_add(1, Ordering::Relaxed);
        Self(self.0.clone())
    }
}

impl Receiver {
    pub(crate) async fn recv(&mut self) -> Option<String> {
        loop {
            let ready = self.ready.notified();
            {
                let mut q = self.shared.queue.lock().unwrap_or_else(|e| e.into_inner());
                if let Some(line) = q.lines.pop_front() {
                    if line.snapshot.is_some() {
                        q.snapshots -= 1;
                    } else {
                        q.reliable -= 1;
                    }
                    return Some(line.text);
                }
                if q.closed {
                    return None;
                }
            }
            ready.await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn initial_welcome_stays_ahead_of_chat_and_voice() {
        let (tx, mut rx) = channel();
        let welcome = |peers| {
            wire::encode(&ControlMsg::Welcome {
                peers,
                peer_names: vec![],
            })
            .unwrap()
        };
        tx.try_send(welcome(vec!["host".into()])).unwrap();
        let chat = wire::encode(&ControlMsg::Chat {
            id: "id".into(),
            text: "message".into(),
            sent_at: String::new(),
            from: None,
            from_name: None,
        })
        .unwrap();
        tx.try_send(chat.clone()).unwrap();
        tx.try_send(
            wire::encode(&ControlMsg::VoiceState {
                speaking: true,
                muted: false,
                peer: Some("new".into()),
            })
            .unwrap(),
        )
        .unwrap();
        let latest = welcome(vec!["host".into(), "new".into()]);
        tx.try_send(latest.clone()).unwrap();
        assert_eq!(rx.recv().await.unwrap(), latest);
        assert_eq!(rx.recv().await.unwrap(), chat);
        assert!(matches!(
            wire::decode(&rx.recv().await.unwrap()).unwrap(),
            ControlMsg::VoiceState { .. }
        ));
    }

    #[tokio::test]
    async fn snapshot_keys_are_bounded_and_last_sender_wakes_receiver() {
        let (tx, mut rx) = channel();
        for i in 0..=SNAPSHOT_LIMIT {
            let result = tx.try_send(
                wire::encode(&ControlMsg::VoiceState {
                    speaking: true,
                    muted: false,
                    peer: Some(i.to_string()),
                })
                .unwrap(),
            );
            assert_eq!(result.is_ok(), i < SNAPSHOT_LIMIT);
        }
        for _ in 0..SNAPSHOT_LIMIT {
            assert!(rx.recv().await.is_some());
        }
        let clone = tx.clone();
        drop(tx);
        let reader = tokio::spawn(async move { rx.recv().await });
        tokio::task::yield_now().await;
        drop(clone);
        assert!(
            tokio::time::timeout(std::time::Duration::from_secs(1), reader)
                .await
                .unwrap()
                .unwrap()
                .is_none()
        );
    }

    #[tokio::test]
    async fn snapshots_do_not_overflow_reliable_queue() {
        let (tx, mut rx) = channel();
        for nonce in 0..RELIABLE_LIMIT {
            tx.try_send(
                wire::encode(&ControlMsg::Ping {
                    nonce: nonce as u64,
                })
                .unwrap(),
            )
            .unwrap();
        }
        for i in 0..1000 {
            tx.try_send(
                wire::encode(&ControlMsg::Welcome {
                    peers: vec![i.to_string()],
                    peer_names: vec![],
                })
                .unwrap(),
            )
            .unwrap();
            tx.try_send(
                wire::encode(&ControlMsg::VoiceState {
                    speaking: i % 2 == 0,
                    muted: false,
                    peer: Some("peer".into()),
                })
                .unwrap(),
            )
            .unwrap();
        }
        assert!(tx
            .try_send(wire::encode(&ControlMsg::Bye).unwrap())
            .is_err());
        for nonce in 0..RELIABLE_LIMIT {
            assert_eq!(
                wire::decode(&rx.recv().await.unwrap()).unwrap(),
                ControlMsg::Ping {
                    nonce: nonce as u64
                }
            );
        }
        assert!(
            matches!(wire::decode(&rx.recv().await.unwrap()).unwrap(), ControlMsg::Welcome { peers, .. } if peers == ["999"])
        );
        assert!(matches!(
            wire::decode(&rx.recv().await.unwrap()).unwrap(),
            ControlMsg::VoiceState {
                speaking: false,
                ..
            }
        ));
        drop(tx);
        assert!(rx.recv().await.is_none());
    }
}
