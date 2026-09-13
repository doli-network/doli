# Faucet

The mainnet DOLI faucet gives a new participant enough DOLI to register as a block producer.

**Discord is the only intake.** Since 2026-09-13, you request DOLI with the `/faucet` command in the
[DOLI Discord](https://discord.gg/uGzCvxGYC). The GitHub issue form is retired. The faucet does not
process GitHub issues.

## What you get

| Item | Value |
|------|-------|
| Amount | **10.000001 DOLI** (1,000,000,100 base units) |
| Breakdown | 1 bond unit (10 DOLI) + 100 base units for the registration fee |
| Limit | One claim per person, for life |
| Destination | A new, empty address |

## How to claim

1. Install a node, create a wallet, and let the node sync. See the
   [installation guide](https://doli.network/guide.html).
2. Join the [DOLI Discord](https://discord.gg/uGzCvxGYC).
3. Run `/faucet` in the server. The bot replies with a private link. Only you can see it.
4. Open the link and press the button. The claim form opens.
5. Submit your address (`doli info`) in the form. The form asks you to prove that you control the
   address.
6. Wait for a human to approve the claim. The faucet sends the DOLI after approval.
7. Check your balance with `doli balance`. The bot does not yet send a Discord message when it pays.
8. Register: `doli producer register --bonds 1`.

If the private link shows *Preparing your claim* for more than about a minute, run `/faucet` again.

## Eligibility rules

All rules must pass. A human approves every claim, including claims that pass every rule.

| Rule | Requirement |
|------|-------------|
| Discord account age | At least 1 year |
| Server membership | Member of the DOLI Discord for at least 7 days |
| One per person | A Discord account that received DOLI cannot claim again. One open claim at a time. |
| One per address | An address that received DOLI cannot receive it again |
| Empty address | The address total balance must be 0 |
| Not a producer | The address must not be a registered producer |
| Valid address | A mainnet `doli1...` address with a valid checksum |
| Proof of control | You prove that you control the address before the claim opens |
| Human check | The form runs an anti-bot check |

## How it works

1. The Discord bot receives `/faucet` and opens a claim.
2. The web form takes the address and the proof of control.
3. Automatic checks apply the eligibility rules.
4. A human approver reviews the claim and approves or rejects it.
5. The faucet pays from its hot wallet with a normal `doli send`. The payment is on-chain and
   anyone can verify it.
6. Operators refill the hot wallet by hand from a separate reserve. The faucet software cannot
   reach the reserve.

## History

- At block 26,979 (~3 days after genesis), the founding producers funded the faucet from their own
  earned rewards: 250 DOLI each, 1,500 DOLI total.
- Until 2026-09-13, the faucet took requests through a GitHub issue form. That intake is closed.
- On 2026-09-13, the Discord faucet paid its first mainnet claim.
