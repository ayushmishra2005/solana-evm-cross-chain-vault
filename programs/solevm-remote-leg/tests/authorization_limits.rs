//! Size, compute and rent of the authorized deployment, as a legacy
//! transaction and as a v0 transaction through an address lookup table, and a
//! check that the harness loaded the intended SBPF binaries.
//!
//! Run with `--nocapture` to see the measurements.

#![allow(clippy::unwrap_used, clippy::panic, clippy::arithmetic_side_effects)]

mod common;

use anchor_lang::solana_program::instruction::Instruction;
use common::{
    ALLOCATE_TRANSFER_ID, Fixture, Pubkey, SignedApproval, consumed_units, loaded_programs,
};
use litesvm::types::TransactionResult;
use solana_address_lookup_table_interface::instruction::{
    create_lookup_table, extend_lookup_table,
};
use solana_compute_budget_interface::ComputeBudgetInstruction;
use solana_instruction::error::InstructionError;
use solana_message::{AddressLookupTableAccount, VersionedMessage, v0};
use solana_signer::Signer;
use solana_transaction::Transaction;
use solana_transaction::versioned::VersionedTransaction;
use solana_transaction_error::TransactionError;

/// Legacy transaction packet limit.
const PACKET_DATA_SIZE: usize = 1_232;
/// Default per-instruction compute budget.
const DEFAULT_INSTRUCTION_COMPUTE_UNITS: u64 = 200_000;
/// Compute a relayer requests for the whole authorized deployment.
const AUTHORIZED_DEPLOY_LIMIT: u32 = 100_000;
const AMOUNT: u64 = 1_000_000;

struct Measurement {
    transaction: usize,
    total: u64,
    consume: u64,
    deploy: u64,
}

fn report(name: &str, measurement: &Measurement) {
    println!(
        "{name:<52} transaction {:>5} B   total {:>6} CU   consume {:>6} CU   deploy {:>6} CU",
        measurement.transaction, measurement.total, measurement.consume, measurement.deploy
    );
}

fn budget(limit: u32) -> [Instruction; 2] {
    [
        ComputeBudgetInstruction::set_compute_unit_limit(limit),
        ComputeBudgetInstruction::set_compute_unit_price(1),
    ]
}

fn funded() -> Fixture {
    let mut fixture = Fixture::deployed();
    fixture.fund_position(ALLOCATE_TRANSFER_ID, 4 * AMOUNT);
    fixture
}

impl Fixture {
    fn authorized(&mut self, prefix: &[Instruction]) -> (SignedApproval, Vec<Instruction>) {
        let approval = self.approve(AMOUNT);
        let mut instructions = prefix.to_vec();
        instructions.extend(self.authorized_deploy(&approval, self.strategy_accounts(), AMOUNT));
        (approval, instructions)
    }

    fn legacy(&mut self, instructions: &[Instruction]) -> Transaction {
        self.svm.expire_blockhash();
        let relayer = self.outsider.insecure_clone();
        Transaction::new_signed_with_payer(
            instructions,
            Some(&relayer.pubkey()),
            &[&relayer],
            self.svm.latest_blockhash(),
        )
    }

    fn versioned(
        &mut self,
        instructions: &[Instruction],
        tables: &[AddressLookupTableAccount],
    ) -> VersionedTransaction {
        self.svm.expire_blockhash();
        let relayer = self.outsider.insecure_clone();
        let message = v0::Message::try_compile(
            &relayer.pubkey(),
            instructions,
            tables,
            self.svm.latest_blockhash(),
        )
        .unwrap();
        VersionedTransaction::try_new(VersionedMessage::V0(message), &[&relayer]).unwrap()
    }

    /// Every account of the authorized deployment that stays the same from
    /// one deployment to the next, in an activated lookup table.
    fn deployment_lookup_table(&mut self) -> AddressLookupTableAccount {
        let authority = self.administrator.insecure_clone();
        let slot = self
            .svm
            .get_sysvar::<anchor_lang::solana_program::clock::Clock>()
            .slot;
        let (create, table) = create_lookup_table(authority.pubkey(), authority.pubkey(), slot);
        let template = self.authorized(&[]).1;
        let mut addresses: Vec<Pubkey> = Vec::new();
        for instruction in &template {
            for meta in &instruction.accounts {
                let invoked = template.iter().any(|ix| ix.program_id == meta.pubkey);
                let per_approval = meta.pubkey == template[2].accounts[13].pubkey;
                if !meta.is_signer && !invoked && !per_approval && !addresses.contains(&meta.pubkey)
                {
                    addresses.push(meta.pubkey);
                }
            }
        }
        let extend = extend_lookup_table(
            table,
            authority.pubkey(),
            Some(authority.pubkey()),
            addresses.clone(),
        );
        self.send_instructions(&[create, extend], &authority, &[&authority])
            .expect("the lookup table is created and extended");
        self.svm.warp_to_slot(slot + 1);
        AddressLookupTableAccount {
            key: table,
            addresses,
        }
    }

