//! GS-026 (INC-I-236) helper: forge a block in a scheduled producer's slot and
//! publish it on the gossip blocks topic. Models the raw gossipsub publisher in
//! `bins/node/tests/inc_i_237_m5_soak.rs`.
//!
//! For GS-026 the forger never holds the named producer's key: `--kind unsigned` leaves
//! `vdf_proof.pi` empty, `--kind forged` signs the header with a throwaway key.
//! Both are rejected above the signature activation height by a HEAD node and
//! accepted by a pre-fix node. Every other field (coinbase, VDF, genesis hash)
//! is built valid so the signature is the only reason a HEAD node rejects it.
//!
//! `--kind sibling` (GS-027) signs with the scheduled producer's OWN key, read
//! from `--key-file <wallet.json>`: a second block for the slot the producer
//! already filled, on the same parent, valid in coinbase, VDF, chainspec genesis
//! and signature, whose hash differs from `--differ-from <hash>`. Extra args:
//! `--chainspec <path> --key-file <path> [--differ-from <hex>]`; `--slot` is used.
//!
//! Usage: `gs026_forge_block --kind forged|unsigned|sibling --tip-b64 <b64> --height <h>
//! --slot <s> --producer <hex> --genesis-time <t> --slot-duration <d>
//! --dial <multiaddr>[,<multiaddr>...]`. Prints `INJECTED kind=… hash=… slot=…`.

use std::time::{Duration, Instant};

use crypto::{Hash, KeyPair, PrivateKey, PublicKey};
use doli_core::block::{Block, BlockBuilder};
use doli_core::chainspec::ChainSpec;
use doli_core::consensus::{construct_vdf_input, reward_pool_pubkey_hash, ConsensusParams};
use doli_core::network::Network;
use doli_core::tpop::heartbeat::{hash_chain_vdf, heartbeat_vdf_iterations_for_network};
use futures::StreamExt;
use libp2p::gossipsub::{self, IdentTopic};
use libp2p::swarm::SwarmEvent;
use libp2p::{identity::Keypair, Multiaddr, Swarm};
use network::gossip::{new_gossipsub, MeshConfig, BLOCKS_TOPIC};
use network::transport::build_transport;
use vdf::VdfOutput;

const PUBLISH_DEADLINE: Duration = Duration::from_secs(40);
const PROPAGATE_GRACE: Duration = Duration::from_secs(3);

fn arg(args: &[String], key: &str) -> Option<String> {
    args.iter()
        .position(|a| a == key)
        .and_then(|i| args.get(i + 1))
        .cloned()
}

fn req(args: &[String], key: &str) -> String {
    arg(args, key).unwrap_or_else(|| {
        eprintln!("missing required argument {key}");
        std::process::exit(2);
    })
}

