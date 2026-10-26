//! INC-I-182 M3 — the GS-028 transaction forge builds exactly the in-process
//! forced-exit fixture, and core validation refuses it for the producer-auth
//! reason once its payout passes the unbonding placement rules.
//!
// shape.rs API: #[derive(Clone, Copy, Debug, PartialEq, Eq)] pub enum Kind { ForcedExit, ForcedExitWrongTail, AddBondUntailed } with Kind::parse(&str) -> Option<Kind>; same derives on pub enum Payout { Normal, Unbonding } with Payout::parse(&str) -> Option<Payout>
// pub struct Coin { pub tx_hash: Hash, pub index: u32, pub amount: u64 }; pub struct Spec { pub kind: Kind, pub victim: PublicKey, pub bonds: u32, pub payout: Payout, pub lock_until: u64, pub fee: u64, pub bond_unit: u64, pub genesis: Hash }
// pub fn build(spec: &Spec, payer: &KeyPair, coin: &Coin) -> Result<Transaction, String> — pure, no I/O; Err when the coin cannot pay the outputs plus the fee
//
// covers: shape main producer_auth unbonding output_authority transaction tx_types core data
//
// OUTPUT CONTRACT: fn shape::build(&Spec, &KeyPair, &Coin) -> Result<Transaction, String>
//   O1 ForcedExit: inputs [coin] with the payer's key and signature; ONE output of coin - fee to
//      bond_owner(victim): Unbonding at lock_until, or Normal with lock 0; extra_data =
//      WithdrawalRequestData(victim, bonds, bond_owner(victim)) with no tail
//   O2 ForcedExitWrongTail: O1 then sign_producer_auth(genesis, payer)
//   O3 AddBondUntailed: inputs [coin] signed by the payer; bonds x Bond(bond_unit, bond_owner(victim),
//      lock_until, 0), then a Normal change output to bond_owner(payer) only when change > 0;
//      extra_data = AddBondData(victim, bonds) with no tail
//   O4 Err: ForcedExit kinds when coin <= fee; AddBondUntailed when coin < bonds * bond_unit + fee
//   O5 validate_transaction verdict of O1..O3 at best + 1 with both gates armed
// OUTPUT CONTRACT: fn Kind::parse, Payout::parse -> Option (the --kind / --payout values)
// OUTPUT CONTRACT: source of examples/gs028_forge_tx/main.rs
//   O6 host literal 127.0.0.1 only, no host flag; getNetworkParams before sendTransaction; "devnet"
//      check; exit(3); no print naming private/secret/seed; no home-dir or .doli key path
// PATHS: P-UNB Unbonding payout above both gates | P-NORM Normal payout | P-LOW lock below the floor
//        | P-TAILED tail by the named producer | P-WRONG tail by the payer | P-ERR coin too small
// MATRIX
//   O1 x P-UNB -> ..._forced_exit_matches_fixture
//   O1, O5 x {P-UNB, P-NORM above and below the 257 gate, P-LOW, P-TAILED} -> ..._refused_for_auth_only_when_placement_passes
//   O2, O5 x P-WRONG -> ..._wrong_tail_refused_for_auth
//   O3, O5 x {untailed, P-TAILED, no change} -> ..._addbond_untailed_refused_for_auth
//   O4 + parse -> ..._parse_and_insufficient_coin
//   O6 -> ..._forge_main_guards (+ planted violations flagged)
// INPUT PARTITIONS:
//   best 1_000, devnet unbonding period, lock best+period+20 | best+period-1; fee 1; coin 500_000
//   | fee | fee+1 | bonds*unit+fee | bonds*unit+fee-1; bonds 2; gates 0 (armed) | best+100 (257 not armed)

#[allow(dead_code)]
#[path = "../../examples/gs028_forge_tx/shape.rs"]
mod shape;

