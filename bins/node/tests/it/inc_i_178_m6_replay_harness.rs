//! INC-I-178 M6 — REQ-BLS-009 / C17 / P5: replay ONE epoch under BOTH bit
//! semantics and diff the qualifier sets at BOTH thresholds.
//!
//! OUTPUT CONTRACT: N/A — fixture file. It declares no `#[test]`; the enumerations live
//! with the functions under test in `inc_i_178_m6_replay.rs`. INPUT PARTITIONS: N/A.
//!
//! WHAT THIS MEASURES. Redefining bit *i* from "attested some block this minute" to
//! "attested this block's parent" is consensus-visible. REQ-BLS-009 makes acceptance
//! conditional on a MEASURED qualifier delta, not on an argument. The harness feeds one
//! epoch of per-block inputs through the two builder rules and then through the SAME
//! downstream — `EpochState::accumulate_block`, `Node::calculate_epoch_rewards`,
//! `EpochState::derive_at_boundary` — and reports the symmetric difference.
//!
//! IT CALLS THE PRODUCTION FUNCTIONS. The bits come from the shipped
//! `commit::build_attestation_commitment_at`, driven at `ah = u64::MAX` for the pre-AH
//! rule and `ah = 0` for the post-AH rule; the accumulation, the reward qualifier and the
//! demotion filter are the shipped functions, called, never copied. The only step the
//! harness composes itself is the three-line decode `post_commit.rs` does between the
//! block and `accumulate_block` (universe -> `decode_attestation_bitfield_vec` -> base
//! split), because `post_commit_actions` needs a `BlockBatch` and an `UndoData` and does
//! ten unrelated things. `assert_universe_is_base` pins the precondition that makes that
//! composition exact: these epochs have no mid-epoch activation, so the extra segment is
//! empty and the base split is the identity.
//!
//! WHY MAINNET-SHAPED PARAMS. `Node::new_for_test` builds `Network::Devnet`, whose
//! `blocks_per_reward_epoch` is 4. That makes `attestation_minutes_per_epoch` 0, so
//! `attestation_qualification_threshold` is 0 and EVERY producer qualifies, and it puts
//! `MIN_ATTESTATION_MINUTES` (30) out of reach for an epoch that spans at most one
//! minute. A devnet-shaped replay is vacuous at BOTH thresholds. [`replay_epoch`]
//! therefore points `config.network` at `Network::Mainnet` — 360 blocks, 60 minutes,
//! threshold 54 — and uses `ACTIVE_PRODUCERS_CAP + 1` producers, because
//! `derive_at_boundary` only reaches its `MIN_ATTESTATION_MINUTES` retain when
//! `new_list.len() > ACTIVE_PRODUCERS_CAP`. Every threshold is read back out of the
//! shipped params or constants; no test spells 54, 30 or 360.
//!
//! The JSON capture format M7 fills, and its loader, live in
//! `inc_i_178_m6_replay_fixture.rs`.
//!
//! WHY THE POOLED SIGNATURES NEED NO MESSAGE BINDING. `commit::pooled_commitment` only
//! calls `BlsSignature::try_from_slice` on a pooled entry — it never verifies the message
//! (that is M5's `verify.rs`, on the validator side). So the harness signs ONE message per
//! producer and reuses those 96 bytes for every parent: 51 signings instead of 18 360, with
//! no loss of fidelity in what is being measured, which is the BIT SEMANTICS.
//!
//! COUNTER HAZARD (M5 lesson). `replay_epoch` drives the real builder rule thousands of
//! times and therefore writes process-global metrics. It does NOT take
//! `inc_i_178_m5_common::counter_lock()` itself — a `tokio::sync::Mutex` is not reentrant,
//! and every consumer needs the lock across its own assertions anyway. Each replay TEST
//! takes the lock as its first statement.

#![allow(dead_code)] // each consumer uses a subset

use std::collections::{BTreeSet, HashMap, HashSet};

use crypto::{bls_sign, BlsKeyPair, Hash, PublicKey};
use doli_core::attestation::{attestation_minute, bls_attest_msg, ParentSignaturePool};
use doli_core::consensus::{reward_pool_pubkey_hash, ACTIVE_PRODUCERS_CAP};
use doli_core::transaction::Output;
use doli_core::{
    Block, BlockAccumulationInput, BlockHeader, EpochDerivationInput, EpochState, Network,
};
use doli_node::node::attestation::commit;
use doli_node::node::Node;
use storage::{Outpoint, UtxoEntry};
use vdf::{VdfOutput, VdfProof};

