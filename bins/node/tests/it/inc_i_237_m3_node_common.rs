//! INC-I-237 M3 — node fixtures: a test node whose producer set holds one
//! member, signed attestations / votes / GSet announcements, a live
//! A -> B(node) -> C line on 127.0.0.1, and a tracing capture for the vendor
//! duplicate-publish refusal.
//!
//! OUTPUT CONTRACT: N/A — fixture file (helpers only, asserts nothing).
//! INPUT PARTITIONS: N/A — fixture file.
#![allow(dead_code)] // each consumer uses a subset

use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use crypto::{Hash, KeyPair, PublicKey, Signature};
use doli_core::{Attestation, ProducerAnnouncement};
use doli_node::node::Node;
use network::protocols::SyncRequest;
use network::{NetworkCommand, NetworkEvent, NetworkService, PeerId};
use tokio::sync::mpsc;

use super::inc_i_237_m2_node_common as c;

/// One gossip / sync event a passive peer saw.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Ev {
    Attestation(Vec<u8>),
    Vote(Vec<u8>),
    Producers(Vec<PublicKey>),
    Digest(PeerId),
    BlockByHash(Hash),
}

pub type Log = Arc<Mutex<Vec<Ev>>>;

pub fn record(ev: &NetworkEvent) -> Option<Ev> {
    Some(match ev {
        NetworkEvent::NewAttestation(d, _, _) => Ev::Attestation(d.clone()),
        NetworkEvent::NewVote(d, _) => Ev::Vote(d.clone()),
        NetworkEvent::ProducerAnnouncementsReceived(a, _) => {
            Ev::Producers(a.iter().map(|x| x.pubkey).collect())
        }
        NetworkEvent::ProducerDigestReceived { peer_id, .. } => Ev::Digest(*peer_id),
        NetworkEvent::SyncRequest {
            request: SyncRequest::GetBlockByHash { hash },
            ..
        } => Ev::BlockByHash(*hash),
        _ => return None,
    })
}

/// A passive peer that logs every gossip / GetBlockByHash event it receives.
pub fn observe(mut svc: NetworkService) -> Log {
    let log: Log = Arc::default();
    let out = log.clone();
    tokio::spawn(async move {
        while let Some(ev) = svc.next_event().await {
            if let Some(e) = record(&ev) {
                out.lock().unwrap().push(e);
            }
        }
    });
    log
}

pub fn count(log: &Log, want: &Ev) -> usize {
    log.lock().unwrap().iter().filter(|e| *e == want).count()
}

pub fn count_where(log: &Log, f: impl Fn(&Ev) -> bool) -> usize {
    log.lock().unwrap().iter().filter(|e| f(e)).count()
}

/// Gossip and sync-request events go through the production dispatch; peer
/// management events are dropped.
fn dispatched(ev: &NetworkEvent) -> bool {
    matches!(
        ev,
        NetworkEvent::NewAttestation(..)
            | NetworkEvent::NewVote(..)
            | NetworkEvent::ProducerAnnouncementsReceived(..)
            | NetworkEvent::ProducerDigestReceived { .. }
            | NetworkEvent::ProducersAnnounced(..)
            | NetworkEvent::NewTransaction(..)
            | NetworkEvent::SyncRequest { .. }
    )
}

pub async fn pump(node: &mut Node, dur: Duration) {
    let until = Instant::now() + dur;
    while Instant::now() < until {
        let net = node.network.as_mut().expect("node network attached");
        if let Ok(Some(ev)) =
            tokio::time::timeout(Duration::from_millis(50), net.next_event()).await
        {
            if dispatched(&ev) {
                let _ = node.handle_network_event(ev).await;
            }
        }
    }
}

pub async fn member_node() -> (Node, KeyPair, tempfile::TempDir) {
    let temp = tempfile::TempDir::new().expect("tempdir");
    let member = KeyPair::generate();
    let node = Node::new_for_test(temp.path().to_path_buf(), vec![member.clone()])
        .await
        .expect("Node::new_for_test");
    (node, member, temp)
}

pub struct Line {
    pub b: Node,
    pub member: KeyPair,
    pub a_cmd: mpsc::Sender<NetworkCommand>,
    pub a_peer: PeerId,
    pub a_log: Log,
    pub c_log: Log,
    pub _t: tempfile::TempDir,
}