    fn measure(&mut self, result: TransactionResult, transaction: usize) -> Measurement {
        let meta = result.expect("the authorized deployment lands");
        Measurement {
            transaction,
            total: meta.compute_units_consumed,
            consume: consumed_units(&meta.logs, &es256_authorization_verifier::ID),
            deploy: consumed_units(&meta.logs, &solevm_remote_leg::ID),
        }
    }
}

fn legacy_size(transaction: &Transaction) -> usize {
    1 + 64 * transaction.signatures.len() + transaction.message_data().len()
}

fn versioned_size(transaction: &VersionedTransaction) -> usize {
    1 + 64 * transaction.signatures.len() + transaction.message.serialize().len()
}

#[test]
fn a_legacy_authorized_deployment_fits_one_packet() {
    let mut fixture = funded();

    let (_, plain) = fixture.authorized(&[]);
    let transaction = fixture.legacy(&plain);
    let size = legacy_size(&transaction);
    let result = fixture.svm.send_transaction(transaction);
    let plain_m = fixture.measure(result, size);
    report("legacy [secp256r1, consume, deploy]", &plain_m);

    let (_, budgeted) = fixture.authorized(&budget(AUTHORIZED_DEPLOY_LIMIT));
    let transaction = fixture.legacy(&budgeted);
    let size = legacy_size(&transaction);
    let result = fixture.svm.send_transaction(transaction);
    let budgeted_m = fixture.measure(result, size);
    report("legacy with compute limit and price", &budgeted_m);

    for measurement in [&plain_m, &budgeted_m] {
        assert!(measurement.transaction <= PACKET_DATA_SIZE);
        assert!(measurement.consume <= DEFAULT_INSTRUCTION_COMPUTE_UNITS);
        assert!(measurement.deploy <= DEFAULT_INSTRUCTION_COMPUTE_UNITS);
        assert!(measurement.total <= u64::from(AUTHORIZED_DEPLOY_LIMIT));
    }
    println!(
        "legacy headroom with the compute-budget prefix: {} bytes",
        PACKET_DATA_SIZE - budgeted_m.transaction
    );
}

#[test]
fn a_v0_authorized_deployment_resolves_through_a_lookup_table() {
    let mut fixture = funded();
    let table = fixture.deployment_lookup_table();
    println!("lookup table holds {} addresses", table.addresses.len());

    let (approval, instructions) = fixture.authorized(&budget(AUTHORIZED_DEPLOY_LIMIT));
    let legacy = legacy_size(&fixture.legacy(&instructions));
    let transaction = fixture.versioned(&instructions, std::slice::from_ref(&table));
    let VersionedMessage::V0(message) = &transaction.message else {
        panic!("expected a v0 message");
    };
    assert_eq!(message.address_table_lookups.len(), 1);
    let size = versioned_size(&transaction);
    let result = fixture.svm.send_transaction(transaction);
    let measurement = fixture.measure(result, size);
    report(
        "v0 through the lookup table, with compute budget",
        &measurement,
    );
    println!("saved against legacy: {} bytes", legacy - size);

    assert!(size < legacy);
    assert!(measurement.total <= u64::from(AUTHORIZED_DEPLOY_LIMIT));
    assert_eq!(fixture.position().deployed_principal, AMOUNT);
    assert_eq!(fixture.receipt(&approval.nonce()).unwrap().amount, AMOUNT);
}

