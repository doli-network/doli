//! The one per-topic gossip relay table (INC-I-237 D2). Blocks keep the one
//! pre-verdict Accept (decode + stale <= 6 slots) and undecodable block bytes
//! are the only Reject. Every other topic is Ignore, LocalOnly (processed,
//! never forwarded) or Defer (the node's verdict decides).

use crypto::{Hash, PublicKey};
use doli_core::{
    decode_digest, decode_producer_set, is_legacy_bincode_format, Attestation, Block,
    ProducerAnnouncement, ProducerBloomFilter, Transaction,
};
use libp2p::gossipsub::MessageAcceptance;

use super::staleness::{
    attestation_is_current, seen_key, GossipTopic, StalenessCtx, VOTE_MAX_AGE_SECS,
};
use super::validation::{classify_block_gossip, classify_producer_gossip};
use super::{
    decode_tx_gossip, TxGossipMessage, ATTESTATION_TOPIC, BLOCKS_TOPIC, HEADERS_TOPIC,
    HEARTBEATS_TOPIC, PRODUCERS_TOPIC, TIER1_BLOCKS_TOPIC, TRANSACTIONS_TOPIC, VOTES_TOPIC,
};

/// Every topic a gossip message can name.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RelayTopic {
    Blocks,
    Transactions,
    Producers,
    Votes,
    Attestations,
    Headers,
    Heartbeats,
    Tier1Blocks,
    RegionBlocks,
    Unknown,
}

impl RelayTopic {
    /// Exact topic match; canonical `region_topic(n)` strings are `RegionBlocks`.
    pub fn from_topic_str(topic: &str) -> Self {
        match topic {
            BLOCKS_TOPIC => Self::Blocks,
            TRANSACTIONS_TOPIC => Self::Transactions,
            PRODUCERS_TOPIC => Self::Producers,
            VOTES_TOPIC => Self::Votes,
            ATTESTATION_TOPIC => Self::Attestations,
            HEADERS_TOPIC => Self::Headers,
            HEARTBEATS_TOPIC => Self::Heartbeats,
            TIER1_BLOCKS_TOPIC => Self::Tier1Blocks,
            t if is_region_topic(t) => Self::RegionBlocks,
            _ => Self::Unknown,
        }
    }
}

fn is_region_topic(topic: &str) -> bool {
    topic
        .strip_prefix("/doli/r")
        .and_then(|t| t.strip_suffix("/blocks/1"))
        .and_then(|n| n.parse::<u32>().ok().filter(|r| r.to_string() == n))
        .is_some()
}

/// The swarm's verdict for one gossip message.
#[derive(Debug)]
pub enum RelayDecision {
    /// Undecodable block bytes.
    Reject,
    /// Neither forwarded nor processed.
    Ignore,
    /// Forwarded now; the decoded block goes to the node.
    AcceptBlock(Box<Block>),
    /// Processed locally, never forwarded.
    LocalOnly,
    /// Processed; the node's verdict decides the forward.
    Defer,
}

impl RelayDecision {
    /// The swarm's validation report for this decision; `None` = the node reports.
    pub fn acceptance(&self) -> Option<MessageAcceptance> {
        match self {
            Self::Reject => Some(MessageAcceptance::Reject),
            Self::Ignore | Self::LocalOnly => Some(MessageAcceptance::Ignore),
            Self::AcceptBlock(_) => Some(MessageAcceptance::Accept),
            Self::Defer => None,
        }
    }
}

/// Decoded content of a message the node processes.
pub(crate) enum Payload {
    None,
    Txs(Vec<Transaction>),
    TxHashes(Vec<Hash>),
    Digest(ProducerBloomFilter),
    LegacyProducers(Vec<PublicKey>),
    Announcements(Vec<ProducerAnnouncement>),
    Vote,
    Attestation,
}

/// The relay decision for `data` received on `topic`.
pub fn relay_decision(topic: RelayTopic, data: &[u8], ctx: &mut StalenessCtx<'_>) -> RelayDecision {
    decide(topic, data, ctx).0
}

