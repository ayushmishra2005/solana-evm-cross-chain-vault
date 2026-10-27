//! Transaction-size and compute measurements at the boundaries of the
//! profile, and a check that the harness loaded the intended binary.
//!
//! Run with `--nocapture` to see the measurements.

#![allow(
    clippy::unwrap_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)]

mod common;

use anchor_lang::solana_program::instruction::Instruction;
use common::*;
use es256_authorization_verifier::VerifierError;
use es256_authorization_verifier::precompile::MAX_SIGNING_INPUT_LEN;
use solana_compute_budget_interface::ComputeBudgetInstruction;
use solana_instruction::error::InstructionError;
use solana_precompile_error::PrecompileError;

/// Legacy transaction packet limit.
const PACKET_DATA_SIZE: usize = 1_232;
/// Default per-instruction compute budget.
const DEFAULT_INSTRUCTION_COMPUTE_UNITS: u64 = 200_000;

/// Claims whose signing input is exactly `target` bytes, found by varying
/// the jti length and whether `typ` is present.
fn claims_of_length(target: usize) -> (String, VerdictClaims) {
    for with_typ in [true, false] {
        for jti_len in 1..=64 {
            let claims = VerdictClaims {
                jti: "j".repeat(jti_len),
                nonce: format!("n_len_{target}"),
                ..VerdictClaims::default()
            };
            let header = if with_typ {
                claims.header_json()
            } else {
                format!(r#"{{"alg":"ES256","kid":"{KID}"}}"#)
            };
            let input = es256_jws_adapter::signing_input(
                header.as_bytes(),
                claims.payload_json().as_bytes(),
            );
            if input.len() == target {
                return (header, claims);
            }
        }
    }
    panic!("no claims of length {target}");
}

struct Measurement {
    signing_input: usize,
    transaction: usize,
    compute_units: u64,
}

fn measure(
    fixture: &mut Fixture,
    instructions: &[Instruction],
    expect_success: bool,
) -> Measurement {
    let consumer = fixture.consumer.insecure_clone();
    let transaction = fixture.transaction(instructions, &[&consumer]);
    let size = transaction_size(&transaction);
    let signing_input = instructions
        .iter()
        .find(|instruction| instruction.program_id == solana_sdk_ids::secp256r1_program::ID)
        .map_or(0, |instruction| instruction.data.len() - 113);
    let compute_units = match fixture.svm.send_transaction(transaction) {
        Ok(meta) => {
            assert!(expect_success, "transaction unexpectedly succeeded");
            meta.compute_units_consumed
        }
        Err(failure) => {
            assert!(!expect_success, "transaction failed: {:?}", failure.err);
            failure.meta.compute_units_consumed
        }
    };
    Measurement {
        signing_input,
        transaction: size,
        compute_units,
    }
}

fn report(name: &str, measurement: &Measurement) {
    println!(
        "{name:<48} signing input {:>4} B   transaction {:>5} B   {:>6} CU",
        measurement.signing_input, measurement.transaction, measurement.compute_units
    );
}

#[test]
fn accepted_verdicts_fit_the_packet_and_compute_limits() {
    let mut fixture = Fixture::new();
    let budget = || {
        [
            ComputeBudgetInstruction::set_compute_unit_limit(60_000),
            ComputeBudgetInstruction::set_compute_unit_price(1),
        ]
    };

    let typical = fixture.sign(&VerdictClaims::default());
    let typical_ix = [
        typical.precompile(),
        fixture.consume_instruction(typical.nonce_hash()),
    ];
    let typical_m = measure(&mut fixture, &typical_ix, true);
    report("typical verdict", &typical_m);
    let receipt = fixture
        .svm
        .get_account(&fixture.receipt_address(&typical.nonce_hash()))
        .unwrap();
    assert_eq!(
        receipt.lamports,
        fixture
            .svm
            .minimum_balance_for_rent_exemption(receipt.data.len())
    );
    println!(
        "receipt: {} bytes, {} lamports of permanent rent",
        receipt.data.len(),
        receipt.lamports
    );

    let (header, claims) = claims_of_length(MAX_SIGNING_INPUT_LEN);
    let input =
        es256_jws_adapter::signing_input(header.as_bytes(), claims.payload_json().as_bytes());
    let largest = SignedVerdict::sign_input(&fixture.signer, input, &claims.nonce);
    assert_eq!(largest.signing_input.len(), MAX_SIGNING_INPUT_LEN);
    let mut largest_ix = budget().to_vec();
    largest_ix.extend([
        largest.precompile(),
        fixture.consume_instruction(largest.nonce_hash()),
    ]);
    let largest_m = measure(&mut fixture, &largest_ix, true);
    report("512-byte verdict with compute-budget prefix", &largest_m);

    for measurement in [&typical_m, &largest_m] {
        assert!(measurement.transaction <= PACKET_DATA_SIZE);
        assert!(measurement.compute_units <= DEFAULT_INSTRUCTION_COMPUTE_UNITS);
    }
    println!(
        "headroom at the largest accepted verdict: {} bytes",
        PACKET_DATA_SIZE - largest_m.transaction
    );
}

#[test]
fn rejected_verdicts_are_bounded_too() {
    let mut fixture = Fixture::new();

    let (header, claims) = claims_of_length(MAX_SIGNING_INPUT_LEN);
    let mut payload = claims.payload_json();
    payload.push(' ');
    let oversized = SignedVerdict::sign_input(
        &fixture.signer,
        es256_jws_adapter::signing_input(header.as_bytes(), payload.as_bytes()),
        &claims.nonce,
    );
    assert!(oversized.signing_input.len() > MAX_SIGNING_INPUT_LEN);
    let oversized_ix = [
        oversized.precompile(),
        fixture.consume_instruction(oversized.nonce_hash()),
    ];
    let result = fixture.send_as_consumer(&oversized_ix);
    assert_verifier_error(result, 1, VerifierError::InvalidSigningInputLength);
    report(
        "over-limit verdict (rejected by the verifier)",
        &measure(&mut fixture, &oversized_ix, false),
    );

    let one_byte = fixture.sign_input(b"e".to_vec());
    let one_byte_ix = [
        one_byte.precompile(),
        fixture.consume_instruction(one_byte.nonce_hash()),
    ];
    assert_verifier_error(
        fixture.send_as_consumer(&one_byte_ix),
        1,
        VerifierError::MalformedSigningInput,
    );
    report(
        "1-byte message",
        &measure(&mut fixture, &one_byte_ix, false),
    );

    let cases: [(&str, VerdictClaims, (u8, u32)); 3] = [
        (
            "parser rejection (exponent amount)",
            VerdictClaims {
                amount: "3.4e2".into(),
                ..VerdictClaims::default()
            },
            (1, u32::from(VerifierError::InvalidNumber)),
        ),
        (
            "last check rejection (category)",
            VerdictClaims {
                category: "travel".into(),
                ..VerdictClaims::default()
            },
            (1, u32::from(VerifierError::CategoryMismatch)),
        ),
        (
            "native rejection (high S)",
            VerdictClaims::default(),
            (0, PrecompileError::InvalidSignature as u32),
        ),
    ];
    for (name, claims, (index, code)) in cases {
        let mut verdict = fixture.sign(&claims);
        if name.starts_with("native") {
            verdict.signature = es256_jws_adapter::high_s_twin(&verdict.signature).unwrap();
        }
        let instructions = [
            verdict.precompile(),
            fixture.consume_instruction(verdict.nonce_hash()),
        ];
        assert_eq!(
            instruction_error(fixture.send_as_consumer(&instructions)),
            (index, InstructionError::Custom(code)),
            "{name}"
        );
        let measurement = measure(&mut fixture, &instructions, false);
        assert!(measurement.compute_units <= DEFAULT_INSTRUCTION_COMPUTE_UNITS);
        report(name, &measurement);
    }
}

/// `SBF_OUT_DIR` selects the binary with no fallback. When
/// `ES256_EXPECTED_SBPF_ARCH` is set, the loaded ELF must be that
/// architecture, so a v3 run cannot silently test the v0 build.
#[test]
fn harness_loads_the_requested_sbpf_artifact() {
    for name in ["es256_authorization_verifier", "es256_cpi_forwarder"] {
        let elf = built_program(name);
        assert_eq!(&elf[..4], b"\x7fELF", "{name} is an ELF");
        let flags = u32::from_le_bytes(elf[48..52].try_into().unwrap());
        println!("{}/{name}.so: e_flags {flags}", sbf_out_dir());
        match std::env::var("ES256_EXPECTED_SBPF_ARCH").as_deref() {
            Ok("v0") => assert_eq!(flags, 0, "{name} must be SBPFv0"),
            Ok("v3") => assert_eq!(flags, 3, "{name} must be SBPFv3"),
            Ok(other) => panic!("unknown ES256_EXPECTED_SBPF_ARCH {other}"),
            Err(_) => assert!(flags == 0 || flags == 3, "unexpected e_flags {flags}"),
        }
    }
}
