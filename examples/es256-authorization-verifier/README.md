# ES256 authorization verifier

A standalone Solana program that consumes an ES256-signed authorization verdict, verified by Solana's native secp256r1 precompile, exactly once.

It is an independent demonstration of the verification technique. It is not an escrow or payment system, it moves no tokens, and it is not an official Saifuro integration. The verdict format follows Saifuro's published examples so the profile has a concrete, documented shape.

```
examples/es256-authorization-verifier
├── src/                         on-chain program (Anchor 1.2.1)
│   ├── precompile.rs            finds and pins the native verification
│   ├── jwt.rs                   strict compact-JWS / JSON profile parser
│   ├── instructions/            admin and consume instructions
│   └── state.rs                 accounts, seeds, bounds
├── adapter/                     host-side helpers (JWK compression, low-S, instruction builder)
├── test-programs/cpi-forwarder/ test-only program used to attack CPI and program identity
└── tests/                       LiteSVM suites against the compiled SBF binary
```

## Architecture

An administrator registers the keys it trusts and the payment contexts it approves. A consumer then submits each signed verdict in a two-instruction transaction:

```
[ compute budget ]*   optional, nothing else may precede the precompile
[ secp256r1 precompile ]   native: verifies sig over the JWS signing input
[ consume_authorization ]  program: reads the precompile's bytes, parses, checks, records nonce
[ other instructions ]*    allowed, unless they target this program or the precompile
```

| Instruction | Signer | Effect |
|---|---|---|
| `initialize_config` | program upgrade authority | Creates the config for one environment (`Production` or `Sandbox`) and names the administrator and consumer. |
| `register_trusted_key` | administrator | Records a kid, its compressed P-256 key and its validity window. |
| `revoke_trusted_key` | administrator | Permanently revokes a key. The account remains as a tombstone, so the kid cannot be registered again. |
| `register_mandate` | administrator | Records an approved context: mandate id, amount, currency, recipient, policy version, agent and category. |
| `revoke_mandate` | administrator | Permanently revokes a mandate. |
| `consume_authorization` | consumer | Verifies one verdict against a trusted key and an approved mandate and creates its permanent nonce receipt. |

A receipt is the PDA `["consumed", config, sha256("es256-authorization-verifier:nonce:v1" ‖ nonce)]`, created with Anchor `init`. If it already exists, the consume fails, so each nonce succeeds at most once per config. The program has no instruction that closes or resets a receipt.

## Cryptographic verification

- **Signing input.** ES256 signs `ASCII(BASE64URL(header) "." BASE64URL(payload))`. The precompile hashes exactly those bytes with SHA-256. The signature is raw 64-byte `r ‖ s`, not DER.
- **Public key.** JWKS publishes `x` and `y`. The adapter checks `kty = EC`, `crv = P-256`, 32-byte coordinates and that the point is on the curve. It then produces the 33-byte compressed SEC1 key that the administrator registers. The program checks only the `0x02`/`0x03` prefix; it does no curve arithmetic. A registered key that is not a valid point fails native verification on every use.
- **Low S.** The precompile rejects `s > n/2`. OpenSSL and many JOSE libraries emit high-S about half the time, so the adapter rewrites `s` to `n - s`. That form verifies the same message under the same key. Normalization does not affect replay protection, because receipts are keyed by the signed nonce, not by the signature.
- **Why the program reads the verified bytes itself.** The native program accepts many layouts: up to eight signatures, offsets into other instructions, overlapping slices, an ignored padding byte, account metas and trailing bytes. The program accepts exactly one layout:
  - one signature and a zero padding byte;
  - all three instruction-index fields set to `u16::MAX`, so the key, signature and message are all read from the precompile instruction's own data;
  - fixed byte offsets within that data: the 33-byte key at 16, the 64-byte signature at 49 and the message at 113;
  - message length 1 to 512, running to the end of the data;
  - no accounts.

  The precompile must be the instruction immediately before `consume_authorization`, and the call must be top level (not via CPI). Under these conditions, the claims the program parses are the bytes the native program verified.

## Security guarantees

Enforced on-chain:

