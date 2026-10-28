//! Only new strategy exposure needs a risk approval. Everything that reduces
//! or reports exposure keeps working with the authorization verifier gone.

#![allow(clippy::unwrap_used, clippy::panic, clippy::arithmetic_side_effects)]

mod common;

use anchor_lang::error::ErrorCode as AnchorError;
use common::{
    ALLOCATE_TRANSFER_ID, CONSUME_INDEX, Fixture, MAX_REMOTE_PRINCIPAL, RECALL_TRANSFER_ID,
    adapter_bytes, expect_anchor_error, expect_error, instruction_failure, risk_key_address,
};
use protocol_types::ResolveAction;
use solana_account::Account;
use solana_signer::Signer;
use solevm_remote_leg::{RemoteLegError, TransferStatus};

const AMOUNT: u64 = 1_000_000;
const EPOCH: u64 = 12;
const RECOVERY_ID: [u8; 32] = [0x5A; 32];

/// A leg with control, strategy and outbound state that never loaded the
/// authorization verifier, never had a deployment authority and never
/// adopted a policy, so no risk approval can exist for it.
fn never_authorized() -> Fixture {
    let mut fixture = Fixture::ready();
    fixture.install_adapter();
    fixture
        .initialize_strategy_state(MAX_REMOTE_PRINCIPAL)
        .expect("strategy state initializes");
    fixture
        .initialize_outbound_state()
        .expect("outbound state initializes");
    for key in [
        es256_authorization_verifier::ID,
        fixture.deployment_authority(),
        fixture.authorization_policy(),
    ] {
        assert!(fixture.svm.get_account(&key).is_none(), "{key} exists");
    }
    fixture
}

impl Fixture {
    /// Replaces the verifier with a program that refuses every call, and
    /// erases its deployment authority, the risk key and the leg's adopted
    /// policy.
    fn lose_the_authorization_verifier(&mut self) {
        self.svm
            .add_program(es256_authorization_verifier::ID, &adapter_bytes())
            .unwrap();
        let authority = self.deployment_authority();
        let risk_key = risk_key_address(&authority, &self.risk_public_key());
        for key in [authority, risk_key, self.authorization_policy()] {
            self.svm.set_account(key, Account::default()).unwrap();
        }

        let approval = self.approve_unchecked(1);
        let instructions = self.authorized_deploy(&approval, self.strategy_accounts(), 1);
        let (index, _) = instruction_failure(self.relay(&instructions));
        assert_eq!(index, CONSUME_INDEX, "the verifier is still reachable");
    }

    /// An approval signed without reading verifier state, which is gone.
    fn approve_unchecked(&mut self, amount: u64) -> common::SignedApproval {
        let now = self.now();
        let message = es256_authorization_verifier::deployment::DeploymentAuthorization {
            authority: self.deployment_authority(),
            network_domain: common::NETWORK_DOMAIN,
            policy_generation: 0,
            deployment_context: self.observed_context().commitment(),
            amount,
            nonce: self.fresh_nonce(),
            issued_at: now,
            expires_at: now + 60,
        };
        self.sign_approval(message)
    }
}

#[test]
fn a_recall_unwinds_and_reports_with_the_verifier_gone() {
    let mut fixture = Fixture::ready_to_publish();
    fixture.fund_position(ALLOCATE_TRANSFER_ID, AMOUNT);
    fixture.deploy(AMOUNT).expect("the deposit lands");
    fixture.lose_the_authorization_verifier();

    fixture.accept_recall(RECALL_TRANSFER_ID, AMOUNT);
    fixture
        .withdraw(RECALL_TRANSFER_ID, AMOUNT)
        .expect("the withdrawal lands");
    fixture
        .send_recall(RECALL_TRANSFER_ID, AMOUNT)
        .expect("the send lands");
    fixture
        .publish_recall_sent(RECALL_TRANSFER_ID)
        .expect("the recall notice publishes");
    fixture.publish_report(EPOCH).expect("the report publishes");
    fixture.reconcile().expect("custody reconciles");

    assert_eq!(
        fixture.transfer(&RECALL_TRANSFER_ID).status,
        TransferStatus::Complete
    );
    assert_eq!(fixture.position().deployed_principal, 0);
    assert_eq!(fixture.token_amount(fixture.escrow), AMOUNT);
}

