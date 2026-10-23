# Solana devnet verification

On 23 October 2026 the remote leg and the deterministic test strategy adapter were deployed to Solana devnet. The run that followed covered allocation, custody reconciliation, reporting, a close after a partial arrival, a recovery, a guardian-only close on a frozen leg, and six attacks that must fail. Every transaction is recorded in [`devnet-evidence.json`](devnet-evidence.json) and was re-verified from the devnet RPC at finalized commitment. The tables below are rendered from that file by the driver's `report` command.

This is a test deployment with disposable keys and a valueless test token. It is not an audit, and it does not exercise a real bridge; see [What this does not show](#what-this-does-not-show).

## Check it yourself

```bash
scripts/devnet/verify.sh docs/devnet-evidence.json
```

This needs only a Rust toolchain: no keys and no SOL. It confirms that the RPC serves devnet by checking its genesis hash. Then it re-fetches, at finalized commitment:

- **Each program account.** It must be owned by the upgradeable loader, be executable and have the recorded upgrade authority. Its program data must have the SHA-256 of the recorded local build, with nothing after the binary.
- **Each deploy transaction and each of the 29 scenario transactions.** Each must be at the recorded slot, invoke the recorded program, and succeed or fail as recorded. A rejection must carry the exact custom error code.

`SOLEVM_RPC_URL` selects another devnet RPC. The Solana CLI shows the same data, for example `solana program show 27zReYgZvFTaUSWwob4qirN2Yxekcm32mnQDpMzeTyAE --url devnet` or `solana confirm -v <signature> --url devnet`.

RPC nodes do not serve past account state, so `verify` cannot recheck before and after states. The driver checked those against live state during the run. It read every tracked account before and after each transaction, at a context slot no older than the previous transaction, and evaluated the 96 checks recorded in the evidence. After each rejection, it required every tracked account and token balance to be byte-for-byte unchanged. The snapshots are in each step's `before` and `after`.

The published record leaves out the wallet addresses of the disposable signers. The one exception is each program's upgrade authority, which `verify` compares with the chain. As on any Solana transaction, the signers remain visible in the transactions themselves.

## What the run shows

Amounts are base units of the 6-decimal test token.

- **Deployment and namespace.** Both programs run the exact local build, with the disposable deployer as upgrade authority. A namespace claim co-signed by any other key fails with `Unauthorized` and creates nothing (N01). The upgrade authority's claim binds the administrator, guardian, transport verifier, mint, custody and escrow (S04).
- **Allocation and custody.** Allocate A (1,000,000) is accepted at inbound sequence 1. The control-lane commitment advances to `keccak(domain, previous, message id)` (T01). The test tokens reach custody without touching the leg's accounting (T02). A permissionless reconcile classifies them as unattributed (T03), and attribution completes A (T04). After each attribution, custody equals attributed + recalled + unattributed.
- **Reporting.** RemoteReport epoch 1 is outbound sequence 1. Its bytes decode canonically and re-encode to the same message id, and the lane commitment advances by the same rule (T05). The epoch 2 report, at outbound sequence 4, states remote principal 1,900,000 (T12).
- **Close after a partial arrival.** Only 600,000 of allocation B's 1,000,000 arrives (T07, T08). Resolve(Close) credits 600,000, impairs 400,000, and publishes a TransportSettlement in the same instruction (T09).
- **Recovery.** The remaining 300,000 arrives after the close (T10). Resolve(Recover) credits it against the impairment pool, which falls from 400,000 to 100,000. The claim of 1,000,000 still equals credit 600,000 + impaired 100,000 + recovered 300,000. B keeps its 600,000 and the recovery record holds the 300,000, so nothing is counted twice (T11).
- **Replay and forgery.** Replaying Allocate A's exact bytes (N02), or the Recover message (N04), fails with `InvalidSequence`. A valid next Allocate signed by an attacker instead of the transport verifier fails with `Unauthorized` (N03).
- **Frozen leg.** The guardian freezes the leg (T14). Closing allocation C then fails with `Frozen` without the guardian (N05), and with `Unauthorized` when an attacker co-signs as guardian (N06). The guardian co-signed close succeeds with nothing credited (T15).

At the end, the transport ledger holds closed claims of 1,500,000 = credit 600,000 + impaired 600,000 + recovered 300,000. Custody holds 1,900,000, all of it attributed principal, and no transfer is open. The leg stays frozen, so this namespace accepts no new allocation; a new run uses a new label and namespace.

The most expensive transaction was T09's close, at 80,707 compute units. The 29 transactions cost 275,000 lamports in fees.

## Run

| Field | Value |
|---|---|
| Cluster | devnet (genesis `EtWTRABZaYq6iMfeYKouRu166VU2xqa1wcaWoxPkrZBG`) |
| RPC | `https://api.devnet.solana.com` |
| Run | `devnet-1`, 2026-10-23T10:23:33Z to 2026-10-23T10:25:42Z |
| Source | commit `7f38ba90b38aa136a38f88612f677ffde4eee608` plus uncommitted changes, tree `b3aaf0ee47347dc4467ce7187eeebdae051dc0bc` |
| Build | cargo-build-sbf 4.4.0, platform-tools v1.58, SBPFv0, `--cfg solevm_devnet` |
| Result | 29 transactions (23 expected successes, 6 expected rejections), 96/96 checks passed, all finalized: true |

## Programs

| Program | Program id | On-chain sha256 = local build | Deploy transaction |
|---|---|---|---|
| `solevm_remote_leg` | [`27zReYgZvFTaUSWwob4qirN2Yxekcm32mnQDpMzeTyAE`](https://explorer.solana.com/address/27zReYgZvFTaUSWwob4qirN2Yxekcm32mnQDpMzeTyAE?cluster=devnet) | `0de7a35acbcb41c33f71af50d0efce4803da1a297a21650876ddd1bc45df6dc3` (match) | [2bMZd9Yc…pJwcSsFw](https://explorer.solana.com/tx/2bMZd9YcTUWEAZduVMN9kfFP6MJkePzKwFwkCw6RfLSvkLqCeF6jnweKHfprS4sXSbxw2Tt7BfnW3VzDpJwcSsFw?cluster=devnet) slot 514214756 |
| `solevm_test_strategy` | [`8bFiHDwdAurdhTv5F9JaVD4QgdJ2XxEHRK2BAyiCyjZ8`](https://explorer.solana.com/address/8bFiHDwdAurdhTv5F9JaVD4QgdJ2XxEHRK2BAyiCyjZ8?cluster=devnet) | `02a978e2e299cb089edcd8d24396e9b9579cf01d34d2b5ba58d54a3eadbd39fb` (match) | [45T1Nfhk…TJaaaftH](https://explorer.solana.com/tx/45T1NfhkxHBW7K3wRWUKqfkFehqPPjLW9hbkjsbHh9pMqEfBghTrKCbj2hKvq8h4ZLgn8FaAGK6zUD8uTJaaaftH?cluster=devnet) slot 514214836 |

## Transactions

S steps set up state, T steps exercise the protocol, and N steps are attacks that must fail.

| Step | What it proves | Instruction | Expected | Observed | Slot | Block time (UTC) | Transaction |
|---|---|---|---|---|---|---|---|
| S01 | Fund the administrator and transport verifier, who pay rent for the records they create | `system::transfer` | success | success | 514214943 | 2026-10-23T10:23:35Z | [52VEnSaS…5JAFksxH](https://explorer.solana.com/tx/52VEnSaS8pJRqR8vLTZ5ynqPsFS8TkChbjM3zZsp3GkZ45QfMVHH1GrKjh4NjSWY2BQP3zArarqXqjW5JAFksxH?cluster=devnet) |
| S02 | Create the labelled 6-decimal devnet test mint (not Circle USDC) | `spl_token::initialize_mint2 + memo` | success | success | 514214951 | 2026-10-23T10:23:37Z | [4nHJn17T…tmxXJjHK](https://explorer.solana.com/tx/4nHJn17TSTompLC594aHqbn1ddmqL7pt9xxieadic1raRxjUDJbPUkppV5C3p9C41VEy5BhPv8Zqb3A3tmxXJjHK?cluster=devnet) |
| S03 | Create custody, escrow, adapter vault and transport token accounts; mint test tokens to the transport | `associated_token::create_idempotent + spl_token::mint_to_checked` | success | success | 514214960 | 2026-10-23T10:23:39Z | [4Xq641F5…ULt5anrF](https://explorer.solana.com/tx/4Xq641F5no3GGpuz5ZPCv9GMHwRG2SR3pg9UCkdqbnsL6v1fcvWcm6FxZrDPQfeyhpYrobGiEAdvcSFaULt5anrF?cluster=devnet) |
| N01 | Namespace claim co-signed by a key that is not the program's upgrade authority | `initialize_remote_leg` | rejected: Unauthorized (6000) | rejected: Unauthorized (6000) | 514214968 | 2026-10-23T10:23:41Z | [2eGaV8dV…LZNx4b8U](https://explorer.solana.com/tx/2eGaV8dVdMEEwwV8iBCRakxpsPJdvErUhRfP9MN21mxxTrejsgq1qJ6oG5ppLCUr46c37F4vH2Mu7jqqLZNx4b8U?cluster=devnet) |
| S04 | Claim the namespace: initialize_remote_leg co-signed by the upgrade authority | `initialize_remote_leg` | success | success | 514214976 | 2026-10-23T10:23:43Z | [59w7yZ3C…5ZVn9LSa](https://explorer.solana.com/tx/59w7yZ3CLk8TfMC6EGEsBFMAPjwLc8J6X91p9szaFkhgW8jazbtjr6SWqFtdzfKXkSDk9mpx9ZL79DAx5ZVn9LSa?cluster=devnet) |
| S05 | Create the risk config and the strict-next inbound control lane | `initialize_control_state` | success | success | 514214996 | 2026-10-23T10:23:48Z | [4wLHNUHP…SCMDPU2R](https://explorer.solana.com/tx/4wLHNUHPgpSVAdC6tBu3XSaMKg6qie92hXv59oCWEGC3ncZexme2G629F8M9PxVNA4CB121VgSUxN2sDSCMDPU2R?cluster=devnet) |
| S06 | Initialize the deterministic test strategy adapter bound to this namespace | `initialize_adapter` | success | success | 514215003 | 2026-10-23T10:23:49Z | [2qMmKXdT…BvJ4MzX2](https://explorer.solana.com/tx/2qMmKXdTm2mKgVwG8mE6cRKpBFiZpTjxjctYpDqJgoPtZHUT8UZB5QRTCzt4zeHF4jV6rtpP7vvutujeBvJ4MzX2?cluster=devnet) |
| S07 | Create the strategy config and the remote position | `initialize_strategy_state` | success | success | 514215010 | 2026-10-23T10:23:51Z | [4qPjTooD…BSnmQJbw](https://explorer.solana.com/tx/4qPjTooDBGWHgLtmvn5XZYthRQ74r5p1X67vbaiwwZbFjUFjZrCEHcFKfPxnKju9q1R79cHqgy6CYizdBSnmQJbw?cluster=devnet) |
| S08 | Create the outbound reporting lane | `initialize_outbound_state` | success | success | 514215018 | 2026-10-23T10:23:53Z | [2m1sXoA8…k4Kp2J5e](https://explorer.solana.com/tx/2m1sXoA8Yt7nJzrPTKR7GL6fjyP5pXyXkGf2V7hqxEvK4AXZdrjQticX68JduDU7r9X3LVvX4Mc3ath7k4Kp2J5e?cluster=devnet) |
| T01 | Authenticated Allocate A (sequence 1) delivered by the transport verifier | `process_allocate` | success | success | 514215026 | 2026-10-23T10:23:55Z | [4m9hRAYL…m13Pixrp](https://explorer.solana.com/tx/4m9hRAYLk7RB4NJit6VbV2zE7Zgr9f6WmzYE8Bn3tRUZ31cpAvzMZRhUTFyG2PRk6X8nFL5f7jfc7BtRm13Pixrp?cluster=devnet) |
| N02 | Replay of Allocate A's exact canonical bytes | `process_allocate` | rejected: InvalidSequence (6036) | rejected: InvalidSequence (6036) | 514215035 | 2026-10-23T10:23:57Z | [5gJpbLY5…5QLsU4WQ](https://explorer.solana.com/tx/5gJpbLY5uiu1UwiHSTgm2fDopcrQDqYQxGjHwprtzS55ktpZafkyKycgw1mdEAenPHGHrW7SUyGWUfV15QLsU4WQ?cluster=devnet) |
| N03 | Valid next Allocate signed by an attacker posing as the transport verifier | `process_allocate` | rejected: Unauthorized (6000) | rejected: Unauthorized (6000) | 514215046 | 2026-10-23T10:24:00Z | [2gqYVrX2…V6xBJuNE](https://explorer.solana.com/tx/2gqYVrX2h5vp5GYNZiZpNYeBQjBSsYjtpWBAwst8mydwcrdQUQRSrQ64rdvDLFcv2uQDG9zqpZSpr2ubV6xBJuNE?cluster=devnet) |
| T02 | Bridge stand-in delivers allocation A's test tokens to custody | `spl_token::transfer_checked` | success | success | 514215056 | 2026-10-23T10:24:02Z | [4jk4ZEyJ…9NESoF53](https://explorer.solana.com/tx/4jk4ZEyJwiRthsf1aW1W4eDKnDX5NGNbX1fTsJ8FuhJMFvJWr28wu7t9mqNPqeDsFuirBaXzNo68amMg9NESoF53?cluster=devnet) |
| T03 | Permissionless reconcile classifies the arrival as unattributed custody | `reconcile_custody` | success | success | 514215064 | 2026-10-23T10:24:04Z | [5MAxm8U4…WL9gfhpT](https://explorer.solana.com/tx/5MAxm8U4UyWEBK6Hh6onWBALJaEHmtCHY8DefedbH6CiCBqcTeaN9XUGSYCv1xmpHNp4mYAfUamG3R5hWL9gfhpT?cluster=devnet) |
| T04 | Attribute the arrival to allocation A, completing it | `attribute_allocation` | success | success | 514215072 | 2026-10-23T10:24:06Z | [3gR8vAAg…hnRorC47](https://explorer.solana.com/tx/3gR8vAAgoPiuxncByXg5hi37PDfWYGbtPDhQy4ZsVhjbsb4NzkWNq1QtnHjd5fdibdGSQ2a4vfrvXxhEhnRorC47?cluster=devnet) |
| T05 | Publish RemoteReport epoch 1 on the outbound lane | `publish_remote_report` | success | success | 514215079 | 2026-10-23T10:24:08Z | [55WAavp2…fvaivvbm](https://explorer.solana.com/tx/55WAavp2G9vTvfuwP8x2RPDFE9hGKSzzRyVDxb8z63Q6FqvTgB4ooG6LJy2b6uEUy8g884fntP3h6cyzfvaivvbm?cluster=devnet) |
| T06 | Authenticated Allocate B (sequence 2) | `process_allocate` | success | success | 514215088 | 2026-10-23T10:24:10Z | [5jV5WMQS…FuC59wcW](https://explorer.solana.com/tx/5jV5WMQS6mbxtMbZ1TBZJJ8Jeb8vbGE29GQwSqnwGNx2hNLfcYJbU3wKMConHg7Am4UUyXakevVMxXNTFuC59wcW?cluster=devnet) |
| T07 | Only part of allocation B arrives in custody | `spl_token::transfer_checked` | success | success | 514215096 | 2026-10-23T10:24:12Z | [WSuhtDK5…f42UYzct](https://explorer.solana.com/tx/WSuhtDK527YTtdo7gg9rrzSN2EWzu5HuQap37uzcgrR2BK76WU3iiLgpBKdtKDzb7stLzUWeBZhQYRyf42UYzct?cluster=devnet) |
| T08 | Attribute the partial arrival to allocation B | `attribute_allocation` | success | success | 514215117 | 2026-10-23T10:24:17Z | [5CFAjdvn…8qtzW9ie](https://explorer.solana.com/tx/5CFAjdvn5PQ23ttSpBebtfVcj84hUUiBYwH8dPfbi4jaUmXoTVys4q5rmzWRaSsont6ffnfkD5NuXCkz8qtzW9ie?cluster=devnet) |
| T09 | Resolve(Close) of allocation B after a partial arrival | `process_settlement` | success | success | 514215128 | 2026-10-23T10:24:19Z | [4mPisxQF…B1YUMcq7](https://explorer.solana.com/tx/4mPisxQFGQHNV14FDXf4drJRuYVurBvmZhykZ1FZzJ2Ppz8WpAjpQ6aoiQVPJSJ9B7s8r1BGgZcMVFxqB1YUMcq7?cluster=devnet) |
| T10 | The rest of B's assets arrive after the close | `spl_token::transfer_checked` | success | success | 514215136 | 2026-10-23T10:24:21Z | [2qik3Q6F…zSPMAfGf](https://explorer.solana.com/tx/2qik3Q6FHUKHgMwaiCbGfyq2Knxu3GjSLHJ8uuNkHWDkxAtTUHipz13CFjeQn4kw371QKT23CgrLAEqRzSPMAfGf?cluster=devnet) |
| T11 | Resolve(Recover) credits the late arrival against the impairment pool | `process_settlement` | success | success | 514215144 | 2026-10-23T10:24:23Z | [5FUavUZ3…wKM3n2Mx](https://explorer.solana.com/tx/5FUavUZ3VN8nMDLfkCwSHRtmNjqfsggdWJ9EMr2nHHxqQafHZ3KSByHfrav8ZoPk8rVwMafX2qH1H8sPwKM3n2Mx?cluster=devnet) |
| N04 | Replay of the Recover message cannot credit the pool twice | `process_settlement` | rejected: InvalidSequence (6036) | rejected: InvalidSequence (6036) | 514215155 | 2026-10-23T10:24:26Z | [3o8qmnFQ…jhrSg9tE](https://explorer.solana.com/tx/3o8qmnFQkr9N47hrse6p4eVBhcB3fre5GYmPrC8p5XXEL3D23bktDENsAhZ5Fz16k3rP7AqEsgAk6UsCjhrSg9tE?cluster=devnet) |
| T12 | Publish RemoteReport epoch 2 after the close and recovery | `publish_remote_report` | success | success | 514215167 | 2026-10-23T10:24:29Z | [3bbSXaZ9…3CGGLzgi](https://explorer.solana.com/tx/3bbSXaZ9kT7mU4qMNGS9U1HKfATetBG9UyAuBH3y9xJkmTpDDvTwfnBgFFUnTgyzX3ZZ8k8DVaXrpwHU3CGGLzgi?cluster=devnet) |
| T13 | Authenticated Allocate C (sequence 5) | `process_allocate` | success | success | 514215179 | 2026-10-23T10:24:32Z | [sYtzPaNM…21WzWaMZ](https://explorer.solana.com/tx/sYtzPaNM9qZpQUmcZ1KNzH8GqkJDX9UDNqnekNX7cZTuieFW4wEq4qL5bWhvzvLkhuWSbWAxb5W8xdz21WzWaMZ?cluster=devnet) |
| T14 | Emergency guardian freezes the leg | `freeze_remote_leg` | success | success | 514215187 | 2026-10-23T10:24:34Z | [3DKEykqC…v1y7n397](https://explorer.solana.com/tx/3DKEykqCkRdHqfhutz1hBKq9wbHuSXTgMrpYPJsB866K2N4vjW1qMJRHomX3TTgwp3FSgw1VZTo4gZqDv1y7n397?cluster=devnet) |
| N05 | Close of C on the frozen leg without the guardian | `process_settlement` | rejected: Frozen (6002) | rejected: Frozen (6002) | 514215204 | 2026-10-23T10:24:38Z | [2byt7oNy…UHxaPiyt](https://explorer.solana.com/tx/2byt7oNyStnrGADRgMGC2VCKjwGdyztau4B3g1kB3EeKbPsXQ4DfgVomwhtPzwbsCQpTg8aLWN1jBrGpUHxaPiyt?cluster=devnet) |
| N06 | Close of C on the frozen leg co-signed by an attacker posing as the guardian | `process_settlement` | rejected: Unauthorized (6000) | rejected: Unauthorized (6000) | 514215212 | 2026-10-23T10:24:40Z | [2j3JknyA…Q8t9zq4F](https://explorer.solana.com/tx/2j3JknyAWwXvwnbh95T9yXWdu9xbZYvxVr7WvvqGmRM23jUne7x2sgzDEyFPyYCqxJrTBpAc5Qfx8fRMQ8t9zq4F?cluster=devnet) |
| T15 | Guardian co-signed Close of C on the frozen leg (nothing arrived) | `process_settlement` | success | success | 514215224 | 2026-10-23T10:24:43Z | [61ihddDF…EB2Xwy25](https://explorer.solana.com/tx/61ihddDFPKbMDyrdY2RnP9ZWu2pEQTBUyZTRYJa7nAUnSKF2aNbXTWUo1mNoK1YtQwwyG968fLEE4L5hEB2Xwy25?cluster=devnet) |

## Source and build provenance

The run used commit [`7f38ba9`](https://github.com/ayushmishra2005/solana-evm-cross-chain-vault/commit/7f38ba90b38aa136a38f88612f677ffde4eee608) plus the hardening changes, which were not yet committed at the time. `scripts/devnet/run.sh` recorded the working tree, through a temporary index, as git tree `b3aaf0ee47347dc4467ce7187eeebdae051dc0bc`.

Those changes were merged into `main` as commit [`1aebf3f`](https://github.com/ayushmishra2005/solana-evm-cross-chain-vault/commit/1aebf3f7a4f7d2a84756a94f574c394075dba585), which has the same build inputs as the run: each hash below equals `git rev-parse 1aebf3f:<path>`. Since the run, only documentation and the devnet driver's reporting have changed.

| Build input | Git object at run time |
|---|---|
| `programs/solevm-remote-leg/src` | `78a57c8f3e83eda0edd1648ea333e3f0235a2960` |
| `programs/solevm-remote-leg/test-strategy` | `22751384d0a19e1fbc8d9af4fb3399b76a97222b` |
| `programs/solevm-remote-leg/Cargo.toml` | `72c1ae9aa14f4d889563dd27d375a56860ab9018` |
| `programs/solevm-remote-leg/Cargo.lock` | `dd8526dada468903db845c856a03b6b3a1d0d6fb` |
| `crates` | `487256f6227f883773a4326b88edc32d3316d15d` |
| `Cargo.toml` | `a76d3adbb7a31dbff3008d868159662e69d104ef` |

The build IDs are the SHA-256 digests of the deployed binaries, listed under [Programs](#programs). `scripts/devnet/build.sh` produced them with `cargo-build-sbf` 4.4.0, platform-tools v1.58, SBPFv0 and `--cfg solevm_devnet`, and a rebuild on the same machine reproduced both digests. A rebuild on another machine may differ, because host paths can end up in the binary. No containerized verifiable build was made.

## What this does not show

- **No bridge.** No real EVM→Solana bridge is involved. The disposable transport-verifier key stands in for the authenticated message receiver. The asset-transport key stands in for the bridge's token delivery by sending SPL transfers to custody. The source application is a fixed identifier, `keccak256("solevm/devnet/simulated-evm-hub")`. No EVM contract was deployed, and no message crossed chains.
- **A test token only.** The token is a fresh 6-decimal SPL token, labelled on chain by a memo as having no value and not being USDC. It is not Circle USDC.
- **Live versus local.** Only the two deployments and the 29 transactions above ran on devnet. The Rust, Solana program, Foundry and differential suites run locally and in CI, and the Solidity vault was tested there, not on devnet. A rehearsal on a local `solana-test-validator` preceded this run; it is not part of this evidence.
- **Disposable roles.** Every role is a disposable key held outside the repository. A single disposable key as upgrade authority is not a production setup.
- **No audit.** No external audit has been performed. The internal [security review](security-review.md) is not an audit, and neither passing tests nor this run substitute for one.
- **Devnet is a test cluster.** If devnet is reset, or RPC nodes stop serving this history, the links and `verify.sh` will stop resolving. The evidence file still records what was observed.

The [unresolved release risks](../README.md#unresolved-release-risks) still apply.