use crypto::{Hash, KeyPair, PublicKey};
use doli_core::consensus::ConsensusParams;
use doli_core::transaction::{
    sign_producer_auth, AddBondData, OutputType, Transaction, TxType, WithdrawalRequestData,
};
use doli_core::validation::{bond_owner, validate_transaction, ValidationContext, ValidationError};
use doli_core::{Network, NetworkParams};
use shape::{build, Coin, Kind, Payout, Spec};
use storage::Outpoint;

use super::inc_i_182_m1_common::{forced_exit, CODE, COIN, FEE, LOCK};

const BEST: u64 = 1_000;
const BONDS: u32 = 2;
const TAIL: usize = 64;

fn payer() -> KeyPair {
    KeyPair::from_seed([0x71; 32])
}

fn victim_key() -> KeyPair {
    KeyPair::from_seed([0x72; 32])
}

fn victim() -> PublicKey {
    *victim_key().public_key()
}

fn genesis() -> Hash {
    ConsensusParams::devnet().genesis_hash
}

fn period() -> u64 {
    NetworkParams::defaults(Network::Devnet).unbonding_period
}

fn coin(amount: u64) -> Coin {
    Coin {
        tx_hash: Hash::from_bytes([0x82; 32]),
        index: 3,
        amount,
    }
}

fn spec(kind: Kind, payout: Payout, lock_until: u64) -> Spec {
    Spec {
        kind,
        victim: victim(),
        bonds: BONDS,
        payout,
        lock_until,
        fee: FEE,
        bond_unit: Network::Devnet.bond_unit(),
        genesis: genesis(),
    }
}

fn forged(kind: Kind, payout: Payout, lock_until: u64) -> Transaction {
    build(&spec(kind, payout, lock_until), &payer(), &coin(COIN))
        .unwrap_or_else(|e| panic!("{kind:?}/{payout:?} build: {e}"))
}

fn ctx(c1: u64, pauth: u64) -> ValidationContext {
    ValidationContext::new(ConsensusParams::devnet(), Network::Devnet, 0, BEST + 1)
        .with_inc_i_234_activation_height(0)
        .with_inc_i_257_unbonding(c1, period())
        .with_inc_i_182_producer_auth(pauth)
}

fn code_of(r: &Result<(), ValidationError>) -> String {
    match r {
        Err(ValidationError::InvalidTransaction(m)) => m
            .split(']')
            .next()
            .map(|c| c.trim_start_matches('[').to_string())
            .unwrap_or_default(),
        Err(e) => format!("{e:?}"),
        Ok(()) => "ok".to_string(),
    }
}

fn input_signed_by(tx: &Transaction, i: usize, k: &KeyPair) -> bool {
    let msg = tx.signing_message_for_input(i);
    tx.inputs[i].public_key == Some(*k.public_key())
        && tx.inputs[i].signature == crypto::signature::sign_hash(&msg, k.private_key())
}

fn wd_base_len() -> usize {
    WithdrawalRequestData::new(victim(), BONDS, bond_owner(&victim()))
        .to_bytes()
        .len()
}

// REQ-LDP-030 — Decision: a failure means the live forced-exit probe is not the transaction the in-process RED proved dangerous, so a live refusal says nothing about the attack.
#[test]
fn inc_i_182_m3_req_ldp_030_forced_exit_matches_fixture() {
    let tx = forged(Kind::ForcedExit, Payout::Unbonding, LOCK);
    let outpoint = Outpoint::new(coin(COIN).tx_hash, coin(COIN).index);
    assert_eq!(
        tx,
        forced_exit(&payer(), outpoint, &victim(), BONDS),
        "forge ForcedExit must equal the inc_i_182_m1_common::forced_exit fixture"
    );
    assert_eq!(tx.tx_type, TxType::RequestWithdrawal);
    assert_eq!(tx.inputs.len(), 1, "only the payer's own coin");
    assert!(
        input_signed_by(&tx, 0, &payer()),
        "the coin is signed by the payer"
    );
    let out = &tx.outputs[0];
    assert_eq!(
        (
            tx.outputs.len(),
            out.output_type,
            out.pubkey_hash,
            out.amount,
            out.lock_until
        ),
        (
            1,
            OutputType::Unbonding,
            bond_owner(&victim()),
            COIN - FEE,
            LOCK
        ),
        "one Unbonding payout of coin - fee to the victim's bond owner"
    );
    let data = tx.withdrawal_request_data().expect("withdrawal data");
    assert_eq!(
        (data.producer_pubkey, data.bond_count, data.destination),
        (victim(), BONDS, bond_owner(&victim())),
        "the payload names the victim"
    );
    assert_eq!(tx.extra_data.len(), wd_base_len(), "no producer-auth tail");
}