#[test]
fn the_guardian_releases_a_frozen_leg_with_the_verifier_gone() {
    let mut fixture = Fixture::deployed();
    fixture.fund_position(ALLOCATE_TRANSFER_ID, AMOUNT);
    fixture.deploy(AMOUNT).expect("the deposit lands");
    fixture.lose_the_authorization_verifier();

    let guardian = fixture.guardian.insecure_clone();
    fixture.freeze(&guardian).expect("the guardian freezes");
    let sequence = fixture.control_lane().highest_consumed_sequence + 1;
    let bytes = fixture.recall_bytes(RECALL_TRANSFER_ID, AMOUNT, sequence);
    fixture
        .recall_cosigned(RECALL_TRANSFER_ID, sequence, bytes, &guardian)
        .expect("a co-signed recall lands while frozen");
    fixture
        .withdraw(RECALL_TRANSFER_ID, AMOUNT)
        .expect("the withdrawal lands while frozen");
    fixture
        .send_recall(RECALL_TRANSFER_ID, AMOUNT)
        .expect("the send lands while frozen");

    assert_eq!(fixture.position().deployed_principal, 0);
    assert_eq!(fixture.token_amount(fixture.escrow), AMOUNT);
}

#[test]
fn control_messages_and_housekeeping_proceed_with_the_verifier_gone() {
    let mut fixture = Fixture::deployed();
    fixture.lose_the_authorization_verifier();

    fixture.accept_updates(4);
    let administrator = fixture.administrator.insecure_clone();
    fixture
        .advance_watermark(&administrator, 2)
        .expect("the watermark advances");
    fixture.close_record(1).expect("the record closes");
    fixture.fund_position(ALLOCATE_TRANSFER_ID, AMOUNT);
    assert_eq!(fixture.position().attributed_principal, AMOUNT);
}

#[test]
fn closes_and_recoveries_settle_on_a_leg_that_never_loaded_the_verifier() {
    let mut fixture = never_authorized();
    let sequence = fixture.accept_allocation(ALLOCATE_TRANSFER_ID, AMOUNT);
    fixture.credit(fixture.custody, 600_000);
    let close = fixture.settlement_bytes(
        ALLOCATE_TRANSFER_ID,
        sequence,
        sequence + 1,
        ResolveAction::Close,
    );

    let outsider = fixture.outsider.insecure_clone();
    let administrator = fixture.administrator.insecure_clone();
    let mut forged =
        fixture.settlement_instruction(ALLOCATE_TRANSFER_ID, sequence + 1, close.clone(), None);
    forged.accounts[0].pubkey = outsider.pubkey();
    expect_error(
        fixture.send_as(forged, &administrator, &[&administrator, &outsider]),
        RemoteLegError::Unauthorized,
    );
    assert_eq!(fixture.control_lane().highest_consumed_sequence, sequence);

    let reports = fixture.outbound_lane().next_sequence;
    fixture
        .settle(ALLOCATE_TRANSFER_ID, sequence + 1, close.clone())
        .expect("the transport verifier's close settles");
    assert_eq!(
        fixture.transfer(&ALLOCATE_TRANSFER_ID).status,
        TransferStatus::Closed
    );
    let ledger = fixture
        .transport_ledger()
        .expect("the close opens the ledger");
    assert_eq!(ledger.cumulative_closed_credit, 600_000);
    assert_eq!(ledger.impaired_outstanding, 400_000);
    assert_eq!(fixture.outbound_lane().next_sequence, reports + 1);
    expect_error(
        fixture.settle(ALLOCATE_TRANSFER_ID, sequence + 1, close),
        RemoteLegError::InvalidSequence,
    );
    assert_eq!(fixture.transport_ledger(), Some(ledger));

    fixture.credit(fixture.custody, 300_000);
    expect_error(
        fixture.recover_impairment(RECOVERY_ID, sequence + 1),
        RemoteLegError::RecoveryStateMismatch,
    );
    fixture
        .recover_impairment(RECOVERY_ID, sequence)
        .expect("the recovery settles");
    let ledger = fixture.transport_ledger().unwrap();
    assert_eq!(ledger.cumulative_recovered, 300_000);
    assert_eq!(ledger.impaired_outstanding, 100_000);
    assert_eq!(
        fixture.transfer(&RECOVERY_ID).status,
        TransferStatus::Complete
    );
    assert_eq!(fixture.position().attributed_principal, 900_000);
    assert_eq!(fixture.outbound_lane().next_sequence, reports + 2);

    let receipt = fixture.receipt_key(&[0; 16]);
    let deploy = fixture.deploy_instruction(fixture.strategy_accounts(), 900_000, receipt);
    expect_anchor_error(fixture.relay(&[deploy]), AnchorError::AccountNotInitialized);
    assert_eq!(fixture.token_amount(fixture.custody), 900_000);
    assert_eq!(fixture.position().deployed_principal, 0);
}

