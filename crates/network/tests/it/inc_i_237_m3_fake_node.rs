//! INC-I-237 M3 — a fake node loop on a real `NetworkService` for the non-tx
//! topics: attestations are judged decode + Ed25519 + membership (R2), other
//! deferred messages are judged by `judge_*` or kept for the test to release;
//! txs are consumed slowly (a bulk-lane flood) and Ignored.
//!
//! OUTPUT CONTRACT: N/A — fixture file (helpers only, asserts nothing).
//! INPUT PARTITIONS: N/A — fixture file.
#![allow(dead_code)] // each consumer uses a subset

use std::collections::HashSet;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crypto::{Hash, KeyPair, PublicKey, Signature};
use doli_core::{Attestation, ProducerAnnouncement, ProducerBloomFilter, ProducerGSet};
use network::service::{send_gossip_verdict, Verdict};
use network::{GossipHandle, NetworkCommand, NetworkEvent, NetworkService, PeerId};
use tokio::sync::mpsc;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Attestation,
    Vote,
    Producers,
    Digest,
    Tx,
}

/// One gossip event the fake node received.
#[derive(Clone, Debug)]
pub struct Got {
    pub kind: Kind,
    /// Attestation / vote raw bytes; producer sets: the announced pubkeys.
    pub data: Vec<u8>,
    /// NewAttestation source / ProducerDigestReceived peer_id.
    pub from: Option<PeerId>,
    /// Topic name stamped in the event's verdict handle (`None` = unstamped).
    pub topic: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    /// Judge every deferred message and send the verdict at once.
    Judge,
    /// Keep every deferred handle; the test releases it.
    Keep,
}

pub type Kept = Arc<Mutex<Vec<(Kind, Vec<u8>, GossipHandle)>>>;

pub struct Fake3 {
    pub got: Arc<Mutex<Vec<Got>>>,
    pub kept: Kept,
    /// receive -> verdict-sent latency of every attestation verdict, in ms.
    pub attest_verdict_ms: Arc<Mutex<Vec<u128>>>,
    pub mode: Arc<Mutex<Mode>>,
    pub cmd: mpsc::Sender<NetworkCommand>,
    pub peer_id: PeerId,
    pub peers: Arc<tokio::sync::RwLock<std::collections::HashMap<PeerId, network::peer::PeerInfo>>>,
}

impl Fake3 {
    pub async fn peer_count(&self) -> usize {
        self.peers.read().await.len()
    }

    pub fn set(&self, m: Mode) {
        *self.mode.lock().unwrap() = m;
    }

    pub fn count(&self, kind: Kind, data: &[u8]) -> usize {
        self.got
            .lock()
            .unwrap()
            .iter()
            .filter(|g| g.kind == kind && g.data == data)
            .count()
    }

    pub fn count_kind(&self, kind: Kind) -> usize {
        self.got
            .lock()
            .unwrap()
            .iter()
            .filter(|g| g.kind == kind)
            .count()
    }

    /// Release every kept handle of `kind` whose bytes are `data` with `v`.
    pub fn release(&self, kind: Kind, data: &[u8], v: Verdict) -> usize {
        let mut kept = self.kept.lock().unwrap();
        let mut n = 0;
        kept.retain(|(k, d, h)| {
            if *k == kind && d == data {
                send_gossip_verdict(&self.cmd, h.clone(), v);
                n += 1;
                false
            } else {
                true
            }
        });
        n
    }
}

/// The topic a handle reports its verdict on, read from its Debug form
/// (the origin is crate-private); `None` for an unstamped handle.
pub fn origin_topic(h: &GossipHandle) -> Option<String> {
    let s = format!("{h:?}");
    let rest = s.split("origin: Some((").nth(1)?;
    Some(rest.split(',').next()?.trim().to_string())
}

/// Attestation verdict R2: decode + Ed25519 + attester in `members`.
pub fn judge_attestation(data: &[u8], members: &HashSet<PublicKey>) -> Verdict {
    match Attestation::from_bytes(data) {
        Some(a) if a.verify().is_ok() && members.contains(&a.attester) => Verdict::Accept,
        _ => Verdict::Ignore,
    }
}

