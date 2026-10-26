//! GS-028 (INC-I-182) transaction forge: build a producer-changing transaction
//! paid and signed by a third party, then submit the same hex to every devnet
//! port. The shape is built by the pure `shape` module; this file only loads the
//! payer key, pulls a coin and the genesis hash over RPC, guards the target, and
//! reports each submit. Devnet-guarded and never installed.

mod shape;

use std::time::Duration;

use crypto::{Hash, KeyPair, PrivateKey, PublicKey};
use doli_core::transaction::Transaction;
use doli_core::validation::bond_owner;
use serde_json::{json, Value};
use shape::{build, Coin, Kind, Payout, Spec};

fn arg(args: &[String], key: &str) -> Option<String> {
    args.iter()
        .position(|a| a == key)
        .and_then(|i| args.get(i + 1))
        .cloned()
}

fn flag(args: &[String], key: &str) -> bool {
    args.iter().any(|a| a == key)
}

fn die(code: i32, msg: &str) -> ! {
    eprintln!("gs028 forge: {msg}");
    std::process::exit(code);
}

fn req(args: &[String], key: &str) -> String {
    arg(args, key).unwrap_or_else(|| die(2, "missing required argument"))
}

/// Keypair of the wallet's first address. No key byte is ever printed.
fn load_wallet_keypair(path: &str) -> KeyPair {
    let text = std::fs::read_to_string(path).unwrap_or_else(|_| die(2, "key file unreadable"));
    let wallet: Value =
        serde_json::from_str(&text).unwrap_or_else(|_| die(2, "key file is not wallet JSON"));
    let hex_key = wallet["addresses"][0]["private_key"]
        .as_str()
        .unwrap_or_else(|| die(2, "key file has no first address"));
    let key = PrivateKey::from_hex(hex_key).unwrap_or_else(|_| die(2, "key file is malformed"));
    KeyPair::from_private_key(key)
}

fn url_for(port: u16) -> String {
    format!("http://127.0.0.1:{port}/")
}

async fn rpc(client: &reqwest::Client, url: &str, method: &str, params: Value) -> Option<Value> {
    let request = json!({ "jsonrpc": "2.0", "method": method, "params": params, "id": 1 });
    let resp = client
        .post(url)
        .json(&request)
        .timeout(Duration::from_secs(5))
        .send()
        .await
        .ok()?;
    resp.json::<Value>().await.ok()
}

/// Every port must answer `getNetworkParams` with `network == "devnet"` before any
/// submit. A missing answer exits 4; a non-devnet answer exits 3 with nothing sent.
async fn guard_devnet(client: &reqwest::Client, ports: &[u16]) -> u64 {
    let mut bond_unit = 0u64;
    for &port in ports {
        let Some(resp) = rpc(client, &url_for(port), "getNetworkParams", json!({})).await else {
            die(4, "a port did not answer getNetworkParams");
        };
        let net = resp["result"]["network"].as_str();
        if net != Some("devnet") {
            eprintln!("gs028 forge: a port is not devnet; nothing sent");
            std::process::exit(3);
        }
        if bond_unit == 0 {
            bond_unit = resp["result"]["bondUnit"].as_u64().unwrap_or(0);
        }
    }
    bond_unit
}

async fn genesis_hash(client: &reqwest::Client, port: u16) -> Hash {
    let resp = rpc(client, &url_for(port), "getChainInfo", json!({}))
        .await
        .unwrap_or_else(|| die(4, "getChainInfo did not answer"));
    resp["result"]["genesisHash"]
        .as_str()
        .and_then(Hash::from_hex)
        .unwrap_or_else(|| die(4, "getChainInfo carried no genesis hash"))
}

/// The payer's largest spendable Normal coin on the first port.
async fn payer_coin(client: &reqwest::Client, port: u16, payer: &PublicKey) -> Coin {
    let address = bond_owner(payer).to_hex();
    let resp = rpc(
        client,
        &url_for(port),
        "getUtxos",
        json!({ "address": address }),
    )
    .await
    .unwrap_or_else(|| die(4, "getUtxos did not answer"));
    let rows = resp["result"].as_array().cloned().unwrap_or_default();
    rows.into_iter()
        .filter(|u| {
            u["outputType"].as_str() == Some("normal") && u["spendable"].as_bool() == Some(true)
        })
        .filter_map(|u| {
            Some(Coin {
                tx_hash: Hash::from_hex(u["txHash"].as_str()?)?,
                index: u32::try_from(u["outputIndex"].as_u64()?).ok()?,
                amount: u["amount"].as_u64()?,
            })
        })
        .max_by_key(|c| c.amount)
        .unwrap_or_else(|| die(2, "the payer has no spendable coin"))
}