#[test]
fn a_frozen_leg_unwinds_with_the_guardian_and_never_the_verifier() {
    let mut fixture = never_authorized();
    let sequence = fixture.accept_allocation(ALLOCATE_TRANSFER_ID, AMOUNT);
    fixture.credit(fixture.custody, 600_000);
    let administrator = fixture.administrator.insecure_clone();
    fixture.freeze(&administrator).expect("freeze lands");
    let guardian = fixture.guardian.insecure_clone();
    let outsider = fixture.outsider.insecure_clone();

    expect_error(
        fixture.close_allocation(ALLOCATE_TRANSFER_ID, sequence),
        RemoteLegError::Frozen,
    );
    let close = fixture.settlement_bytes(
        ALLOCATE_TRANSFER_ID,
        sequence,
        sequence + 1,
        ResolveAction::Close,
    );
    expect_error(
        fixture.settle_cosigned(ALLOCATE_TRANSFER_ID, sequence + 1, close.clone(), &outsider),
        RemoteLegError::Unauthorized,
    );
    fixture
        .settle_cosigned(ALLOCATE_TRANSFER_ID, sequence + 1, close, &guardian)
        .expect("the guardian's close lands");

    fixture.credit(fixture.custody, 400_000);
    expect_error(
        fixture.recover_impairment(RECOVERY_ID, sequence),
        RemoteLegError::Frozen,
    );
    let recover =
        fixture.settlement_bytes(RECOVERY_ID, sequence, sequence + 2, ResolveAction::Recover);
    fixture
        .settle_cosigned(RECOVERY_ID, sequence + 2, recover, &guardian)
        .expect("the guardian's recovery lands");
    assert_eq!(fixture.transport_ledger().unwrap().impaired_outstanding, 0);
    assert_eq!(fixture.position().attributed_principal, AMOUNT);

    let sequence = fixture.control_lane().highest_consumed_sequence + 1;
    let bytes = fixture.recall_bytes(RECALL_TRANSFER_ID, AMOUNT, sequence);
    expect_error(
        fixture.recall_cosigned(RECALL_TRANSFER_ID, sequence, bytes.clone(), &outsider),
        RemoteLegError::Unauthorized,
    );
    fixture
        .recall_cosigned(RECALL_TRANSFER_ID, sequence, bytes, &guardian)
        .expect("the guardian's recall lands");
    fixture
        .send_recall(RECALL_TRANSFER_ID, AMOUNT)
        .expect("the recalled principal leaves");
    assert_eq!(fixture.token_amount(fixture.escrow), AMOUNT);
    assert_eq!(fixture.token_amount(fixture.custody), 0);
}

#[test]
fn only_the_deployment_reads_authorization_state() {
    let source = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let allowed = [
        "lib.rs",
        "errors.rs",
        "events.rs",
        "authorization.rs",
        "instructions/mod.rs",
        "instructions/deploy.rs",
        "instructions/deploy_authorization.rs",
    ];
    let mut readers = Vec::new();
    for entry in walk(&source) {
        let text = std::fs::read_to_string(&entry).expect("source file reads");
        if [
            "es256_authorization_verifier",
            "authorization::",
            "DeployAuthorization",
            "DeploymentReceipt",
            "authorization_policy",
            "authorization_receipt",
        ]
        .iter()
        .any(|marker| text.contains(marker))
        {
            readers.push(entry);
        }
    }

    for reader in &readers {
        assert!(
            allowed.iter().any(|file| reader.ends_with(file)),
            "{} reads authorization state",
            reader.display()
        );
    }
    assert!(
        readers
            .iter()
            .any(|reader| reader.ends_with("instructions/deploy.rs")),
        "the deployment no longer reads the approval"
    );
}

fn walk(directory: &std::path::Path) -> Vec<std::path::PathBuf> {
    let mut files = Vec::new();
    for entry in std::fs::read_dir(directory).expect("source directory reads") {
        let path = entry.expect("directory entry").path();
        if path.is_dir() {
            files.extend(walk(&path));
        } else if path.extension().is_some_and(|kind| kind == "rs") {
            files.push(path);
        }
    }
    files
}