/// Spawn the fake node; `tx_delay` is the per-tx-event processing time.
pub fn spawn_fake3(
    mut svc: NetworkService,
    mode: Mode,
    members: HashSet<PublicKey>,
    tx_delay: Duration,
) -> Fake3 {
    let fake = Fake3 {
        got: Arc::default(),
        kept: Arc::default(),
        attest_verdict_ms: Arc::default(),
        mode: Arc::new(Mutex::new(mode)),
        cmd: svc.command_sender(),
        peer_id: *svc.local_peer_id(),
        peers: svc.peers_arc(),
    };
    let (got, kept, lat, md, cmd) = (
        fake.got.clone(),
        fake.kept.clone(),
        fake.attest_verdict_ms.clone(),
        fake.mode.clone(),
        fake.cmd.clone(),
    );
    tokio::spawn(async move {
        while let Some(ev) = svc.next_event().await {
            let (kind, data, from, handle) = match ev {
                NetworkEvent::NewAttestation(d, src, h) => (Kind::Attestation, d, Some(src), h),
                NetworkEvent::NewVote(d, h) => (Kind::Vote, d, None, h),
                NetworkEvent::ProducerAnnouncementsReceived(anns, h) => {
                    let keys = anns
                        .iter()
                        .flat_map(|a| a.pubkey.as_bytes().to_vec())
                        .collect();
                    (Kind::Producers, keys, None, h)
                }
                NetworkEvent::ProducerDigestReceived {
                    peer_id, handle, ..
                } => {
                    got.lock().unwrap().push(Got {
                        kind: Kind::Digest,
                        data: Vec::new(),
                        from: Some(peer_id),
                        topic: origin_topic(&handle),
                    });
                    continue;
                }
                NetworkEvent::NewTransaction(_, h) => {
                    got.lock().unwrap().push(Got {
                        kind: Kind::Tx,
                        data: Vec::new(),
                        from: None,
                        topic: origin_topic(&h),
                    });
                    tokio::time::sleep(tx_delay).await;
                    send_gossip_verdict(&cmd, h, Verdict::Ignore);
                    continue;
                }
                _ => continue,
            };
            got.lock().unwrap().push(Got {
                kind,
                data: data.clone(),
                from,
                topic: origin_topic(&handle),
            });
            let mode = *md.lock().unwrap();
            match (mode, kind) {
                (Mode::Keep, _) => kept.lock().unwrap().push((kind, data, handle)),
                (Mode::Judge, Kind::Attestation) => {
                    let v = judge_attestation(&data, &members);
                    lat.lock()
                        .unwrap()
                        .push(handle.recv_at.elapsed().as_millis());
                    send_gossip_verdict(&cmd, handle, v);
                }
                (Mode::Judge, _) => {
                    send_gossip_verdict(&cmd, handle, Verdict::Accept);
                }
            }
        }
    });
    fake
}

pub fn attestation(kp: &KeyPair, block_tag: u8, slot: u32) -> Attestation {
    let block = Hash::from_bytes([block_tag; 32]);
    Attestation::new(block, slot, 1, 1, kp.private_key(), *kp.public_key())
}

/// Same signed fields (block, slot, attester), garbage Ed25519 signature.
pub fn forge(a: &Attestation) -> Attestation {
    let mut f = a.clone();
    f.signature = Signature::from_bytes([0x5A; 64]);
    f
}

/// A JSON vote in the wire shape the RPC and the updater publish.
pub fn vote_json(kp: &KeyPair, timestamp: u64) -> Vec<u8> {
    let msg = format!("6.43.3:approve:{timestamp}");
    let sig = crypto::signature::sign(msg.as_bytes(), kp.private_key());
    format!(
        "{{\"version\":\"6.43.3\",\"vote\":\"approve\",\"producerId\":\"{}\",\"timestamp\":{timestamp},\"signature\":\"{}\"}}",
        kp.public_key().to_hex(),
        sig.to_hex()
    )
    .into_bytes()
}

pub fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs()
}

pub fn announcement(kp: &KeyPair) -> ProducerAnnouncement {
    ProducerAnnouncement::new(kp, 99, 1, Hash::ZERO)
}

/// The pubkeys of `anns` as the fake node records a producer set.
pub fn keys(anns: &[ProducerAnnouncement]) -> Vec<u8> {
    anns.iter()
        .flat_map(|a| a.pubkey.as_bytes().to_vec())
        .collect()
}

/// A bloom digest that holds `kps`.
pub fn digest(kps: &[&KeyPair]) -> ProducerBloomFilter {
    let mut g = ProducerGSet::new(99, Hash::ZERO);
    for kp in kps {
        let _ = g.merge_one(announcement(kp));
    }
    g.to_bloom_filter()
}

/// Poll `probe` every 50 ms until it is true or `within` elapses.
pub async fn wait_for(within: Duration, mut probe: impl FnMut() -> bool) -> bool {
    let deadline = Instant::now() + within;
    loop {
        if probe() {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}