/// Build the forged block: valid coinbase, VDF and genesis hash; the signature
/// is the only thing a keyless forger cannot produce.
fn forge(args: &[String]) -> (Block, u32) {
    let kind = req(args, "--kind");
    let tip_b64 = req(args, "--tip-b64");
    let height: u64 = req(args, "--height").parse().expect("height");
    let producer = PublicKey::from_hex(&req(args, "--producer")).expect("producer hex");
    let genesis_time: u64 = req(args, "--genesis-time").parse().expect("genesis-time");
    let slot_duration: u64 = req(args, "--slot-duration").parse().expect("slot-duration");

    use base64::Engine;
    let tip_bytes = base64::engine::general_purpose::STANDARD
        .decode(tip_b64.trim())
        .expect("tip base64");
    let tip = Block::deserialize(&tip_bytes).expect("tip block bincode");
    let prev_hash = tip.header.hash();
    let genesis_hash = tip.header.genesis_hash;
    let prev_slot = tip.header.slot;

    let mut params = ConsensusParams::devnet();
    params.genesis_time = genesis_time;
    params.slot_duration = slot_duration;
    params.genesis_hash = genesis_hash;

    // Choose the slot from the DECODED tip, not from a CLI guess: it must be
    // strictly greater than the tip's slot (BlockBuilder rejects a non-increasing
    // slot) AND its timestamp must not be in the future (the node rejects
    // timestamp > now + MAX_DRIFT=1s). slot = prev_slot + 1 satisfies the first;
    // for the second we busy-wait until wall-clock reaches that slot. Production
    // is paused by the runner, so prev_slot is frozen and the wait is bounded by
    // one slot.
    let slot: u32 = prev_slot + 1;
    let timestamp = params.slot_to_timestamp(slot);
    loop {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        if now >= timestamp {
            break;
        }
        let wait = timestamp - now;
        if wait > slot_duration + 2 {
            // The tip is more than a slot ahead of wall-clock; something is off.
            eprintln!("forge: tip slot {prev_slot} is {wait}s ahead of now; aborting");
            std::process::exit(3);
        }
        std::thread::sleep(Duration::from_millis(250));
    }

    let mut builder = BlockBuilder::new(prev_hash, prev_slot, producer)
        .with_params(params)
        .with_fork_id(Hash::ZERO);
    builder.add_coinbase(height, slot, reward_pool_pubkey_hash());
    let (mut header, txs) = builder.build(timestamp).expect("block builder slot");
    header.genesis_hash = genesis_hash;

    let vdf_input = construct_vdf_input(&prev_hash, &header.merkle_root, slot, &producer);
    let iterations = heartbeat_vdf_iterations_for_network(Network::Devnet);
    header.vdf_output = VdfOutput {
        value: hash_chain_vdf(&vdf_input, iterations).to_vec(),
    };

    match kind.as_str() {
        "unsigned" => {} // pi stays empty: no signature at all
        "forged" => {
            let impostor = KeyPair::from_seed([0x26; 32]);
            let msg = header.signing_message();
            header.vdf_proof.pi =
                crypto::signature::sign(msg.as_bytes(), impostor.private_key()).to_vec();
        }
        other => {
            eprintln!("--kind must be forged or unsigned, got {other}");
            std::process::exit(2);
        }
    }

    (Block::new(header, txs), slot)
}

fn die(code: i32, msg: &str) -> ! {
    eprintln!("gs026 forge: {msg}");
    std::process::exit(code);
}

/// Keypair of the wallet's first address; no key byte is ever printed.
fn load_wallet_keypair(path: &str) -> KeyPair {
    let text = std::fs::read_to_string(path).unwrap_or_else(|_| die(2, "key file unreadable"));
    let wallet: serde_json::Value =
        serde_json::from_str(&text).unwrap_or_else(|_| die(2, "key file is not wallet JSON"));
    let hex_key = wallet["addresses"][0]["private_key"]
        .as_str()
        .unwrap_or_else(|| die(2, "key file has no first address key"));
    let key = PrivateKey::from_hex(hex_key).unwrap_or_else(|_| die(2, "key file key is malformed"));
    KeyPair::from_private_key(key)
}