use crate::inc_i_178_m0_common::make_node;

/// The producer count that makes BOTH thresholds reachable: one past the tier cap, so
/// `derive_at_boundary` takes the branch that consults `MIN_ATTESTATION_MINUTES`.
pub fn replay_producer_count() -> usize {
    ACTIVE_PRODUCERS_CAP + 1
}

/// Big enough that `pool / qualifiers` is never 0 — `calculate_epoch_rewards` drops a
/// zero-value output, which would make the qualifier set unreadable from its return.
const REWARD_POOL: u64 = 100_000_000_000;

pub const FIXTURE_FORMAT: &str = "inc-i-178-m6-epoch-replay/1";

// ---------------------------------------------------------------------------
// Inputs
// ---------------------------------------------------------------------------

/// Which builder rule produced the bits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BitSemantics {
    /// Today: bit `i` is set iff `universe[i]` appears in the minute tracker for this
    /// block's attestation minute.
    PreAhMinuteUnion,
    /// M4: bit `i` is set iff `universe[i]` holds a valid pooled signature for THIS
    /// block's parent.
    PostAhParentAttestation,
}

impl BitSemantics {
    pub const BOTH: [BitSemantics; 2] = [
        BitSemantics::PreAhMinuteUnion,
        BitSemantics::PostAhParentAttestation,
    ];

    /// The gate value that selects this arm inside the shipped builder. Nothing else in
    /// the harness branches on the semantics.
    fn activation_height(self) -> u64 {
        match self {
            BitSemantics::PreAhMinuteUnion => u64::MAX,
            BitSemantics::PostAhParentAttestation => 0,
        }
    }
}

/// Which downstream decision the qualifier set comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QualifierThreshold {
    /// `calculate_epoch_rewards`: 90 % of the epoch's attestation minutes (54/60 at
    /// mainnet shape). The set is the producers that received a reward output.
    Reward54,
    /// `derive_at_boundary`: `MIN_ATTESTATION_MINUTES` (30) plus the 80 %-of-expected
    /// production floor, applied through the 3-epoch liveness union. The set is
    /// `active_list`.
    Demotion30,
}

impl QualifierThreshold {
    pub const BOTH: [QualifierThreshold; 2] =
        [QualifierThreshold::Reward54, QualifierThreshold::Demotion30];
}

/// One `(attester, minute)` row of the minute tracker as it stood at build time.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Attendance {
    pub attester: usize,
    pub minute: u32,
}

/// One block of the replayed epoch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReplayBlock {
    pub height: u64,
    pub slot: u32,
    pub producer: usize,
    pub parent_hash: Hash,
    pub attendance: Vec<Attendance>,
    pub parent_attesters: Vec<usize>,
}

/// One epoch of per-block inputs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReplayEpoch {
    pub label: String,
    pub epoch: u64,
    pub producer_count: usize,
    pub blocks: Vec<ReplayBlock>,
}

// ---------------------------------------------------------------------------
// Outputs
// ---------------------------------------------------------------------------

/// Everything one semantics produced, in universe-index space.
#[derive(Debug, Clone)]
pub struct SemanticsRun {
    pub semantics: BitSemantics,
    /// Producers that received a reward output from `calculate_epoch_rewards`.
    pub reward_qualifiers: BTreeSet<usize>,
    /// `active_list` after `derive_at_boundary`.
    pub demotion_survivors: BTreeSet<usize>,
    /// Distinct attestation minutes credited per producer by the reward scan's own
    /// skip rules. Reported, never used as a qualifier — the qualifier is the shipped
    /// function's return value.
    pub reward_minutes: Vec<usize>,
    /// Distinct minutes per producer in `attestation_accum[0]`, the number the demotion
    /// retain reads. Includes a producer's own blocks, which the reward scan does not.
    pub accum_minutes: Vec<usize>,
    /// Total set bits across the epoch — the "one bit instead of six" magnitude.
    pub set_bits_total: usize,
}

/// The REQ-BLS-009 answer for one epoch.
#[derive(Debug, Clone)]
pub struct ReplayReport {
    pub label: String,
    pub producer_count: usize,
    pub block_count: usize,
    pub runs: Vec<SemanticsRun>,
    pub blocks_with_differing_bitfield: usize,
    pub blocks_with_differing_presence_root: usize,
}