#[test]
fn an_unresolvable_lookup_table_fails_before_anything_runs() {
    let mut fixture = funded();
    let table = fixture.deployment_lookup_table();
    let (approval, instructions) = fixture.authorized(&[]);

    let missing = AddressLookupTableAccount {
        key: Pubkey::new_unique(),
        addresses: table.addresses.clone(),
    };
    let transaction = fixture.versioned(&instructions, &[missing]);
    let failure = fixture.svm.send_transaction(transaction).unwrap_err();
    assert_eq!(failure.err, TransactionError::AddressLookupTableNotFound);

    let mut beyond = table.clone();
    beyond.addresses.push(Pubkey::new_unique());
    let receipt = fixture.receipt_key(&approval.nonce());
    let mut rerouted = instructions.clone();
    rerouted[2].accounts[4].pubkey = *beyond.addresses.last().unwrap();
    let transaction = fixture.versioned(&rerouted, &[beyond]);
    let failure = fixture.svm.send_transaction(transaction).unwrap_err();
    assert_eq!(
        failure.err,
        TransactionError::InvalidAddressLookupTableIndex
    );

    assert!(fixture.svm.get_account(&receipt).is_none());
    assert_eq!(fixture.position().deployed_principal, 0);
    assert_eq!(fixture.token_amount(fixture.custody), 4 * AMOUNT);
}

#[test]
fn a_compute_limit_below_the_deployment_spends_nothing() {
    let mut fixture = funded();
    let (approval, instructions) = fixture.authorized(&budget(30_000));
    let result = fixture.relay(&instructions);
    let (index, error) = match result.unwrap_err().err {
        TransactionError::InstructionError(index, error) => (index, error),
        other => panic!("expected an instruction error, got {other:?}"),
    };
    println!("30 000 CU limit: instruction {index} failed with {error:?}");
    assert_eq!(error, InstructionError::ComputationalBudgetExceeded);
    assert!(fixture.receipt(&approval.nonce()).is_none());
    assert_eq!(fixture.position().deployed_principal, 0);

    let (approval, retried) = fixture.authorized(&budget(AUTHORIZED_DEPLOY_LIMIT));
    fixture.relay(&retried).expect("an adequate limit lands");
    assert_eq!(fixture.receipt(&approval.nonce()).unwrap().amount, AMOUNT);
}

#[test]
fn the_relayer_pays_the_receipt_rent() {
    let mut fixture = funded();
    let relayer = fixture.outsider.pubkey();
    let before = fixture.lamports(relayer);

    let (approval, instructions) = fixture.authorized(&[]);
    let transaction = fixture.legacy(&instructions);
    let meta = fixture
        .svm
        .send_transaction(transaction)
        .expect("the authorized deployment lands");

    let receipt = fixture
        .svm
        .get_account(&fixture.receipt_key(&approval.nonce()))
        .unwrap();
    let rent = fixture
        .svm
        .minimum_balance_for_rent_exemption(receipt.data.len());
    assert_eq!(
        receipt.data.len(),
        8 + <es256_authorization_verifier::DeploymentReceipt as anchor_lang::Space>::INIT_SPACE
    );
    assert_eq!(receipt.lamports, rent);
    assert_eq!(receipt.owner, es256_authorization_verifier::ID);
    assert_eq!(before - fixture.lamports(relayer), meta.fee + rent);
    println!(
        "receipt: {} bytes, {} lamports of permanent rent, fee {} lamports",
        receipt.data.len(),
        rent,
        meta.fee
    );
}

/// `SBF_OUT_DIR` and `HOSTILE_SBF_OUT_DIR` choose the binaries, with no
/// fallback. When `SOLEVM_EXPECTED_SBPF_ARCH` is set, every loaded ELF must
/// be that architecture, so a v3 run cannot silently fall back to a v0 build.
#[test]
fn harness_loads_the_requested_sbpf_artifacts() {
    for (name, elf) in loaded_programs() {
        assert_eq!(&elf[..4], b"\x7fELF", "{name} is an ELF");
        let flags = u32::from_le_bytes(elf[48..52].try_into().unwrap());
        println!("{name}: e_flags {flags}");
        match std::env::var("SOLEVM_EXPECTED_SBPF_ARCH").as_deref() {
            Ok("v0") => assert_eq!(flags, 0, "{name} must be SBPFv0"),
            Ok("v3") => assert_eq!(flags, 3, "{name} must be SBPFv3"),
            Ok(other) => panic!("unknown SOLEVM_EXPECTED_SBPF_ARCH {other}"),
            Err(_) => assert!(flags == 0 || flags == 3, "unexpected e_flags {flags}"),
        }
    }
}