/// A second, validly signed block by `--producer` for `--slot` on the tip parent.
fn forge_sibling(args: &[String]) -> (Block, u32) {
    use base64::Engine;
    let tip_bytes = base64::engine::general_purpose::STANDARD
        .decode(req(args, "--tip-b64").trim())
        .unwrap_or_else(|_| die(2, "tip base64"));
    let tip = Block::deserialize(&tip_bytes).unwrap_or_else(|| die(2, "tip block bincode"));
    let height: u64 = req(args, "--height").parse().expect("height");
    let slot: u32 = req(args, "--slot").parse().expect("slot");
    let producer = PublicKey::from_hex(&req(args, "--producer")).expect("producer hex");
    let differ = arg(args, "--differ-from").and_then(|h| Hash::from_hex(&h));
    let spec = ChainSpec::load(req(args, "--chainspec")).unwrap_or_else(|_| die(2, "chainspec"));

    let mut params = ConsensusParams::devnet();
    params.apply_chainspec(&spec);
    if params.genesis_hash != tip.header.genesis_hash {
        die(3, "chainspec genesis differs from the tip block genesis");
    }
    let keypair = load_wallet_keypair(&req(args, "--key-file"));
    if *keypair.public_key() != producer {
        die(2, "key file does not belong to --producer");
    }
    if slot <= tip.header.slot {
        die(3, "--slot must be above the tip slot");
    }

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let slot_start = params.slot_to_timestamp(slot);
    let iterations = Network::Devnet.heartbeat_vdf_iterations();
    for timestamp in slot_start..slot_start + params.slot_duration {
        if timestamp > now || timestamp <= tip.header.timestamp {
            continue;
        }
        let mut builder = BlockBuilder::new(tip.header.hash(), tip.header.slot, producer)
            .with_params(params.clone())
            .with_fork_id(tip.header.fork_id);
        builder.add_coinbase(height, slot, reward_pool_pubkey_hash());
        let Some((mut header, txs)) = builder.build(timestamp) else {
            continue;
        };
        header.genesis_hash = params.genesis_hash;
        header.vdf_output = VdfOutput {
            value: hash_chain_vdf(&header.vdf_input(), iterations).to_vec(),
        };
        let msg = header.signing_message();
        header.vdf_proof.pi =
            crypto::signature::sign(msg.as_bytes(), keypair.private_key()).to_vec();
        if header.slot == slot && Some(header.hash()) != differ {
            return (Block::new(header, txs), slot);
        }
    }
    die(3, "no in-slot timestamp yields a distinct sibling yet");
}

#[tokio::main]
async fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let kind = req(&args, "--kind");
    let dials: Vec<Multiaddr> = req(&args, "--dial")
        .split(',')
        .filter(|s| !s.is_empty())
        .map(|s| s.parse().expect("dial multiaddr"))
        .collect();

    let (block, slot) = if kind == "sibling" {
        forge_sibling(&args)
    } else {
        forge(&args)
    };
    let bytes = block.serialize();
    let hash = block.hash().to_hex();

    let kp = Keypair::generate_ed25519();
    let peer_id = kp.public().to_peer_id();
    let mesh = MeshConfig {
        mesh_n: 6,
        mesh_n_low: 4,
        mesh_n_high: 12,
        gossip_lazy: 6,
    };
    let mut swarm: Swarm<gossipsub::Behaviour> = Swarm::new(
        build_transport(&kp, None).expect("transport"),
        new_gossipsub(&kp, &mesh).expect("gossipsub"),
        peer_id,
        libp2p::swarm::Config::with_tokio_executor()
            .with_idle_connection_timeout(Duration::from_secs(600)),
    );
    let topic = IdentTopic::new(BLOCKS_TOPIC);
    swarm.behaviour_mut().subscribe(&topic).expect("subscribe");
    for addr in &dials {
        swarm.dial(addr.clone()).expect("dial");
    }

    let start = Instant::now();
    let mut published_at: Option<Instant> = None;
    loop {
        if let Some(t) = published_at {
            if t.elapsed() >= PROPAGATE_GRACE {
                break;
            }
        } else if start.elapsed() >= PUBLISH_DEADLINE {
            eprintln!("gs026 forge: no gossip mesh peer within {PUBLISH_DEADLINE:?}");
            std::process::exit(1);
        }
        let step = tokio::time::timeout(Duration::from_millis(500), swarm.select_next_some()).await;
        if let Ok(SwarmEvent::Behaviour(gossipsub::Event::Subscribed { .. })) = step {
            // a peer joined the blocks mesh; publish is attempted below
        }
        if published_at.is_none()
            && swarm
                .behaviour_mut()
                .publish(topic.clone(), bytes.clone())
                .is_ok()
        {
            published_at = Some(Instant::now());
            println!("INJECTED kind={kind} hash={hash} slot={slot}");
        }
    }
}