impl ReplayReport {
    pub fn run(&self, semantics: BitSemantics) -> &SemanticsRun {
        self.runs
            .iter()
            .find(|r| r.semantics == semantics)
            .expect("both semantics are always replayed")
    }

    pub fn qualifiers(
        &self,
        semantics: BitSemantics,
        threshold: QualifierThreshold,
    ) -> &BTreeSet<usize> {
        let run = self.run(semantics);
        match threshold {
            QualifierThreshold::Reward54 => &run.reward_qualifiers,
            QualifierThreshold::Demotion30 => &run.demotion_survivors,
        }
    }

    /// The producers whose qualification DEPENDS on which bit semantics ran. Empty means
    /// the switch is invisible to that decision.
    pub fn symmetric_difference(&self, threshold: QualifierThreshold) -> BTreeSet<usize> {
        let pre = self.qualifiers(BitSemantics::PreAhMinuteUnion, threshold);
        let post = self.qualifiers(BitSemantics::PostAhParentAttestation, threshold);
        pre.symmetric_difference(post).copied().collect()
    }

    /// One line per threshold, for the test's failure message and the artifact.
    pub fn summary(&self) -> String {
        let mut out = format!(
            "[M6_REPLAY] label={} producers={} blocks={} differing_bitfields={} \
             differing_roots={}",
            self.label,
            self.producer_count,
            self.block_count,
            self.blocks_with_differing_bitfield,
            self.blocks_with_differing_presence_root
        );
        for t in QualifierThreshold::BOTH {
            out.push_str(&format!(
                "\n[M6_REPLAY] threshold={:?} pre={} post={} delta={:?}",
                t,
                self.qualifiers(BitSemantics::PreAhMinuteUnion, t).len(),
                self.qualifiers(BitSemantics::PostAhParentAttestation, t)
                    .len(),
                self.symmetric_difference(t)
            ));
        }
        out
    }
}

// ---------------------------------------------------------------------------
// The replay
// ---------------------------------------------------------------------------

/// Run `spec` under both semantics and diff the qualifier sets.
pub async fn replay_epoch(spec: &ReplayEpoch) -> ReplayReport {
    let mut runs = Vec::new();
    let mut bodies: Vec<Vec<(Vec<u8>, Hash)>> = Vec::new();
    for semantics in BitSemantics::BOTH {
        let (run, body) = replay_one(spec, semantics).await;
        runs.push(run);
        bodies.push(body);
    }

    let differing_bitfield = bodies[0]
        .iter()
        .zip(bodies[1].iter())
        .filter(|(a, b)| a.0 != b.0)
        .count();
    let differing_root = bodies[0]
        .iter()
        .zip(bodies[1].iter())
        .filter(|(a, b)| a.1 != b.1)
        .count();

    ReplayReport {
        label: spec.label.clone(),
        producer_count: spec.producer_count,
        block_count: spec.blocks.len(),
        runs,
        blocks_with_differing_bitfield: differing_bitfield,
        blocks_with_differing_presence_root: differing_root,
    }
}

