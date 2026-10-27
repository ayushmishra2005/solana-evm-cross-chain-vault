# Devnet evidence: ES256 authorization verifier

On 2026-10-27 the verifier was deployed to Solana devnet as SBPFv0. One synthetic allow verdict was consumed and its receipt matched the verdict field for field. The same verdict was then submitted again under a new blockhash and was rejected on chain, leaving every program account unchanged. Every signature below was read back at `finalized` commitment.

The verdict is locally generated test data. It was signed by a P-256 key generated in memory for this run and never written to disk. It is not a Saifuro-issued token and proves nothing about Saifuro's keys.

## Network

| | |
|---|---|
| Cluster | devnet, `https://api.devnet.solana.com` |
| Genesis hash | `EtWTRABZaYq6iMfeYKouRu166VU2xqa1wcaWoxPkrZBG` |
| Validator version | `solana-core` 4.4.0-beta.1, feature set 3993424257 |
| secp256r1 program | `Secp256r1SigVerify1111111111111111111111111`, executable |

## Source and build

The program was built from commit `05c4e8e23d9857b2a6632a3899deee15672952e1` with one change: `declare_id!` set to `da9yz64zbrJ4fyi8VTR3xfVZu8GhfhUHkC7AZQxCGG7`. That change is committed with this record. The previously declared ID, `6b14551jSHhBRGhdFuqEqzEa5EDsujpC3Ku4jQtDQGao`, was never deployed, and its keypair is no longer available.

| | |
|---|---|
| Builder | `cargo-build-sbf` 4.4.0, `--tools-version v1.58 --arch v0` (platform-tools rustc 1.95.0-dev) |
| ELF | machine `0x107` (SBF), `e_flags` 0 (SBPFv0) |
| Size | 329,952 bytes |
| SHA-256 | `efe5d65e04d6c593a0eeedae9fc117407468ed0d0c1a89d3c156b154dccdb3a4` |
| On-chain bytes | `solana program dump` output is byte-identical to the local build |

The 125-test suite passed against this build, and against the SBPFv3 build of the same source.

## Deployment