- No authorization succeeds unless the native precompile verified a signature over the exact signing input the program parses.
- The verifying key must equal the bytes registered for the token's `kid`, in this config's environment. The key must not be revoked, the cluster clock must have reached the key's `not_before`, and the token's `iat` and `exp` must lie inside the key's window.
- The signed mandate id, amount, currency, recipient, policy version, agent and category must equal the referenced mandate account, which belongs to the same config. The only instruction argument, `nonce_hash`, must equal the hash of the signed nonce. No argument can supply payment terms.
- A nonce is consumed at most once per config, and failed transactions leave no receipt.
- Only the upgrade authority creates configs, and only the config's administrator registers or revokes keys and mandates.

Off-chain responsibilities:

- Fetching and authenticating the issuer's JWKS, converting keys with the adapter, and registering or revoking them on time.
- Deciding which mandates to approve.
- Acting on a consumed authorization. A receipt proves that a verdict was valid and consumed once. It moves no funds.

## JWT profile

The parser accepts one profile and rejects everything else. **It deliberately rejects some tokens that are valid JWTs.**

- **Header:** `alg` must be `ES256` and `kid` is required; `typ`, if present, must be `JWT`. `jwk`, `jku`, `x5c`, `x5u`, `x5t`, `x5t#S256`, `crit` and `b64` are rejected (`UnsupportedClaim`), because keys come only from program state. Any other header is `UnknownField`.
- **Claims:** `iss`, `jti`, `iat`, `exp`, `verdict`, `mandate`, `policy_version`, `nonce`, and `request` with `agent`, `amount`, `currency`, `recipient` and `category`. All are required. `aud`, `nbf` and `supersedes` are rejected, and any other field is `UnknownField`.
- **JSON subset:** objects, strings and unsigned numbers only, with `request` as the only nested object.
  - Strings are printable ASCII. Any backslash escape (including `\uXXXX`), control character or non-ASCII byte is rejected, so every name and value has one spelling.
  - A repeated field is rejected at the header, claims and `request` levels, whichever copy matches.
  - Only space, tab, CR and LF count as whitespace. Trailing content is rejected.
- **Base64URL:** unpadded and canonical, with zero trailing bits. Exactly two non-empty segments.
- **Numbers:** `iat` and `exp` are positive integers with no sign, fraction, exponent or leading zero, at most 253402300799. `amount` is a non-negative decimal with up to 13 integer digits and up to 6 fractional digits, scaled exactly to micro-units with checked integer arithmetic, so `340` equals `340.000000`. Exponents, signs, leading zeros and a seventh fractional digit are rejected. No rounding is applied and no floats are used.
- **Bounds:**

  | Field | Maximum bytes |
  |---|---|
  | `kid` | 32 |
  | `jti`, `mandate`, `policy_version`, `nonce`, `agent` | 64 |
  | `recipient` | 128 |
  | `category` | 32 |
  | `currency` | 16 |

  The whole signing input is at most 512 bytes, enforced separately from the per-field limits. A single field can reach its maximum when the others are short; a 128-byte recipient, for example, fits in a 509-byte signing input. Not every field can be at its maximum at once.
- **Context values are compared byte for byte.** `iss` must be `https://verdicts.saifuro.com` and `verdict` must be `allow`. Currency, recipient and the other context fields are case-sensitive.

Time and window boundaries:

| Check | Accepted when |
|---|---|
| Lifetime | `exp == iat + 300` exactly |
| Future issue | `iat <= now + 60` |
| Expiry | `now < exp` (rejected at `now == exp`) |
| Key window | `not_before <= now`, `not_before <= iat` and `exp <= not_after`, all inclusive. The `iat` skew never lets a key authorize before `not_before`. Since `now < exp`, the last second a key authorizes is `not_after - 1`. |
| Environment | Production refuses kids starting with `sandbox-`; Sandbox requires that prefix. |

## Trust boundaries

