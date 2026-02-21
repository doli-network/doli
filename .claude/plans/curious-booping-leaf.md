# Plan: Document All CLI Operations

## Context

CLAUDE.md has architecture/dev info but zero CLI operation reference. The existing skills (`network-setup`, `auto-update`, `doli-network`) cover specific domains but there's no single place documenting how to perform common operations like checking wallet balances, adding bonds, upgrading nodes, voting on updates, etc. After reviewing all 32 `doli` CLI commands, 11+ `doli-node` commands, and 20 RPC methods from source code, the gaps are clear.

## Approach: New Skill + CLAUDE.md Cross-Reference

**Why a new skill instead of expanding CLAUDE.md:**
- CLAUDE.md is 291 lines of architecture/dev rules — adding 400+ lines of operations bloats it
- Existing pattern: domain skills (`network-setup` = 773 lines, `auto-update` = 669 lines)
- Keeps CLAUDE.md as "project brain", new skill as "operations runbook"

## File 1: `.claude/skills/doli-operations/SKILL.md` (NEW, ~450 lines)

Task-oriented runbook covering every operation. Sections:

### 1. Network Quick Reference (~25 lines)
Single table: network → ports, bond unit, data dir, address prefix, genesis

### 2. Binary Disambiguation (~15 lines)
When to use `doli` (wallet CLI) vs `doli-node` (node binary). Key overlaps: `update` and `maintainer` exist in both — `doli` uses RPC, `doli-node` works offline.

### 3. Wallet Management (~50 lines)
- Create wallet: `doli -w <path> new`
- Show info/address: `doli -w <path> info`, `doli addresses`
- Check balance: `doli -r <rpc> balance` (confirmed/unconfirmed/immature)
- Send coins: `doli -r <rpc> send <pubkey_hash> <amount>` (with UTXO double-spend warning)
- Export/import: `doli export <path>`, `doli import <path>`
- Sign/verify messages

### 4. Producer Operations (~80 lines)
- Register: `doli producer register -b <bonds>` (end-to-end with funding)
- Check status: `doli producer status`
- List all: `doli producer list [--active]`
- Add bonds: `doli producer add-bond -c <count>`
- Request withdrawal: `doli producer request-withdrawal -c <count> [-d <dest>]`
- Claim withdrawal: `doli producer claim-withdrawal [-i <index>]`
- Exit: `doli producer exit [--force]` (with penalty table reference)
- Slash: `doli producer slash --block1 <hash> --block2 <hash>`

### 5. Rewards (~30 lines)
- `doli rewards info` — current epoch, block reward, progress
- `doli rewards list` — claimable epochs
- `doli rewards claim <epoch>` / `doli rewards claim-all`
- `doli rewards history`
- Note: coinbase maturity = 100 blocks before spendable

### 6. Node Operations (~60 lines)
- Run node: `doli-node [--network <net>] run [flags]` — full flag table
- Init: `doli-node init`
- Recover: `doli-node recover [--yes]` — rebuilds UTXO + producer set from blocks
- Upgrade: `doli-node upgrade [--version <ver>] [--yes]` — self-upgrade from GitHub
- Data directory structure reference

### 7. Update Governance (~60 lines)
Both `doli` and `doli-node` update commands side by side:
- Check: `doli-node update check`
- Status: `doli update status` / `doli-node update status`
- Vote: `doli update vote --veto` (via RPC) / `doli-node update vote --veto --key <key>` (offline)
- View votes: `doli update votes --version <ver>`
- Apply: `doli-node update apply [--force]`
- Rollback: `doli-node update rollback`
- Verify: `doli-node update verify --version <ver>`
- Vote weight formula: `bonds x seniority_multiplier`
- Veto threshold: 40%, veto period: 7 days mainnet / 60s devnet

### 8. Maintainer Management (~40 lines)
- List: `doli maintainer list` / `doli-node maintainer list`
- Add: `doli-node maintainer add --target <pubkey> --key <key>` (3/5 multisig)
- Remove: `doli-node maintainer remove --target <pubkey> --key <key>`
- Sign proposal: `doli-node maintainer sign --proposal-id <id> --key <key>`
- Verify: `doli-node maintainer verify --pubkey <key>`

### 9. Release Signing (~20 lines)
- `doli-node release sign --key <key> --version <ver> [--hash <sha256>]`
- Workflow: sign → collect 3/5 → publish to GitHub release

### 10. Devnet Management (~15 lines)
Brief summary + "See `network-setup` skill for full devnet guide":
- `doli-node devnet init --nodes N`
- `doli-node devnet start/stop/status/clean`
- `doli-node devnet add-producer [--count N]`

### 11. Cross-References (~10 lines)
- Devnet deep dive → `network-setup` skill
- Auto-update internals → `auto-update` skill
- RPC API → `docs/rpc_reference.md`
- Full CLI reference → `docs/cli.md`

## File 2: `CLAUDE.md` (EDIT — add ~15 lines)

Add a new subsection after the existing "Commands (Wrapped)" section:

```markdown
### CLI Operations Reference

Two binaries: `doli` (wallet/CLI) and `doli-node` (node daemon).

| Binary | Global Flags | Purpose |
|--------|-------------|---------|
| `doli` | `-w <wallet>`, `-r <rpc_url>` | Wallet, producer, rewards, governance |
| `doli-node` | `--network <net>`, `--data-dir <dir>` | Node, sync, update, devnet, recovery |

**Complete operations guide**: `.claude/skills/doli-operations/SKILL.md`

See also: `docs/cli.md` (full reference), `docs/rpc_reference.md` (RPC API)
```

## Verification

1. Every command in the new skill matches the actual clap parser in source code
2. No duplication with `network-setup` skill (devnet section is just a pointer)
3. No duplication with `auto-update` skill (governance section covers user operations, not implementation milestones)
4. CLAUDE.md stays concise (<310 lines after edit)