/// [`relay_decision`] plus the decoded payload, so the swarm decodes once.
pub(crate) fn decide(
    topic: RelayTopic,
    data: &[u8],
    ctx: &mut StalenessCtx<'_>,
) -> (RelayDecision, Payload) {
    let (genesis, slot_secs, now) = (ctx.genesis_time, ctx.slot_duration, ctx.now_unix);
    match topic {
        RelayTopic::Blocks => match classify_block_gossip(data, genesis, slot_secs, now) {
            (MessageAcceptance::Accept, Some(b)) => {
                (RelayDecision::AcceptBlock(Box::new(b)), Payload::None)
            }
            (MessageAcceptance::Reject, _) => (RelayDecision::Reject, Payload::None),
            _ => (RelayDecision::Ignore, Payload::None),
        },
        RelayTopic::Transactions => {
            match first_sight(GossipTopic::Transactions, data, ctx, decode_tx_gossip) {
                Some(TxGossipMessage::FullBatch(txs)) => (RelayDecision::Defer, Payload::Txs(txs)),
                Some(TxGossipMessage::Announce(h)) => {
                    (RelayDecision::LocalOnly, Payload::TxHashes(h))
                }
                None => (RelayDecision::Ignore, Payload::None),
            }
        }
        RelayTopic::Producers => producers(data, now),
        RelayTopic::Votes => {
            let current = |d: &[u8]| {
                serde_json::from_slice::<VoteAge>(d).ok().filter(|v| {
                    v.timestamp == 0 || now <= v.timestamp.saturating_add(VOTE_MAX_AGE_SECS)
                })
            };
            deferred(
                first_sight(GossipTopic::Votes, data, ctx, current),
                Payload::Vote,
            )
        }
        RelayTopic::Attestations => {
            let current = |d: &[u8]| {
                Attestation::from_bytes(d)
                    .filter(|a| attestation_is_current(a, genesis, slot_secs, now))
            };
            let seen = first_sight(GossipTopic::Attestations, data, ctx, current);
            deferred(seen, Payload::Attestation)
        }
        RelayTopic::Headers
        | RelayTopic::Heartbeats
        | RelayTopic::Tier1Blocks
        | RelayTopic::RegionBlocks
        | RelayTopic::Unknown => (RelayDecision::Ignore, Payload::None),
    }
}

/// Only the vote field the age bound needs; identity is the raw bytes.
#[derive(serde::Deserialize)]
struct VoteAge {
    timestamp: u64,
}

/// Raw-bytes identity dedup (INV-NETWORK-004) around `decode`; records the
/// key only when the message is new and `decode` accepts it.
fn first_sight<T>(
    topic: GossipTopic,
    data: &[u8],
    ctx: &mut StalenessCtx<'_>,
    decode: impl FnOnce(&[u8]) -> Option<T>,
) -> Option<T> {
    let key = seen_key(topic, &[data]);
    let decoded = if ctx.seen.contains(&key, ctx.now_unix) {
        None
    } else {
        decode(data)
    };
    if decoded.is_some() {
        ctx.seen.record(key, ctx.now_unix);
    }
    decoded
}

fn deferred<T>(decoded: Option<T>, payload: Payload) -> (RelayDecision, Payload) {
    match decoded {
        Some(_) => (RelayDecision::Defer, payload),
        None => (RelayDecision::Ignore, Payload::None),
    }
}

/// All-stale sets are Ignored first (INC-I-137); then today's decode order:
/// bloom digest, legacy bincode, protobuf set (INC-I-002).
fn producers(data: &[u8], now_unix: u64) -> (RelayDecision, Payload) {
    let payload = match (
        classify_producer_gossip(data, now_unix),
        decode_digest(data),
    ) {
        (MessageAcceptance::Ignore, _) => Payload::None,
        (_, Ok(bloom)) if bloom.size_bits() > 0 => Payload::Digest(bloom),
        _ if is_legacy_bincode_format(data) => bincode::deserialize::<Vec<PublicKey>>(data)
            .map_or(Payload::None, Payload::LegacyProducers),
        _ => decode_producer_set(data).map_or(Payload::None, Payload::Announcements),
    };
    let decision = match payload {
        Payload::LegacyProducers(_) | Payload::Digest(_) => RelayDecision::LocalOnly,
        Payload::Announcements(_) => RelayDecision::Defer,
        _ => RelayDecision::Ignore,
    };
    (decision, payload)
}