async fn replay_one(
    spec: &ReplayEpoch,
    semantics: BitSemantics,
) -> (SemanticsRun, Vec<(Vec<u8>, Hash)>) {
    let (mut node, _producers, _tmp) = make_node(spec.producer_count).await;
    // Mainnet-shaped epoch geometry. See the module doc: devnet's 4-block epoch makes
    // both thresholds unreachable, so a devnet replay cannot fail.
    node.config.network = Network::Mainnet;
    let blocks_per_epoch = node.config.network.blocks_per_reward_epoch();
    assert_eq!(
        spec.blocks.len() as u64,
        blocks_per_epoch,
        "{}: the replay must cover a WHOLE epoch — calculate_epoch_rewards fails fast on \
         any missing height in the window",
        spec.label
    );

    let ah = semantics.activation_height();
    node.inc_i_178_attestation_bls_activation_height = ah;
    seed_reward_pool(&node, REWARD_POOL).await;

    let universe = node.epoch_state.producer_list.clone();
    assert_eq!(
        universe.len(),
        spec.producer_count,
        "the epoch producer list is the universe every index in the spec refers to"
    );
    assert_universe_is_base(&node, &universe, spec.blocks[0].height).await;

    let index_of: HashMap<PublicKey, usize> = universe
        .iter()
        .enumerate()
        .map(|(i, pk)| (*pk, i))
        .collect();
    let signatures = pooled_signatures(spec.producer_count);

    let mut bodies: Vec<(Vec<u8>, Hash)> = Vec::with_capacity(spec.blocks.len());
    let mut reward_minutes: Vec<HashSet<u32>> = vec![HashSet::new(); spec.producer_count];
    let mut set_bits_total = 0usize;

    for b in &spec.blocks {
        let minute = attestation_minute(b.slot);

        let mut pool = ParentSignaturePool::new();
        for &i in &b.parent_attesters {
            assert!(
                i < spec.producer_count,
                "parent_attesters index out of range"
            );
            pool.insert(b.parent_hash, universe[i], signatures[i]);
        }
        let attested: Vec<PublicKey> = b
            .attendance
            .iter()
            .filter(|a| a.minute == minute)
            .map(|a| {
                assert!(
                    a.attester < spec.producer_count,
                    "attendance index out of range"
                );
                universe[a.attester]
            })
            .collect();

        let commitment = commit::build_attestation_commitment_at(
            ah,
            b.height,
            &universe,
            &attested,
            &pool,
            &b.parent_hash,
        );

        let block = stored_block(b, universe[b.producer], &commitment);
        node.block_store
            .put_block_canonical(&block, b.height)
            .expect("put_block_canonical");

        // The three lines `post_commit_actions` runs between the block and the
        // accumulator, with the extra segment empty by the precondition above.
        let indices = if commitment.bitfield.is_empty() {
            Vec::new()
        } else {
            doli_core::decode_attestation_bitfield_vec(&commitment.bitfield, universe.len())
        };
        let has_attestation_data = !commitment.presence_root.is_zero() && !universe.is_empty();
        node.epoch_state.accumulate_block(&BlockAccumulationInput {
            producer: universe[b.producer],
            slot: b.slot,
            has_attestation_data: has_attestation_data && !indices.is_empty(),
            attested_indices: indices.clone(),
        });

        // The reward scan's own skip rules, so the reported minute counts are the ones
        // `calculate_epoch_rewards` computed internally.
        let skipped = commitment.presence_root.is_zero()
            || commit::is_canonical_empty_attendance_at(
                ah,
                b.height,
                &commitment.presence_root,
                &commitment.bitfield,
            );
        if !skipped {
            for idx in &indices {
                reward_minutes[*idx].insert(minute);
            }
        }

        set_bits_total += indices.len();
        bodies.push((commitment.bitfield.clone(), commitment.presence_root));
    }

    let reward_qualifiers = reward_qualifiers(&node, spec.epoch, &universe).await;
    let demotion_survivors =
        demotion_survivors(&node, spec.epoch, blocks_per_epoch, &index_of).await;
    let accum_minutes = accum_minutes(&node, &universe);

    (
        SemanticsRun {
            semantics,
            reward_qualifiers,
            demotion_survivors,
            reward_minutes: reward_minutes.iter().map(|s| s.len()).collect(),
            accum_minutes,
            set_bits_total,
        },
        bodies,
    )
}

/// The universe the three consensus sites share must equal the epoch producer list here.
/// If it ever does not, the base/extra split the harness skips stops being the identity
/// and every index in the report is suspect.
async fn assert_universe_is_base(node: &Node, base: &[PublicKey], height: u64) {
    let active: Vec<PublicKey> = {
        let ps = node.producer_set.read().await;
        ps.active_producers_at_height(height)
            .iter()
            .map(|p| p.public_key)
            .collect()
    };
    let universe = commit::encoder_universe_at(
        node.inc_i_178_attestation_bls_activation_height,
        height,
        base,
        &active,
    );
    assert_eq!(
        universe.len(),
        base.len(),
        "the replay epochs have no mid-epoch activation, so the extra segment must be \
         empty and every bit index is a base index"
    );
}

/// One reusable 96-byte signature per producer. `pooled_commitment` deserializes the
/// bytes and never checks the message, so one signing per producer is exact for what is
/// being measured; message binding is `verify.rs`'s job (M5).
fn pooled_signatures(n: usize) -> Vec<[u8; 96]> {
    let msg = bls_attest_msg(&Hash::ZERO);
    (0..n)
        .map(|_| {
            let bls = BlsKeyPair::generate();
            let sig = bls_sign(&msg, bls.secret_key()).expect("BLS signing must succeed");
            *sig.as_bytes()
        })
        .collect()
}