// REQ-LDP-003 — Decision: a failure means the forged forced exit is refused for a payout-shape reason (or accepted), so the live probe cannot show the producer-auth refusal GS-028 asserts.
#[test]
fn inc_i_182_m3_req_ldp_003_refused_for_auth_only_when_placement_passes() {
    let floor_ok = BEST + period() + 20;
    let cases = [
        (
            "Unbonding, lock best+period+20, both gates armed",
            Payout::Unbonding,
            floor_ok,
            0,
            CODE,
        ),
        (
            "Unbonding, lock below best+1+period",
            Payout::Unbonding,
            BEST + period() - 1,
            0,
            "ERRTX-UNBOND006",
        ),
        (
            "Normal payout above the 257 gate",
            Payout::Normal,
            floor_ok,
            0,
            "ERRTX-UNBOND004",
        ),
        (
            "Normal payout, 257 gate not armed",
            Payout::Normal,
            floor_ok,
            BEST + 100,
            CODE,
        ),
    ];
    for (what, payout, lock, c1, want) in cases {
        let tx = forged(Kind::ForcedExit, payout, lock);
        if payout == Payout::Normal {
            assert_eq!(
                (tx.outputs[0].output_type, tx.outputs[0].lock_until),
                (OutputType::Normal, 0),
                "{what}: a Normal payout carries lock 0"
            );
        }
        let got = code_of(&validate_transaction(&tx, &ctx(c1, 0)));
        assert_eq!(got, want, "{what}: refusal code");
    }
    let mut own = forged(Kind::ForcedExit, Payout::Unbonding, floor_ok);
    sign_producer_auth(&mut own, &genesis(), &victim_key());
    assert_eq!(
        code_of(&validate_transaction(&own, &ctx(0, 0))),
        "ok",
        "NON-VACUITY: with the victim's own tail the same shape is valid, so only auth refuses it"
    );
}

// REQ-LDP-032 — Decision: a failure means a forced exit whose tail is signed by the fee payer instead of the named producer gets through, so the auth rule checks the tail's presence rather than its signer.
#[test]
fn inc_i_182_m3_req_ldp_032_wrong_tail_refused_for_auth() {
    let lock = BEST + period() + 20;
    let tx = forged(Kind::ForcedExitWrongTail, Payout::Unbonding, lock);
    let mut want = forged(Kind::ForcedExit, Payout::Unbonding, lock);
    sign_producer_auth(&mut want, &genesis(), &payer());
    assert_eq!(
        tx, want,
        "wrong-tail = ForcedExit + a tail signed by the payer"
    );
    assert_eq!(
        tx.extra_data.len(),
        wd_base_len() + TAIL,
        "the tail is present"
    );
    assert!(
        input_signed_by(&tx, 0, &payer()),
        "the input signature survives the tail"
    );
    assert_eq!(
        code_of(&validate_transaction(&tx, &ctx(0, 0))),
        CODE,
        "a tail by the payer is refused for the producer-auth reason"
    );
}