fn errtx_code(message: &str, reason: &str) -> String {
    for field in [message, reason] {
        if let Some(start) = field.find("ERRTX-") {
            let code: String = field[start..]
                .chars()
                .take_while(|c| c.is_ascii_alphanumeric() || *c == '-')
                .collect();
            if code.len() > "ERRTX-".len() {
                return code;
            }
        }
    }
    "none".to_string()
}

fn printable(message: &str) -> String {
    message
        .chars()
        .map(|c| if c == '"' { '\'' } else { c })
        .filter(|c| c.is_ascii_graphic() || *c == ' ')
        .take(160)
        .collect()
}

async fn submit(client: &reqwest::Client, port: u16, tx_hex: &str) {
    let Some(resp) = rpc(
        client,
        &url_for(port),
        "sendTransaction",
        json!({ "tx": tx_hex }),
    )
    .await
    else {
        println!("SUBMIT port={port} accepted=0 code=none err=\"no answer\"");
        return;
    };
    if resp.get("result").is_some() {
        println!("SUBMIT port={port} accepted=1 code=none err=\"\"");
        return;
    }
    let message = resp["error"]["message"].as_str().unwrap_or("");
    let reason = resp["error"]["data"]["reason"].as_str().unwrap_or("");
    let code = errtx_code(message, reason);
    println!(
        "SUBMIT port={port} accepted=0 code={code} err=\"{}\"",
        printable(message)
    );
}

#[tokio::main]
async fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let kind = Kind::parse(&req(&args, "--kind")).unwrap_or_else(|| die(2, "unknown --kind"));
    let payout =
        Payout::parse(&req(&args, "--payout")).unwrap_or_else(|| die(2, "unknown --payout"));
    let payer = load_wallet_keypair(&req(&args, "--payer-key"));
    let victim =
        PublicKey::from_hex(&req(&args, "--victim")).unwrap_or_else(|_| die(2, "bad --victim hex"));
    let bonds: u32 = req(&args, "--bonds")
        .parse()
        .unwrap_or_else(|_| die(2, "bad --bonds"));
    let lock_until: u64 = req(&args, "--lock-until")
        .parse()
        .unwrap_or_else(|_| die(2, "bad --lock-until"));
    let fee: u64 = arg(&args, "--fee")
        .unwrap_or_else(|| "1".to_string())
        .parse()
        .unwrap_or_else(|_| die(2, "bad --fee"));
    let ports: Vec<u16> = req(&args, "--ports")
        .split(',')
        .filter(|s| !s.is_empty())
        .map(|s| s.parse().unwrap_or_else(|_| die(2, "bad --ports")))
        .collect();
    if ports.is_empty() {
        die(2, "no --ports");
    }
    let dry_run = flag(&args, "--dry-run");
    let payout_name = match payout {
        Payout::Normal => "normal",
        Payout::Unbonding => "unbonding",
    };

    let client = reqwest::Client::new();
    let bond_unit = guard_devnet(&client, &ports).await;
    let genesis = genesis_hash(&client, ports[0]).await;
    let coin = payer_coin(&client, ports[0], payer.public_key()).await;

    let spec = Spec {
        kind,
        victim,
        bonds,
        payout,
        lock_until,
        fee,
        bond_unit,
        genesis,
    };
    let tx: Transaction =
        build(&spec, &payer, &coin).unwrap_or_else(|e| die(2, &format!("build: {e}")));
    let tx_hex = hex::encode(tx.serialize());

    println!(
        "FORGED kind={} txid={} victim={} payout={payout_name} lock_until={lock_until}",
        req(&args, "--kind"),
        tx.hash().to_hex(),
        &victim.to_hex()[..8],
    );

    if dry_run {
        return;
    }
    for &port in &ports {
        submit(&client, port, &tx_hex).await;
    }
}