fn stored_block(
    b: &ReplayBlock,
    producer: PublicKey,
    commitment: &commit::AttestationCommitment,
) -> Block {
    let header = BlockHeader {
        version: 2,
        prev_hash: b.parent_hash,
        merkle_root: Hash::ZERO,
        presence_root: commitment.presence_root,
        genesis_hash: Hash::ZERO,
        timestamp: 1_700_000_000 + b.height,
        slot: b.slot,
        producer,
        vdf_output: VdfOutput {
            value: vec![0u8; 32],
        },
        vdf_proof: VdfProof::empty(),
        missed_producers: Vec::new(),
        data_root: Hash::ZERO,
        fork_id: Hash::ZERO,
    };
    let mut block = Block::new(header, Vec::new());
    block.attestation_bitfield = commitment.bitfield.clone();
    block.aggregate_bls_signature = commitment.aggregate.clone();
    block
}

async fn seed_reward_pool(node: &Node, amount: u64) {
    let entry = UtxoEntry {
        output: Output::normal(amount, reward_pool_pubkey_hash()),
        height: 0,
        is_coinbase: true,
        is_epoch_reward: false,
    };
    let mut utxo = node.utxo_set.write().await;
    utxo.insert(
        Outpoint::new(crypto::hash::hash(b"inc-i-178-m6-replay-pool"), 0),
        entry,
    )
    .expect("insert pool UTXO");
}

/// The Reward54 set, read out of the shipped function's RETURN VALUE.
async fn reward_qualifiers(node: &Node, epoch: u64, universe: &[PublicKey]) -> BTreeSet<usize> {
    let by_hash: HashMap<Hash, usize> = universe
        .iter()
        .enumerate()
        .map(|(i, pk)| {
            (
                crypto::hash_with_domain(crypto::ADDRESS_DOMAIN, pk.as_bytes()),
                i,
            )
        })
        .collect();
    let outputs = node
        .calculate_epoch_rewards(epoch)
        .await
        .expect("the replay window is complete, so the reward scan must not fail fast");
    outputs
        .iter()
        .map(|(_amount, pkh)| {
            *by_hash.get(pkh).unwrap_or_else(|| {
                panic!(
                    "a reward output addressed a hash that is not a producer address — \
                     the harness cannot map it back to a universe index"
                )
            })
        })
        .collect()
}

/// The Demotion30 set: `active_list` after the shipped boundary derivation.
async fn demotion_survivors(
    node: &Node,
    epoch: u64,
    blocks_per_epoch: u64,
    index_of: &HashMap<PublicKey, usize>,
) -> BTreeSet<usize> {
    let height = (epoch + 1) * blocks_per_epoch;
    let (active_producers, registered_at) = {
        let ps = node.producer_set.read().await;
        let active = ps.active_producers_at_height(height);
        (
            active.iter().map(|p| p.public_key).collect::<Vec<_>>(),
            active
                .iter()
                .map(|p| (p.public_key, p.registered_at))
                .collect::<HashMap<_, _>>(),
        )
    };
    let params = node.config.network.params();
    let input = EpochDerivationInput {
        active_producers,
        bond_counts: node.epoch_state.bond_snapshot.clone(),
        blocks_per_epoch,
        snap_attestation_skip_height: params.snap_attestation_skip_height,
        height,
        // The boundary at the end of `epoch` ENTERS `epoch + 1`; the tier retain reads
        // `prev.attestation_accum[0]`, which is the epoch just replayed.
        epoch: epoch + 1,
        registered_at,
        ghost_exclusion_activation_height: params.ghost_exclusion_activation_height,
        epoch_prune_activation_height: params.epoch_prune_activation_height,
        inc_i_190_floor_bound_activation_height: params.inc_i_190_floor_bound_activation_height,
    };
    let derived = EpochState::derive_at_boundary(&node.epoch_state, &input);
    derived
        .active_list
        .iter()
        .filter_map(|pk| index_of.get(pk).copied())
        .collect()
}