| | |
|---|---|
| Program ID | [`da9yz64zbrJ4fyi8VTR3xfVZu8GhfhUHkC7AZQxCGG7`](https://explorer.solana.com/address/da9yz64zbrJ4fyi8VTR3xfVZu8GhfhUHkC7AZQxCGG7?cluster=devnet) |
| ProgramData | `8sWQvyuukDkq6fi7TXDd3m99sypRLF71TPFNkkt79wt2` (329,997 bytes, 1.677035 SOL) |
| Upgrade authority | `9WUUr2WNUiKMzwxJgbb4oxS81oYAyhrBFkv3NSg2mjbj`, a disposable devnet wallet |
| Deploy signature | [`3AqRXiXM2kodhojwqKDmYXwR6SYmxxd137Buit5ENviwnWuu2HfD28Z65bSqHH4LSoYF53hS6f8H223wu3ykaBUr`](https://explorer.solana.com/tx/3AqRXiXM2kodhojwqKDmYXwR6SYmxxd137Buit5ENviwnWuu2HfD28Z65bSqHH4LSoYF53hS6f8H223wu3ykaBUr?cluster=devnet) |
| Finalized slot | 515,606,874 (2026-10-27T06:46:04Z) |

The first deploy attempt sent buffer writes through the public RPC endpoint, which throttled them until the CLI gave up. The second attempt resumed into the same buffer, `MMbAFzPF2JSHgXKJMbQbHUE584cMcTruowNtvm7yogr`, by sending to validators directly. Deployment consumed that buffer. No program was upgraded.

## Configured state

| Account | Address | Contents |
|---|---|---|
| Config (Sandbox) | `F3SUhPwNis6poeP5v17b6LhbxqWfgQjtchDTdt7ic9k6` | administrator `9WUUr2WNUiKMzwxJgbb4oxS81oYAyhrBFkv3NSg2mjbj`, consumer `2zn2Cq5r2bcwgUHW8iaXArpQbSghzWJiRGKLwrQnryDj` |
| Trusted key | `7RhQSeJifiSen3vhEqqgMZoUn4cGKPZFG4xnajwHTuqB` | kid `sandbox-devnet-01`, key `038293b625723b9ffb5ad3508b0873f8dcd8818d4acb56f70e8ec791c9579ea8d5`, valid 2026-10-27T06:25:28Z to 2026-11-26T07:25:28Z |
| Mandate | `Byv1Jn8irus84ydLe58JM5eXEwt6cqUqayT5EqEABGtY` | hash `7a9fea5e9ba846a624108f3089dd745087e77845bd13bda74261d8a5f3d66801` |

The mandate approves `mnd_devnet_01`: 340 USD (`amount_micros` 340,000,000) to `vendor:acme_saas`, under policy `pol_2026-08-12.1`, for agent `agt_procurement_01`, category `saas`.

## Verdict

Header: `{"alg":"ES256","typ":"JWT","kid":"sandbox-devnet-01"}`

Payload:

```json
{"iss":"https://verdicts.saifuro.com","jti":"dec_devnet_8293b625","iat":1793085945,"exp":1793086245,"verdict":"allow","mandate":"mnd_devnet_01","policy_version":"pol_2026-08-12.1","nonce":"n_devnet_8293b625","request":{"agent":"agt_procurement_01","amount":340,"currency":"USD","recipient":"vendor:acme_saas","category":"saas"}}
```

`iat` was taken from devnet's clock. The signing input is 510 bytes, and the low-S signature is `cf71a71358ddc3ae39dc78d0df02a56f0557f5a3c65b250847ee440d59ddb6b33a46bce9eb220eb7848cae9e32c6a865aef06717fcffa08bbe68f1ed8e1747e9`.

## Transactions

All are legacy transactions, finalized.

| Step | Signature | Slot | Fee (lamports) | Compute units | Result |
|---|---|---|---|---|---|
| Fund consumer | [`5pdhNRyf…XsCqgKPL`](https://explorer.solana.com/tx/5pdhNRyfXbivsrxMVSS3Ew59We2PYFZKdgiJ5wzX6mQSBoT1y32ooQcfmd1p28sE84QwhGSvXfME9GAyXsCqgKPL?cluster=devnet) | 515,616,782 | 5,000 | 150 | ok |
| Initialize config | [`5bmVkVo7…xkSaJKXT`](https://explorer.solana.com/tx/5bmVkVo71qhhQ9CZb8GEkED4H4ZAQd8Z437RLt5uNQjAdc24ZRKmiRs2DjkzjpKYaWCdjVrSMATVp7XzxkSaJKXT?cluster=devnet) | 515,616,799 | 5,000 | 15,450 | ok |
| Register key | [`9JfPLQMJ…zUoY1bFX`](https://explorer.solana.com/tx/9JfPLQMJowMWJW5isBMtJZd8EvDRunAK7MVh1ainuHGkpCNFr9hP9pzbMcZfGrFDH7vuaafDN7zyqGGzUoY1bFX?cluster=devnet) | 515,616,816 | 5,000 | 21,502 | ok |
| Register mandate | [`2g45654X…5kBRGNQC`](https://explorer.solana.com/tx/2g45654XMwnKhoiLWwyjJBDUp2NTqar5t9QCZKACDpiPcV5Km69JCCGxotevQppaRHuBYdUmfbNfNZtX5kBRGNQC?cluster=devnet) | 515,616,832 | 5,000 | 12,143 | ok |
| Consume | [`3fA2qvGC…SJJJojBy`](https://explorer.solana.com/tx/3fA2qvGCtKD5i2Jrj4wdfgXSAA1KXVNv3ig8ovagvEj5mgf56VNvvLSHHB3FLbPTYLitygC3gPARJKjrSJJJojBy?cluster=devnet) | 515,616,851 | 10,000 | 41,697 | ok |
| Replay | [`4UQcqvKv…5mEhjSwU`](https://explorer.solana.com/tx/4UQcqvKvfJs1qvPUaVHFErt3ZnCYLtQoUmjQZKJTYrGdkoATCDAjcbJgqq75wNUkt52FL7P8RNJBFAi75mEhjSwU?cluster=devnet) | 515,616,875 | 10,000 | 11,300 | `InstructionError(1, Custom(0))` |

Full signatures of the two consume transactions:

- Consume: `3fA2qvGCtKD5i2Jrj4wdfgXSAA1KXVNv3ig8ovagvEj5mgf56VNvvLSHHB3FLbPTYLitygC3gPARJKjrSJJJojBy`
- Replay: `4UQcqvKvfJs1qvPUaVHFErt3ZnCYLtQoUmjQZKJTYrGdkoATCDAjcbJgqq75wNUkt52FL7P8RNJBFAi75mEhjSwU`

Both consume transactions are 1,067 bytes: the secp256r1 instruction followed by `consume_authorization`, signed by the consumer alone. Each pays two signature fees, one for the consumer and one for the precompile signature. The precompile does not appear in the compute log. The 41,697 units are the verifier's own execution, including the system-program call that creates the receipt.

## Receipt

Address [`E2DFaam4iJM7m8nPeADLetVLmwVuc4YwF1G8CdvD9iV8`](https://explorer.solana.com/address/E2DFaam4iJM7m8nPeADLetVLmwVuc4YwF1G8CdvD9iV8?cluster=devnet): owned by the program, 185 bytes, 1,590,040 lamports.

| Field | Value | Checked against |
|---|---|---|
| `config` | `F3SUhPwNis6poeP5v17b6LhbxqWfgQjtchDTdt7ic9k6` | the Sandbox config |
| `nonce_hash` | `099bc0ee9c011114ae174eb2e520383418754677179ca499623af3733a206099` | `sha256("es256-authorization-verifier:nonce:v1" ‖ "n_devnet_8293b625")` |
| `signing_input_hash` | `c41cf92840d8c06f952e264d019c2b7daed5c453663c62c174b62e2fbff0d04b` | SHA-256 of the 510-byte signing input |
| `jti_hash` | `de43189c1bebec00fd64ff240e0d9c30b9e4899d361d2ab16469549189b752a9` | `sha256("es256-authorization-verifier:jti:v1" ‖ "dec_devnet_8293b625")` |
| `mandate` | `Byv1Jn8irus84ydLe58JM5eXEwt6cqUqayT5EqEABGtY` | the mandate account |
| `consumed_at` | 1793085946 (2026-10-27T07:25:46Z) | between `iat` − 60 and `exp` |
| `expires_at` | 1793086245 (2026-10-27T07:30:45Z) | `exp` |
| `bump` | 253 | the canonical bump of the receipt address |

## Replay

The replay carried the identical instructions and signed verdict under a different recent blockhash: `Ehb54w36cGzE8CJAJWb8x7Ew1ZxUtHMUrYbozDjGRHKY`, where the accepted transaction used `AvFjVu9MPT5u9q6F6t84PXKLa8XLHW4nscbr23He7WME`. That made it a new transaction rather than a duplicate. Simulation returned `InstructionError(1, Custom(0))`. The transaction was then sent with `skipPreflight`, the only transaction in this run sent that way, so the rejection would be recorded on chain. It landed and was finalized with the same error and these logs:

```text
Program da9yz64zbrJ4fyi8VTR3xfVZu8GhfhUHkC7AZQxCGG7 invoke [1]
Program log: Instruction: ConsumeAuthorization
Program 11111111111111111111111111111111 invoke [2]
Allocate: account Address { address: E2DFaam4iJM7m8nPeADLetVLmwVuc4YwF1G8CdvD9iV8, base: None } already in use
Program 11111111111111111111111111111111 failed: custom program error: 0x0
Program da9yz64zbrJ4fyi8VTR3xfVZu8GhfhUHkC7AZQxCGG7 consumed 11300 of 400000 compute units
Program da9yz64zbrJ4fyi8VTR3xfVZu8GhfhUHkC7AZQxCGG7 failed: custom program error: 0x0
```

Custom error 0 from the system program is `AccountAlreadyInUse`: the receipt's `init` found the nonce already consumed. The verdict was still within its lifetime, so no time check could account for the rejection.

## Atomicity

The config, trusted key, mandate and receipt accounts were read at finalized commitment before and after the replay. All four were identical in lamports, owner and data.

| | Before replay | After replay |
|---|---|---|
| Receipt data SHA-256 | `d3a84317070a1e7526aab763e4abcc63d6bc7affe4a026cb91bad8892c2ae8bd` | `d3a84317070a1e7526aab763e4abcc63d6bc7affe4a026cb91bad8892c2ae8bd` |
| Consumer lamports | 18,399,960 | 18,389,960 |

The consumer's balance fell by exactly the replay's 10,000-lamport fee.

## Funding and cost

No airdrop was requested. The deployer started with 9,151,285,101 devnet lamports and ended with 7,446,357,261.

| Item | Lamports |
|---|---|
| ProgramData rent | 1,677,035,000 |
| Program account rent | 833,120 |
| Deploy transaction fees, both attempts | 1,660,000 |
| Consumer funding | 20,000,000 |
| Config, key and mandate rent | 5,379,720 |
| Configuration transaction fees | 20,000 |
| Total from the deployer | 1,704,927,840 |

The consumer spent 1,590,040 on receipt rent and 20,000 on the two consume fees, and holds 18,389,960.

## CI

Before this change, the last CI run on `05c4e8e` was [run 40460437128](https://github.com/ayushmishra2005/solana-evm-cross-chain-vault/actions/runs/40460437128): `sbpf-v0` and `sbpf-v3` both succeeded. CI has not yet run on the `declare_id!` change committed with this record.

## Limitations

- This is a single run on devnet. Nothing here was run on mainnet or testnet, and devnet runtime versions change.
- The verdict and its key are locally generated. The run shows that the deployed program accepts a correctly signed verdict and rejects its replay. It does not show acceptance of a Saifuro-issued verdict.
- The deployer is also the administrator. The consumer keypair is disposable and kept outside the repository, so this config's consumer role should be treated as expendable.
- Only legacy transactions were sent, and the two consume transactions were not submitted concurrently.
- Compute units here differ from the LiteSVM measurements in the example README (41,697 against 37,327 for SBPFv0). The verdicts differ (510 against 476 bytes of signing input, with longer kid, jti and nonce), and so do the runtimes (LiteSVM 0.18.1 against devnet 4.4.0-beta.1).