- **The administrator is the on-chain trust root for a config.** Whoever controls it can trust any key and approve any context. Nothing here protects against a compromised administrator. The administrator and consumer are fixed when the config is created and cannot be rotated. There is exactly one config per environment, `Production` and `Sandbox`, so replacing the roles of an initialized environment requires a separate deployment under a different program ID.
- **The upgrade authority is a separate trust root.** It alone can create configs, and while the program is upgradeable it can replace the program. Permanent receipts and terminal revocation are properties of this implementation; an authorized upgrade could change them. Making the program immutable after its configs exist removes this trust root, but no further config can then be created.
- **No HTTP on-chain.** JWKS retrieval, TLS and key validation happen off-chain, and key rotation takes administrator transactions. A key revoked by the issuer stays trusted until the administrator revokes it. Re-registering a revoked key's bytes under a new kid is an administrator decision that the program does not prevent.
- **The consumer** must sign every consume. A compromised consumer key cannot forge verdicts, but it can consume, and so burn, valid verdicts before their intended use.
- **A mandate is a standing approval.** Every distinct issuer-signed nonce that matches it can be consumed, and the program sets no usage or spending cap. `jti` is recorded in the receipt as a hash but is not tracked for uniqueness; the nonce is the replay identity.
- **A signed verdict is not settlement.** Whatever acts on a receipt must enforce what actually happens to funds.
- **No signed domain separation.** The verdict does not name this program, config or cluster. Replay protection is local to one config of one deployment. The same token could be consumed once on each cluster or deployment that trusts the key.
- **Environment isolation.** Production and Sandbox have separate config PDAs, and every trusted key, mandate and receipt belongs to exactly one config. On top of that, a Production config refuses `sandbox-` kids and a Sandbox config requires them. The prefix is a naming rule, not proof of where a key came from: the administrator must register only keys taken from the issuer's authenticated JWKS for that environment.
- **Receipts are permanent.** Each receipt (185 bytes) locks rent-exempt lamports forever. That is 1,590,040 lamports under LiteSVM's default rent parameters; the actual cost depends on the cluster's rent settings.

## Threat model

| Threat | Defence | Tests |
|---|---|---|
| Forged or malformed signatures | Native precompile rejects them before the program runs | `cryptography.rs` |
| Signature or claim substitution | The program parses only the bytes the precompile verified, in the one pinned layout | `cryptography.rs`, `introspection.rs` |
| Lookalike or misplaced verification | Program-id, position, top-level and transaction-shape checks | `introspection.rs` |
| Ambiguous JSON, duplicate claims | Strict single-spelling profile; duplicates rejected at every level | `jwt_profile.rs`, `jwt.rs` unit tests |
| Untrusted, revoked or wrong-environment keys | Registered key bytes, revocation tombstones, windows, kid prefix | `cryptography.rs`, `time_and_keys.rs` |
| Unapproved terms | Field-by-field match against the administrator's mandate | `authorization.rs` |
| Replay | Receipt PDA `init`, with atomic rollback on failure | `replay.rs` |
| Account substitution, forged admin state | Anchor owner, discriminator, seeds and `has_one` checks | `authorization.rs`, `admin.rs` |
| Malformed input crashing the program | Bounded, checked parsing; mutation sweeps on SBF | `robustness.rs`, unit tests |

## Testing

