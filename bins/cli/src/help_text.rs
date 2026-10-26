//! Agent-facing help text: the root `doli --help` page and the per-command
//! DANGER notes. `help_text_tests.rs` pins each code-derived number.

pub(crate) const ROOT_LONG_ABOUT: &str = r#"DOLI wallet and node CLI

DOLI is a proof-of-stake blockchain with a UTXO model. Producers lock bonds,
produce blocks, attest, and earn rewards once per epoch."#;

pub(crate) const ROOT_AFTER_LONG_HELP: &str = r#"FACTS
  Units     1 DOLI = 100,000,000 base units (8 decimals).
            CLI amounts and --fee are in DOLI. RPC results are in base units.
  Network   The default is MAINNET. Always pass -n mainnet|testnet|devnet.
  RPC       Default http://127.0.0.1:8500 (testnet :18500, devnet :28500).
            Use your own synced node. The CLI trusts what the node returns.
  Wallet    -w <path>, else $DOLI_WALLET_FILE, else <data-dir>/wallet.json.
            wallet.json holds PLAIN-TEXT private keys.
  Bond      1 bond = 10 DOLI on mainnet. Maximum 3,000 bonds per producer.
  Time      1 slot = 10 s. 1 epoch = 360 blocks = 1 hour.
  Rewards   Each block adds 1 DOLI to a reward pool. At the end of each epoch
            the pool goes, by bond weight, to producers that attested in at
            least 54 of the 60 minutes. Fewer minutes = 0 for that epoch.
  Delays    Register, add-bond, withdrawal, delegation and rotate-bls apply at
            the next epoch boundary. Rewards are spendable after 6 blocks.

NEVER DO THESE - EACH ONE CAN LOSE DOLI
  1. Do not withdraw bonds without checking the penalty first.
     A bond younger than 1 year loses 75%. 1-2 years: 50%. 2-3 years: 25%.
     3 years or more: 0%. The penalty is burned. request-withdrawal does not
     ask for confirmation. From mainnet height 806,859 the payout is also
     locked for 60,480 blocks (about 7 days).
     Do this: doli producer simulate-withdrawal --count N
     To stop operating a node, delegate the bond. Do not withdraw it.
  2. Do not run one producer key on two nodes at the same time.
     From mainnet height 806,859, this burns 100% of the bonds and bars the
     key for life.
     Do this: stop the old node and confirm it is stopped before you start
     the new one.
  3. Do not pass --fee to send or channel. It is in DOLI: --fee 1 pays
     1 DOLI. The fee above the minimum is destroyed.
     Do this: leave out --fee. The CLI pays the minimum (1 base unit).
  4. Do not put a public key where an address is expected. Some commands
     accept 64-hex text as an address hash. Coins sent to a public key are
     lost forever.
     Do this: use the doli1... address that "doli info" shows.
  5. Do not use "doli spend" without a change output. All value that you do
     not list as an output is destroyed as fee.
     Do this: --output for the recipient AND --output for your own change.
  6. Do not lock coins with a bare timelock(...) or hashlock(...). Anyone can
     spend them when the condition becomes true.
     Do this: vesting(<your address>, <height>).
  7. Do not wipe or snap a data directory that holds wallet files.
     wipe and snap keep only wallet.json and wallet.seed.txt by exact name.
     They delete all other files, for example producer.json or backups.
     "wipe" ignores -n: use "doli wipe --network <net>".
     Do this: keep wallets outside the data directory (-w <path>).
  8. Do not trust the 24 words as your only backup. Keys made by
     "doli address", "add-bls", "import-bls", and BLS keys of wallets made
     before version 3, cannot come back from the words.
     Do this: back up wallet.json after each of these commands.
  9. Do not run new, init, export, import-bls or bridge-swap where the output
     is logged. They print or write secrets.
 10. Do not run "doli upgrade" without sudo on a host that runs the node as a
     service.

MOVE A PRODUCER TO A NEW MACHINE
  1. Stop the node on the old machine. Confirm that it is stopped.
  2. Copy wallet.json to the new machine. Start the node with it. Done.
  If you have only the 24 words:
     doli restore
     (start the node and let it sync)
     doli producer rotate-bls --yes
  rotate-bls costs 3 base units. If the keys already match, it stops with
  "the wallet BLS key is already the key on chain" and costs nothing.
  Without this step the node runs but can earn 0 rewards.

EXIT CODES
  Most commands exit 0 when you cancel their prompt.
  "producer exit" without --force is a dry run and exits 0.
  Read the output. Do not rely only on the exit code.

MORE
  doli <command> --help shows the DANGER notes for that command.
  Agent skills: $DOLI_SKILLS_DIR or ~/.doli/skills (wallet, cli, delegation,
  bls-key-recovery, network-setup)."#;