fn accum_minutes(node: &Node, universe: &[PublicKey]) -> Vec<usize> {
    universe
        .iter()
        .map(|pk| {
            node.epoch_state.attestation_accum[0]
                .get(pk)
                .map(|s| s.len())
                .unwrap_or(0)
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Synthetic epochs
// ---------------------------------------------------------------------------

/// The four producers whose attestation pattern the degraded epoch varies. Every other
/// index attests every parent in every minute.
#[derive(Debug, Clone, Copy)]
pub struct DegradedRoles {
    /// Attests every OTHER parent.
    pub half: usize,
    /// Attests only on the LAST slot of every minute, all epoch long.
    pub late: usize,
    /// Attests only on the last slot of a minute, and only from minute 6 onward — which
    /// lands its old-semantics minute count exactly ON the 90 % threshold.
    pub brink: usize,
    /// Never attests.
    pub silent: usize,
}

pub const DEGRADED_ROLES: DegradedRoles = DegradedRoles {
    half: 0,
    late: 1,
    brink: 2,
    silent: 3,
};

/// The last slot of an attestation minute — the boundary the shift crosses.
fn is_minute_final(slot: u32) -> bool {
    (slot + 1).is_multiple_of(doli_core::attestation::SLOTS_PER_ATTESTATION_MINUTE)
}

/// Attestation minute counted from the epoch's first minute.
fn epoch_minute(slot: u32, epoch_start_slot: u32) -> i64 {
    attestation_minute(slot) as i64 - attestation_minute(epoch_start_slot) as i64
}

/// The number of whole minutes `brink` skips at the start of the epoch, chosen so its
/// pre-AH minute count lands EXACTLY on the qualification threshold.
fn brink_first_minute(blocks_per_epoch: u64) -> i64 {
    let minutes = doli_core::attestation::attestation_minutes_per_epoch(blocks_per_epoch) as i64;
    let threshold =
        doli_core::attestation::attestation_qualification_threshold(blocks_per_epoch) as i64;
    minutes - threshold
}

/// Does producer `p` attest the block at `slot` (pre-AH view) / hold a pooled signature
/// for the block at `slot` (post-AH view)? ONE rule, applied to the block's own slot for
/// attendance and to the PARENT's slot for the pool — which is the whole semantic
/// difference under test.
fn degraded_attests(p: usize, slot: u32, epoch_start_slot: u32, blocks_per_epoch: u64) -> bool {
    let r = DEGRADED_ROLES;
    if p == r.silent {
        return false;
    }
    if p == r.half {
        return slot.is_multiple_of(2);
    }
    if p == r.late {
        return is_minute_final(slot);
    }
    if p == r.brink {
        return is_minute_final(slot)
            && epoch_minute(slot, epoch_start_slot) >= brink_first_minute(blocks_per_epoch);
    }
    true
}

/// A deterministic parent hash for the block at `height`. Only used as the pool key.
pub fn replay_parent_hash(height: u64) -> Hash {
    crypto::hash::hash(&height.to_le_bytes())
}

fn build_epoch(
    label: &str,
    epoch: u64,
    producer_count: usize,
    blocks_per_epoch: u64,
    attests: &dyn Fn(usize, u32) -> bool,
) -> ReplayEpoch {
    let start = epoch * blocks_per_epoch;
    let blocks = (0..blocks_per_epoch)
        .map(|k| {
            let height = start + k;
            let slot = height as u32;
            let parent_slot = slot.saturating_sub(1);
            let minute = attestation_minute(slot);
            ReplayBlock {
                height,
                slot,
                producer: (k as usize) % producer_count,
                parent_hash: replay_parent_hash(height - 1),
                attendance: (0..producer_count)
                    .filter(|p| attests(*p, slot))
                    .map(|p| Attendance {
                        attester: p,
                        minute,
                    })
                    .collect(),
                parent_attesters: (0..producer_count)
                    .filter(|p| attests(*p, parent_slot))
                    .collect(),
            }
        })
        .collect();
    ReplayEpoch {
        label: label.to_string(),
        epoch,
        producer_count,
        blocks,
    }
}

/// Every producer attests every parent, in every minute.
pub fn healthy_epoch(producer_count: usize, epoch: u64, blocks_per_epoch: u64) -> ReplayEpoch {
    build_epoch(
        "healthy",
        epoch,
        producer_count,
        blocks_per_epoch,
        &|_p, _slot| true,
    )
}

/// Four degraded attestation patterns against a healthy majority. See [`DegradedRoles`].
pub fn degraded_epoch(producer_count: usize, epoch: u64, blocks_per_epoch: u64) -> ReplayEpoch {
    assert!(
        producer_count > DEGRADED_ROLES.silent,
        "the degraded epoch needs at least four producers to carry its roles"
    );
    let start_slot = (epoch * blocks_per_epoch) as u32;
    build_epoch(
        "degraded",
        epoch,
        producer_count,
        blocks_per_epoch,
        &|p, slot| degraded_attests(p, slot, start_slot, blocks_per_epoch),
    )
}