/// A -> B(real node, `member` in its producer set) -> C.
pub async fn line() -> Line {
    let (mut b, member, t) = member_node().await;
    let b_port = c::reserve_port();
    b.network = Some(c::start_net(b_port, vec![]).await);
    let a = c::start_net(c::reserve_port(), vec![c::addr(b_port)]).await;
    let (a_cmd, a_peer) = (a.command_sender(), *a.local_peer_id());
    let a_log = observe(a);
    let c_log = observe(c::start_net(c::reserve_port(), vec![c::addr(b_port)]).await);
    Line {
        b,
        member,
        a_cmd,
        a_peer,
        a_log,
        c_log,
        _t: t,
    }
}

impl Line {
    pub async fn send(&self, cmd: NetworkCommand) {
        self.a_cmd.send(cmd).await.expect("A command channel");
    }

    /// Publish `make(i)` from A and pump B until C logged a `kind` event.
    pub async fn warm(&mut self, kind: fn(&Ev) -> bool, make: impl Fn(u32) -> NetworkCommand) {
        let deadline = Instant::now() + Duration::from_secs(30);
        let mut i = 0;
        while count_where(&self.c_log, kind) == 0 {
            assert!(Instant::now() < deadline, "setup: A -> B -> C never formed");
            self.send(make(i)).await;
            i += 1;
            pump(&mut self.b, Duration::from_millis(500)).await;
        }
    }
}

pub fn is_attestation(e: &Ev) -> bool {
    matches!(e, Ev::Attestation(_))
}

pub fn is_vote(e: &Ev) -> bool {
    matches!(e, Ev::Vote(_))
}

pub fn is_producers(e: &Ev) -> bool {
    matches!(e, Ev::Producers(_))
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

pub fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs()
}

/// A signed gossip vote; `valid = false` keeps every field but the signature.
pub fn vote(kp: &KeyPair, timestamp: u64, valid: bool) -> Vec<u8> {
    let mut v = updater::VoteMessage::new(
        "9.9.9".into(),
        updater::Vote::Veto,
        kp.public_key().to_hex(),
    );
    v.timestamp = timestamp;
    let sig = crypto::signature::sign(&v.message_bytes(), kp.private_key());
    v.signature = if valid {
        sig.to_hex()
    } else {
        Signature::from_bytes([0x5A; 64]).to_hex()
    };
    serde_json::to_vec(&v).expect("vote json")
}

/// A GSet announcement `node` can verify (its network id and genesis hash).
pub async fn announcement(node: &Node, kp: &KeyPair, seq: u64) -> ProducerAnnouncement {
    let gh = node.chain_state.read().await.genesis_hash;
    ProducerAnnouncement::new(kp, node.config.network.id(), seq, gh)
}

/// Same announcement, garbage signature.
pub fn forge_announcement(a: &ProducerAnnouncement) -> ProducerAnnouncement {
    let mut f = a.clone();
    f.signature = Signature::from_bytes([0x5A; 64]);
    f
}

pub async fn gset_has(node: &Node, kp: &KeyPair) -> bool {
    node.producer_gset
        .read()
        .await
        .sorted_producers()
        .contains(kp.public_key())
}

/// Every tracing event (level, fields) of this process from first use on.
pub fn capture() -> Arc<Mutex<Vec<String>>> {
    static LOG: OnceLock<Arc<Mutex<Vec<String>>>> = OnceLock::new();
    LOG.get_or_init(|| {
        use tracing_subscriber::layer::SubscriberExt;
        let log: Arc<Mutex<Vec<String>>> = Arc::default();
        let sub = tracing_subscriber::registry().with(Capture(log.clone()));
        let _ = tracing::subscriber::set_global_default(sub);
        log
    })
    .clone()
}

pub fn captured(log: &Arc<Mutex<Vec<String>>>, needle: &str) -> Vec<String> {
    log.lock()
        .unwrap()
        .iter()
        .filter(|l| l.contains(needle))
        .cloned()
        .collect()
}

pub const DUP_PUBLISH: &str = "Not publishing a message that has already been published";

struct Capture(Arc<Mutex<Vec<String>>>);

struct Fields(String);

impl tracing::field::Visit for Fields {
    fn record_debug(&mut self, f: &tracing::field::Field, v: &dyn std::fmt::Debug) {
        self.0.push_str(&format!("{}={v:?} ", f.name()));
    }
}

impl<S: tracing::Subscriber> tracing_subscriber::Layer<S> for Capture {
    fn on_event(&self, e: &tracing::Event<'_>, _: tracing_subscriber::layer::Context<'_, S>) {
        if *e.metadata().level() <= tracing::Level::INFO {
            let mut f = Fields(String::new());
            e.record(&mut f);
            self.0
                .lock()
                .unwrap()
                .push(format!("{} {}", e.metadata().level(), f.0));
        }
    }
}