pub(crate) const REQUEST_WITHDRAWAL_DANGER: &str = r#"DANGER:
This command does not ask for confirmation. It sends the transaction at once.
The vesting penalty is burned. Nobody gets it. Penalty by bond age:
under 1 year 75%, 1-2 years 50%, 2-3 years 25%, 3 years or more 0%.
Do this: run "doli producer simulate-withdrawal --count N" first.
From mainnet height 806,859 the payout is locked for 60,480 blocks
(about 7 days). It can go only to your own address.
Do this: plan for the delay. Leave out --destination.
--destination reads 64-hex text as an address hash. A public key there
loses the payout forever (INC-I-267).
Do this: if you use --destination, give a doli1... address.
To stop operating a node, you do not need to withdraw the bonds.
Do this: delegate the bonds with "doli producer delegate"."#;

pub(crate) const EXIT_DANGER: &str = r#"DANGER:
"exit --force" withdraws ALL your bonds. The vesting penalty is burned:
under 1 year 75%, 1-2 years 50%, 2-3 years 25%, 3 years or more 0%.
Do this: run "doli producer simulate-withdrawal --count N" first.
Without --force this command is a dry run. It exits 0 and sends nothing.
Do this: read the output. Do not rely only on the exit code.
If you register again later, the new bonds start again at age 0, in the
highest penalty tier.
Do this: keep the bonds if you plan to produce again.
From mainnet height 806,859 the payout is locked for 60,480 blocks
(about 7 days).
Do this: plan for the delay.
To stop operating a node, you do not need to exit.
Do this: delegate the bonds with "doli producer delegate"."#;

pub(crate) const SEND_DANGER: &str = r#"DANGER:
--fee is in DOLI, not base units. "--fee 1" pays 1 DOLI. The fee above the
minimum is destroyed (INC-I-265).
Do this: leave out --fee. The CLI then pays the minimum fee.
A bare "timelock(H)" or "hashlock(H)" condition lets anyone spend the coins.
Do this: use "vesting(<your doli1 address>, <height>)".
If you lose the keys of a multisig, the coins stay locked forever.
Do this: back up every multisig key before you send.
Addresses in a condition accept 64-hex text as an address hash. A public
key there locks the coins forever (INC-I-267).
Do this: use doli1... addresses only.
A cancelled prompt exits 0.
Do this: read the output."#;

pub(crate) const SPEND_DANGER: &str = r#"DANGER:
"doli spend <utxo> <to> <amount>" makes ONE output and NO change output.
The rest of the input is destroyed as fee (INC-I-264).
Do this: use --output for the recipient AND --output for your own change.
Make the outputs add up to the input minus the minimum fee.
The fee is always the input minus the outputs. --fee does not change it.
--yes skips the high-fee warning.
Do this: do not pass --yes.
Recipients accept 64-hex text as an address hash. A public key there loses
the coins forever (INC-I-267).
Do this: use doli1... addresses."#;

pub(crate) const WIPE_DANGER: &str = r#"DANGER:
This command has its own --network flag. It defaults to mainnet and ignores
the global -n. "doli -n testnet wipe" targets the MAINNET data directory
(INC-I-266).
Do this: run "doli wipe --network <net>".
It keeps only these names in the data directory:
keys, .env, wallet.json, wallet.seed.txt, node_key, config.toml
It deletes all other files, for example producer.json or wallet backups.
Do this: move other wallet files out of the data directory first.
A cancelled prompt exits 0.
Do this: read the output."#;

pub(crate) const SNAP_DANGER: &str = r#"DANGER:
This command does not ask for confirmation. It stops the node service and
wipes the data directory BEFORE it downloads the snapshot. If the snapshot
check fails after the wipe, the node has no chain data.
Do this: run it only when you accept a full resync.
It keeps only these names in the data directory:
keys, .env, wallet.json, wallet.seed.txt, node_key, config.toml
It deletes all other files, for example producer.json or wallet backups.
Do this: move other wallet files out of the data directory first.
--trust skips the check that several seeds agree.
Do this: do not use --trust on mainnet."#;

pub(crate) const RESTORE_DANGER: &str = r#"DANGER:
restore always makes a version 3 wallet. Its BLS key comes from the words.
A wallet made before version 3 had a different BLS key. The node then runs
but earns 0 rewards.
Do this: after the node syncs, run "doli producer rotate-bls --yes".
rotate-bls costs 3 base units. If the keys already match, it stops and
costs nothing."#;

pub(crate) const NEW_DANGER: &str = r#"DANGER:
This command prints the 24 words to stdout. It also writes them to a
.seed.txt file next to the wallet.
Do this: do not run it where the output is logged.
Do this: write the words down. Then delete the .seed.txt file."#;

pub(crate) const INIT_DANGER: &str = r#"DANGER:
This command prints the 24 words to stdout. It also writes them to a
.seed.txt file next to the wallet.
Do this: do not run it where the output is logged.
--force overwrites the wallet file. The old keys are lost.
Do this: back up wallet.json before you use --force.
A cancelled prompt exits 0 and saves no wallet.
Do this: read the output."#;

pub(crate) const EXPORT_DANGER: &str = r#"DANGER:
This command writes every private key in plain text to the target file.
Do this: write it only to a safe place. Do not share the file.
--force overwrites the target file.
Do this: check the target path before you use --force."#;

pub(crate) const IMPORT_DANGER: &str = r#"DANGER:
--force overwrites the active wallet. Its keys are lost unless you have a
backup. This includes its BLS producer key.
Do this: back up the active wallet.json before you use --force."#;

