# Faucet Bot

Automated DOLI faucet that processes GitHub issue requests every 5 minutes.

## How it works

1. User opens a GitHub issue using the [faucet request template](https://github.com/doli-network/doli/issues/new?template=faucet-request.yml)
2. Bot polls open issues every 5 minutes via GitHub API
3. Bot validates the address and sends 10.01 DOLI (1 bond + fees)
4. Bot comments with TX hash and next steps, then closes the issue

## Validations

The bot checks 5 conditions before sending. All must pass:

| Check | Rejects if |
|-------|-----------|
| Address format | Not `doli1` + 58 bech32 chars (63 total) |
| Address length | Not exactly 63 characters |
| Node verification | `getBalance` RPC returns an error for the address |
| Existing balance | Address already has any DOLI (confirmed, bonded, or immature) |
| Already funded | Address appears in `sent.log` (prevents re-sends) |
| Already registered | Address is an active or pending producer |

Rejected requests are closed with an explanation comment.

## Infrastructure

| Component | Location |
|-----------|----------|
| Script | `ai2:/mainnet/faucet/faucet-bot.sh` |
| Cron | Every 5 min on ai2: `*/5 * * * *` |
| Log | `ai2:/var/log/doli/mainnet/faucet-bot.log` |
| Sent database | `ai2:/mainnet/faucet/sent.log` |
| GitHub token | `ai2:/mainnet/faucet/.gh-token` (fine-grained, issues only) |
| Faucet wallet | `ai3:/mainnet/faucet/keys/wallet.json` |
| Faucet vault | `ai3:/mainnet/faucet-vault/keys/wallet.json` |

The bot runs on **ai2** but sends DOLI from **ai3** via SSH (`ssh ai3 "doli -w ... send ..."`). The faucet wallet's RPC is the ai3 seed at port 8500.

## Faucet wallets

| Wallet | Address | Purpose |
|--------|---------|---------|
| Faucet (operational) | `doli1em9zhehsseaq2ca4xxfkwevpwqfv9zn252ya24ska8t08dtc9u7szr064k` | Daily sends |
| Faucet-vault (reserve) | `doli1c8aukqm4j209s2g5h7uucmy99ff8f5csy3r9526e2wwvm3l549zqqc6jl7` | Refill faucet when low |

Funded at block 26,979 (~3 days after genesis). 250 DOLI from each founding producer (N1-N6), 1,500 DOLI total.

## sent.log format

```
2026-03-24T02:50:00Z doli1alzf...pjl62 270660437d...eb46 issue#11
```

Fields: `timestamp address tx_hash issue_reference`

One line per send. The bot greps this file to prevent duplicate sends.

## GitHub token

Fine-grained personal access token with:
- **Repository**: `doli-network/doli` only
- **Permissions**: Issues (Read and write), Metadata (Read-only)
- **Name**: `doli-faucet-bot`

To rotate: generate new token at https://github.com/settings/tokens?type=beta, then:
```bash
ssh ai2 "echo 'NEW_TOKEN' > /mainnet/faucet/.gh-token && chmod 600 /mainnet/faucet/.gh-token"
```

## Manual operations

### Check bot status
```bash
ssh ai2 "tail -20 /var/log/doli/mainnet/faucet-bot.log"
```

### Run manually
```bash
ssh ai2 "bash /mainnet/faucet/faucet-bot.sh"
```

### Check sent history
```bash
ssh ai2 "cat /mainnet/faucet/sent.log"
```

### Check faucet balance
```bash
ssh ai3 "doli -w /mainnet/faucet/keys/wallet.json balance"
```

### Refill faucet from vault
```bash
ssh ai3 "echo y | doli -w /mainnet/faucet-vault/keys/wallet.json send doli1em9zhehsseaq2ca4xxfkwevpwqfv9zn252ya24ska8t08dtc9u7szr064k AMOUNT"
```

## Failure modes

| Failure | Bot behavior |
|---------|-------------|
| GitHub API down | Bot exits silently, retries next cron cycle |
| ai3 unreachable | SSH fails, bot comments "send failed" on the issue (does not close) |
| Faucet wallet empty | `doli send` fails, bot comments "send failed" on the issue |
| Duplicate address | Bot comments "already funded" with previous TX hash, closes issue |
| Invalid address | Bot comments with error explanation, closes issue |
| GitHub token expired | Bot exits with "No token" error in log |