// REQ-LDP-019 — Decision: a failure means the fallback unsigned AddBond probe is refused for a bond-shape reason (or accepted) above the auth gate, so gs029-unsigned-tx-refused could pass or fail for the wrong cause.
#[test]
fn inc_i_182_m3_req_ldp_019_addbond_untailed_refused_for_auth() {
    let unit = Network::Devnet.bond_unit();
    let lock = BEST + 100_000;
    let own = Spec {
        victim: *payer().public_key(),
        ..spec(Kind::AddBondUntailed, Payout::Normal, lock)
    };
    let need = unit * u64::from(BONDS) + FEE;
    let tx = build(&own, &payer(), &coin(need + 7)).expect("own AddBond with change");
    assert_eq!(tx.tx_type, TxType::AddBond);
    assert_eq!(tx.inputs.len(), 1, "only the payer's coin");
    assert!(
        input_signed_by(&tx, 0, &payer()),
        "the coin is signed by the payer"
    );
    let pk = *payer().public_key();
    let shape: Vec<_> = tx
        .outputs
        .iter()
        .map(|o| (o.output_type, o.amount, o.pubkey_hash, o.lock_until))
        .collect();
    let bond = (OutputType::Bond, unit, bond_owner(&pk), lock);
    assert_eq!(
        shape,
        vec![bond, bond, (OutputType::Normal, 7, bond_owner(&pk), 0)],
        "two Bond outputs of one unit to the producer, then the change to the payer"
    );
    assert_eq!(
        tx.extra_data,
        AddBondData::new(pk, BONDS).to_bytes(),
        "AddBond payload with no tail"
    );
    assert_eq!(
        code_of(&validate_transaction(&tx, &ctx(0, 0))),
        CODE,
        "an untailed AddBond is refused for the producer-auth reason"
    );
    let mut tailed = tx.clone();
    sign_producer_auth(&mut tailed, &genesis(), &payer());
    assert_eq!(
        code_of(&validate_transaction(&tailed, &ctx(0, 0))),
        "ok",
        "NON-VACUITY: the same AddBond with its producer's tail is valid"
    );
    let exact = build(&own, &payer(), &coin(need)).expect("exact coin");
    assert_eq!(
        exact.outputs.len(),
        BONDS as usize,
        "no change output when change is 0"
    );
}

// REQ-LDP-003 — Decision: a failure means the forge accepts an unknown kind or payout, or signs a transaction its coin cannot pay, so a runner typo or an unfunded payer yields a misleading refusal.
#[test]
fn inc_i_182_m3_req_ldp_003_parse_and_insufficient_coin() {
    assert_eq!(Kind::parse("forced-exit"), Some(Kind::ForcedExit));
    assert_eq!(
        Kind::parse("forced-exit-wrong-tail"),
        Some(Kind::ForcedExitWrongTail)
    );
    assert_eq!(Kind::parse("addbond-untailed"), Some(Kind::AddBondUntailed));
    assert_eq!(Payout::parse("normal"), Some(Payout::Normal));
    assert_eq!(Payout::parse("unbonding"), Some(Payout::Unbonding));
    for bad in [
        "",
        "forced_exit",
        "FORCED-EXIT",
        "unsigned",
        "forced-exit; rm -rf /",
    ] {
        assert_eq!(Kind::parse(bad), None, "kind {bad:?} must be refused");
    }
    for bad in ["", "bond", "Unbonding", "normal\n"] {
        assert_eq!(Payout::parse(bad), None, "payout {bad:?} must be refused");
    }
    let lock = BEST + period() + 20;
    for kind in [Kind::ForcedExit, Kind::ForcedExitWrongTail] {
        for amount in [0, FEE] {
            let r = build(
                &spec(kind, Payout::Unbonding, lock),
                &payer(),
                &coin(amount),
            );
            assert!(
                r.is_err(),
                "{kind:?}: coin {amount} <= fee must be Err, got Ok"
            );
        }
        let r = build(
            &spec(kind, Payout::Unbonding, lock),
            &payer(),
            &coin(FEE + 1),
        );
        assert_eq!(
            r.map(|t| t.outputs[0].amount),
            Ok(1),
            "{kind:?}: coin fee+1 pays 1"
        );
    }
    let need = Network::Devnet.bond_unit() * u64::from(BONDS) + FEE;
    let r = build(
        &spec(Kind::AddBondUntailed, Payout::Normal, lock),
        &payer(),
        &coin(need - 1),
    );
    assert!(r.is_err(), "AddBond: a coin one unit short must be Err");
}