pub(crate) const IMPORT_BLS_DANGER: &str = r#"DANGER:
The secret is a command-line argument. The shell history and the process
list can show it.
Do this: run it only on a private machine. Then clear the shell history.
--force replaces the BLS key in the wallet.
Do this: back up wallet.json first.
If the chain does not hold this key, the node earns 0 rewards.
Do this: pass --rpc. The CLI then compares the key with the chain first.
The 24 words cannot restore an imported key.
Do this: back up wallet.json after this command."#;

pub(crate) const ADDRESS_DANGER: &str = r#"DANGER:
This command makes a RANDOM key. The 24 words cannot restore it.
Do this: back up wallet.json after this command."#;

pub(crate) const ADD_BLS_DANGER: &str = r#"DANGER:
The new BLS key is random. The 24 words cannot restore it.
Do this: back up wallet.json after this command."#;

pub(crate) const UPGRADE_DANGER: &str = r#"DANGER:
On a host that runs the node as a service, the install needs root. Without
sudo it can stop after the CLI is replaced and before the node is replaced.
Do this: sudo doli upgrade
The upgrade restarts the node. The node loses attestation minutes while it
is down.
Do this: upgrade one node at a time.
The upgrade checks the release signatures against maintainer_state.bin.
Do this: never delete maintainer_state.bin. On a permission error, use sudo.
A cancelled prompt exits 0.
Do this: read the output."#;

pub(crate) const ROTATE_BLS_DANGER: &str = r#"DANGER:
The key change is irreversible. The fee is 3 base units.
The new key starts at the next epoch boundary.
Do this: back up wallet.json first. Rotate only to a BLS key that you keep.
If the keys already match, the command stops and costs nothing."#;

pub(crate) const DELEGATE_DANGER: &str = r#"DANGER:
This command does not ask for confirmation. The delegation applies at the
next epoch boundary.
Do this: check the delegatee public key with "doli producer list" first.
If the delegatee attests in fewer than 54 of the 60 minutes of an epoch,
the delegated bonds earn 0 for that epoch.
Do this: delegate only to a producer that stays online and attests."#;

pub(crate) const REVOKE_DELEGATION_DANGER: &str = r#"DANGER:
The revocation applies at the next epoch boundary. Until then the bonds
stay delegated.
If the delegatee attests in fewer than 54 of the 60 minutes of an epoch,
the delegated bonds earn 0 for that epoch.
Do this: revoke as soon as the delegatee stops attesting.
This command does not ask for confirmation."#;

pub(crate) const CHANNEL_OPEN_DANGER: &str = r#"DANGER:
The funding output is a 2-of-2 multisig with NO timeout. Every spend needs
both signatures. You cannot close the channel alone. If the peer does not
sign, the capacity is locked forever.
Do this: open a channel only with a peer who agrees to co-sign. Keep the
capacity small.
You fund the full capacity. The command does not ask for confirmation.
--fee is in DOLI, not base units. "--fee 1" pays 1 DOLI.
Do this: leave out --fee.
<PEER> accepts 64-hex text as an address hash. A public key there locks
the capacity forever (INC-I-267).
Do this: give the peer's doli1... address.
Without channels.json, "channel close" cannot find the channel.
Do this: back up wallet.json and channels.json after each open, pay, close."#;

pub(crate) const POOL_ADD_DANGER: &str = r#"DANGER:
The pool keeps ALL the DOLI and tokens that you add. Your LP shares follow
the smaller side of the pool ratio. The excess goes to the existing providers.
Do this: read "doli pool info" first. Add the tokens in the pool ratio:
tokens = DOLI x token reserve / DOLI reserve.
--doli is in DOLI. --tokens is in raw token units."#;

pub(crate) const POOL_SWAP_DANGER: &str = r#"DANGER:
--min-out defaults to 0. With 0 you have no slippage protection.
Do this: compute the expected output and always pass --min-out.
A token ticker is not unique. Only the asset ID identifies a token.
Do this: check the pool ID and the asset ID with "doli pool info"."#;

pub(crate) const BRIDGE_SWAP_DANGER: &str = r#"DANGER:
This command prints the secret preimage to stdout. It also saves it in
plain text under ~/.doli/swaps/.
The claim path needs only the preimage, not a signature. Anyone who sees
the preimage can take the locked DOLI.
Do this: do not run it where the output is logged. Protect ~/.doli/swaps/.
This command does not ask for confirmation.
Do this: keep the amount small."#;

pub(crate) const NFT_DANGER: &str = r#"DANGER:
"--sell" makes an unsigned offer. The buyer then needs the seller's wallet
file. That file holds ALL the seller's private keys.
Do this: sell with "--sell-sign". Never give a wallet file to anybody.
"--buy" and "--from" with an unsigned offer read the seller's wallet file.
Do this: buy only from a signed offer (made with --sell-sign).
For an encrypted NFT, a 64-hex --to is read as a PUBLIC KEY, not a hash.
Do this: give the recipient's doli1... address to --to.
--mint, --transfer, --buy and --from send at once. They do not ask."#;