Prerequisites:
- `cargo-build-sbf` 4.4.0 (`cargo install cargo-build-sbf --version 4.4.0 --locked`), run with `--tools-version v1.58`; it fetches platform-tools v1.58 on first use. An older Solana CLI install may put a different `cargo-build-sbf` first on `PATH`, so check that `cargo-build-sbf --version` reports 4.4.0.
- Rust 1.89 or newer (the crate's `rust-version`); validated with Rust 1.98.1.
- Dependencies are pinned by this example's own `Cargo.lock`.

Build both programs, then run the suites against the built binary.
- One `cargo test` run covers one architecture. `SBF_OUT_DIR` chooses the build directory: it defaults to `target/deploy`, and when set it is the only place the harness looks, with no fallback.
- `ES256_EXPECTED_SBPF_ARCH` (`v0` or `v3`) makes the harness assert the loaded ELF is that architecture. Without it, the harness only checks that the binary is v0 or v3.

```bash
cd examples/es256-authorization-verifier
cargo-build-sbf --tools-version v1.58 --arch v0
cargo-build-sbf --tools-version v1.58 --arch v0 \
  --manifest-path test-programs/cpi-forwarder/Cargo.toml --sbf-out-dir target/deploy

cargo fmt --all --check
cargo clippy --locked --workspace --all-targets --all-features -- -D warnings
SBF_OUT_DIR=$PWD/target/deploy ES256_EXPECTED_SBPF_ARCH=v0 \
  cargo test --locked --workspace --all-features

cargo-build-sbf --tools-version v1.58 --arch v3 --sbf-out-dir target/deploy-v3
cargo-build-sbf --tools-version v1.58 --arch v3 \
  --manifest-path test-programs/cpi-forwarder/Cargo.toml --sbf-out-dir target/deploy-v3
SBF_OUT_DIR=$PWD/target/deploy-v3 ES256_EXPECTED_SBPF_ARCH=v3 \
  cargo test --locked --workspace --all-features

# measurements, one architecture per run
SBF_OUT_DIR=$PWD/target/deploy ES256_EXPECTED_SBPF_ARCH=v0 \
  cargo test --locked --test limits -- --nocapture --test-threads=1
SBF_OUT_DIR=$PWD/target/deploy-v3 ES256_EXPECTED_SBPF_ARCH=v3 \
  cargo test --locked --test limits -- --nocapture --test-threads=1
```

The suite has 125 tests: 20 program unit tests, 8 adapter unit tests and 97 LiteSVM integration tests.
- **Acceptance and signature scenarios** execute the compiled SBF program with the native secp256r1 precompile enabled (LiteSVM `precompiles` feature), with no mock.
- **Other integration tests** exercise administration and account validation, or inspect the loaded ELF.
- **Mutation sweeps** (`robustness.rs`) run 4,622 re-signed mutations, each on a fresh copy of the configured ledger. Each outcome is checked against the program's own parser run on the host: a rejected input must fail on-chain with exactly that parser error, and a parsed one must be consumed or fail a post-parse authorization check.

Acceptance tests sign with P-256 keys generated in memory on each run. They do not demonstrate acceptance of a live Saifuro-issued verdict. The adapter's conversion of Saifuro's published JWK is unit-tested.

Measured on LiteSVM 0.18.1 (SBPFv0 / SBPFv3):

| Transaction | Signing input | Size (limit 1,232) | Compute units |
|---|---|---|---|
| Typical verdict | 476 B | 1,033 B | 37,327 / 37,297 |
| Largest accepted verdict, plus two compute-budget instructions | 512 B | 1,121 B | 39,881 / 39,851 |
| Over the limit, rejected by the verifier | 513 B | 1,070 B | 15,196 / 15,162 |
| Late rejection (category mismatch) | 479 B | 1,036 B | 38,188 / 38,154 |
| Native rejection (high S) | 476 B | 1,033 B | 0 (the program never runs) |

Compute units are LiteSVM's `compute_units_consumed` for the transaction. That figure is the verifier's own execution, including the system-program call that creates the receipt; the native precompile reports 0 compute units, whether it accepts or rejects. The precompile is paid for through signature fees instead. A consume transaction pays two, one for the payer and one for the precompile signature: 10,000 lamports at LiteSVM's default.

The program runs on the default 32 KiB heap, using no custom allocator. It allocates the decoded header and payload, which together are smaller than the 512-byte input, and the instructions it loads from the sysvar, which are bounded by the transaction size.

Binary sizes are 329,952 bytes for SBPFv0 and 309,576 bytes for SBPFv3. The test-only forwarder is 15,296 and 13,632 bytes.

LiteSVM executes transactions one at a time. The competing-submission test shows that the second of two concurrently prepared transactions fails on the existing receipt. On a validator, both transactions write-lock the same receipt address, so the runtime serializes them; that scheduling path is not exercised here.

[`devnet/`](devnet/) records one accepted verdict and one rejected replay on Solana devnet, with the runner that produced them.

## Limitations

- Not a general JWT library. Tokens outside the profile, including valid ones with escapes, extra claims or other algorithms, are rejected.
- No automatic JWKS synchronization, and no protection against a compromised administrator.
- No replay protection across clusters, deployments or configs.
- Not production payment authorization and not escrow settlement. Nothing moves funds.
- Only legacy transactions are tested. Versioned transactions and address lookup tables are not covered, and the example makes no claim about them.

## References

- Solana [precompiles](https://solana.com/docs/core/programs/precompiles) (secp256r1 instruction layout) and [instruction introspection](https://solana.com/docs/core/instructions/instruction-introspection)
- Saifuro [verdict verification](https://docs.saifuro.com/reference/verify-a-verdict) and [signing keys and rotation](https://docs.saifuro.com/reference/signing-keys)
- [RFC 7515](https://www.rfc-editor.org/rfc/rfc7515) (JWS) and [RFC 7518 §3.4](https://www.rfc-editor.org/rfc/rfc7518#section-3.4) (ES256)