const MAIN_SRC: &str = include_str!("../../examples/gs028_forge_tx/main.rs");

fn code_lines(src: &str) -> Vec<&str> {
    src.lines()
        .filter(|l| !l.trim_start().starts_with("//"))
        .collect()
}

fn guard_violations(src: &str) -> Vec<&'static str> {
    let lines = code_lines(src);
    let c = lines.join("\n");
    let first = |n: &str| c.find(n);
    let printing = [
        "print!(",
        "println!(",
        "eprint!(",
        "eprintln!(",
        "write!(",
        "writeln!(",
    ];
    let secret_print = lines.iter().any(|l| {
        let low = l.to_lowercase();
        printing.iter().any(|m| l.contains(m))
            && ["private", "secret", "seed"]
                .iter()
                .any(|w| low.contains(w))
    });
    let foreign_url = c.match_indices("http").any(|(i, _)| {
        let rest = &c[i..];
        rest.starts_with("https://")
            || (rest.starts_with("http://") && !rest["http://".len()..].starts_with("127.0.0.1"))
    });
    let host_flag = [
        "\"--host\"",
        "\"--url\"",
        "\"--rpc\"",
        "\"--rpc-url\"",
        "\"--addr\"",
        "\"--endpoint\"",
    ]
    .iter()
    .any(|f| c.contains(f));
    let params_first = matches!(
        (first("getNetworkParams"), first("sendTransaction")),
        (Some(p), Some(s)) if p < s
    );
    [
        ("loopback host literal", c.contains("127.0.0.1")),
        ("no host flag", !host_flag),
        ("no non-loopback URL", !foreign_url),
        ("getNetworkParams before sendTransaction", params_first),
        ("devnet check", c.contains("\"devnet\"")),
        ("guard exit 3", c.contains("exit(3)")),
        ("no secret print", !secret_print),
        (
            "no home-dir key path",
            !(c.contains("home_dir") || c.contains(".doli") || c.contains("\"HOME\"")),
        ),
    ]
    .into_iter()
    .filter(|(_, ok)| !ok)
    .map(|(what, _)| what)
    .collect()
}

// REQ-LDP-004 — Decision: a failure means the forge can reach a non-devnet or non-loopback node, sends before checking the network, or prints key material, so a typo in a runner could push a forged transaction outside the throwaway devnet.
#[test]
fn inc_i_182_m3_req_ldp_004_forge_main_guards() {
    let clean = "const H: &str = \"http://127.0.0.1\";\nfn main() { if rpc(\"getNetworkParams\") != \"devnet\" { std::process::exit(3); }\n rpc(\"sendTransaction\"); }";
    assert!(
        guard_violations(clean).is_empty(),
        "NON-VACUITY: the clean shape passes"
    );
    let planted = "fn main() { let h = arg(\"--host\"); rpc(\"sendTransaction\"); rpc(\"getNetworkParams\");\n post(\"http://10.0.0.7:8500\");\n eprintln!(\"seed {}\", s);\n let k = std::fs::read(dirs::home_dir().unwrap().join(\".doli/w.json\")); }";
    assert_eq!(
        guard_violations(planted).len(),
        8,
        "NON-VACUITY: every planted guard violation is flagged: {:?}",
        guard_violations(planted)
    );
    let bad = guard_violations(MAIN_SRC);
    assert!(
        bad.is_empty(),
        "examples/gs028_forge_tx/main.rs violates: {bad:?}"
    );
}
