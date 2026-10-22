//! End-to-end without audio devices: TxPipeline → in-memory
//! `DatagramTransport` → rx loop → Mixer.

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use lc_p2p::DatagramTransport;
use lc_types::AppResult;
use tokio::sync::{mpsc, Mutex as AsyncMutex};

use crate::dsp::{rms, FRAME};
use crate::engine::rx_loop;
use crate::mixer::Mixer;
use crate::tx::{Gate, TxControl, TxPipeline};

/// One side of an in-memory link; `send` lands in the other side's `recv`.
struct MockLink {
    me: String,
    tx: mpsc::UnboundedSender<(String, Vec<u8>)>,
    rx: AsyncMutex<mpsc::UnboundedReceiver<(String, Vec<u8>)>>,
}

fn pair() -> (Arc<MockLink>, Arc<MockLink>) {
    let (atx, arx) = mpsc::unbounded_channel();
    let (btx, brx) = mpsc::unbounded_channel();
    let a = MockLink {
        me: "peer-a".into(),
        tx: btx,
        rx: AsyncMutex::new(arx),
    };
    let b = MockLink {
        me: "peer-b".into(),
        tx: atx,
        rx: AsyncMutex::new(brx),
    };
    (Arc::new(a), Arc::new(b))
}

#[async_trait]
impl DatagramTransport for MockLink {
    async fn send(&self, _to: Option<&str>, payload: &[u8]) -> AppResult<()> {
        let _ = self.tx.send((self.me.clone(), payload.to_vec()));
        Ok(())
    }

    async fn recv(&self) -> Option<(String, Vec<u8>)> {
        self.rx.lock().await.recv().await
    }

    fn max_datagram_size(&self) -> usize {
        1200
    }
}

fn ctl() -> TxControl {
    TxControl {
        gate: Gate::Vad,
        ptt_pressed: false,
        self_muted: false,
        input_gain: 1.0,
        noise_suppression: false,
    }
}

fn input(k: usize, onset: usize) -> Vec<f32> {
    (0..FRAME)
        .map(|i| {
            let n = k * FRAME + i;
            if n < onset {
                0.0
            } else {
                0.3 * (n as f32 * 0.05).sin()
            }
        })
        .collect()
}

async fn settle() {
    for _ in 0..8 {
        tokio::task::yield_now().await;
    }
}

#[tokio::test]
async fn loopback_end_to_end_latency() {
    let (a, b) = pair();
    let mixer = Arc::new(Mutex::new(Mixer::new()));
    let rx = tokio::spawn(rx_loop(b.clone(), mixer.clone()));

    let mut tx = TxPipeline::new(48_000, 3, 0).unwrap();
    let onset = 10 * FRAME + 300;
    let mut out = [0f32; FRAME];
    let mut first_out: Option<usize> = None;
    let mut sent = 0;
    let mut energy = Vec::new();
    for k in 0..40 {
        tx.push_samples(&input(k, onset));
        let frame = tx.next_frame(&ctl()).unwrap();
        if let Some(d) = frame.datagram {
            a.send(None, &d).await.unwrap();
            sent += 1;
        }
        settle().await;
        mixer.lock().unwrap().mix(&mut out);
        energy.push(rms(&out));
        if first_out.is_none() {
            if let Some(i) = out.iter().position(|s| s.abs() > 0.05) {
                // Output of step k starts playing when input frame k is complete.
                first_out = Some((k + 1) * FRAME + i);
            }
        }
    }
    assert!(sent >= 29, "sent {sent}");
    let latency = first_out.expect("audio arrived") - onset;
    let ms = latency as f32 / 48.0;
    println!(
        "loopback latency (framing + opus {} smp + jitter): {ms:.1} ms",
        tx.codec_delay()
    );
    assert!(ms < 80.0, "{ms} ms");
    // Steady speech after the jitter buffer filled.
    assert!(energy[15..40].iter().all(|e| *e > 0.1));
    rx.abort();
}

#[tokio::test]
async fn loopback_survives_loss_and_reordering() {
    let (a, b) = pair();
    let mixer = Arc::new(Mutex::new(Mixer::new()));
    let rx = tokio::spawn(rx_loop(b.clone(), mixer.clone()));
    let mut tx = TxPipeline::new(48_000, 1, 65_530).unwrap();
    let mut out = [0f32; FRAME];
    let mut held: Option<Vec<u8>> = None;
    let mut quiet_frames = 0;
    let mut total = 0.0;
    for k in 0..60 {
        tx.push_samples(&input(k, 0));
        let d = tx.next_frame(&ctl()).unwrap().datagram.unwrap();
        if k % 7 == 3 {
            // lost
        } else if k % 5 == 0 && held.is_none() {
            held = Some(d); // delayed by one frame → reordered
        } else {
            a.send(None, &d).await.unwrap();
            if k % 3 == 0 {
                a.send(None, &d).await.unwrap(); // duplicate
            }
            if let Some(h) = held.take() {
                a.send(None, &h).await.unwrap();
            }
        }
        settle().await;
        mixer.lock().unwrap().mix(&mut out);
        if k > 5 {
            let e = rms(&out);
            total += e;
            if e < 0.01 {
                quiet_frames += 1;
            }
        }
    }
    // 8 losses are concealed (PLC/FEC, attenuated), not dropouts.
    assert!(quiet_frames <= 2, "{quiet_frames} quiet frames");
    assert!(total / 54.0 > 0.15, "mean rms {}", total / 54.0);
    let act = mixer.lock().unwrap().activity();
    assert_eq!(act.len(), 1);
    assert_eq!(act[0].peer_id, "peer-a");
    rx.abort();
}

#[tokio::test]
async fn rx_loop_ends_when_room_closes() {
    let (a, b) = pair();
    let mixer = Arc::new(Mutex::new(Mixer::new()));
    drop(a); // sender side gone → recv returns None
    rx_loop(b, mixer).await;
}
