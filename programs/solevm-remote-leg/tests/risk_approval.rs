//! An application-level run of strategy deployment behind a risk decision.
//!
//! `TestRiskPolicy` is a test risk policy, not a production risk service. It
//! reads the leg as it is, refuses what exceeds fixed limits, and signs with
//! the synthetic P-256 key the deployment authority registered. Only its
//! signature reaches the chain; the verifier and the leg enforce that a
//! deployment spends exactly what it approved, once, while still fresh.

#![allow(clippy::unwrap_used, clippy::panic, clippy::arithmetic_side_effects)]

mod common;

use common::{
    ALLOCATE_TRANSFER_ID, CONSUME_INDEX, DEPLOY_INDEX, Fixture, MAX_REMOTE_PRINCIPAL,
    NETWORK_DOMAIN, Pubkey, RISK_SECRET, SignedApproval, code_of, instruction_failure,
};
use es256_authorization_verifier::VerifierError;
use es256_authorization_verifier::deployment::DeploymentAuthorization;
use es256_jws_adapter::SyntheticSigner;
use solana_instruction::error::InstructionError;
use solevm_remote_leg::RemoteLegError;

const MAX_SINGLE_DEPLOYMENT: u64 = 2_000_000;
const MAX_EXPOSURE_BPS: u64 = 5_000;
const APPROVAL_LIFETIME: i64 = 120;

#[derive(Debug, PartialEq, Eq)]
enum Refusal {
    Frozen,
    UnknownAdapter,
    ZeroAmount,
    AboveSingleLimit,
    AboveExposureLimit,
    AboveAttributedPrincipal,
}

/// A test risk policy, not a production risk service.
struct TestRiskPolicy {
    signer: SyntheticSigner,
    allowed_adapters: Vec<Pubkey>,
}

impl TestRiskPolicy {
    fn new() -> Self {
        Self {
            signer: SyntheticSigner::from_secret(&RISK_SECRET).unwrap(),
            allowed_adapters: vec![solevm_test_strategy::ID],
        }
    }

    /// Approves `amount` for the leg as it is now, or says why not.
    fn evaluate(&self, fixture: &Fixture, amount: u64) -> Result<SignedApproval, Refusal> {
        if fixture.config().frozen {
            return Err(Refusal::Frozen);
        }
        let context = fixture.observed_context();
        if !self.allowed_adapters.contains(&context.adapter_program) {
            return Err(Refusal::UnknownAdapter);
        }
        if amount == 0 {
            return Err(Refusal::ZeroAmount);
        }
        if amount > MAX_SINGLE_DEPLOYMENT {
            return Err(Refusal::AboveSingleLimit);
        }
        let exposure_limit = context.max_remote_principal * MAX_EXPOSURE_BPS / 10_000;
        if context.deployed_principal + amount > exposure_limit {
            return Err(Refusal::AboveExposureLimit);
        }
        if amount > fixture.position().attributed_principal {
            return Err(Refusal::AboveAttributedPrincipal);
        }

        let commitment = context.commitment();
        let issued_at = fixture.now();
        let digest = solana_keccak_hasher::hashv(&[
            b"test-risk-policy:nonce",
            &commitment,
            &amount.to_le_bytes(),
            &issued_at.to_le_bytes(),
        ])
        .to_bytes();
        let message = DeploymentAuthorization {
            authority: fixture.deployment_authority(),
            network_domain: NETWORK_DOMAIN,
            policy_generation: fixture.deployment_authority_state().policy_generation,
            deployment_context: commitment,
            amount,
            nonce: digest[..16].try_into().unwrap(),
            issued_at,
            expires_at: issued_at + APPROVAL_LIFETIME,
        };
        Ok(SignedApproval::sign(&self.signer, message))
    }
}

fn funded(amount: u64) -> Fixture {
    let mut fixture = Fixture::deployed();
    fixture.fund_position(ALLOCATE_TRANSFER_ID, amount);
    fixture
}

fn relay_approved(fixture: &mut Fixture, approval: &SignedApproval) -> common::TxResult {
    let amount = approval.message.amount;
    let instructions = fixture.authorized_deploy(approval, fixture.strategy_accounts(), amount);
    fixture.relay(&instructions)
}

#[test]
fn an_approved_deployment_lands_exactly_as_approved() {
    let mut fixture = funded(MAX_SINGLE_DEPLOYMENT);
    let policy = TestRiskPolicy::new();

    let approval = policy.evaluate(&fixture, MAX_SINGLE_DEPLOYMENT).unwrap();
    relay_approved(&mut fixture, &approval).expect("the approved deployment lands");

    assert_eq!(fixture.position().deployed_principal, MAX_SINGLE_DEPLOYMENT);
    assert_eq!(fixture.token_amount(fixture.custody), 0);
    assert_eq!(fixture.adapter_principal(), MAX_SINGLE_DEPLOYMENT);
    let receipt = fixture.receipt(&approval.nonce()).unwrap();
    assert_eq!(receipt.amount, MAX_SINGLE_DEPLOYMENT);
    assert_eq!(receipt.expires_at - receipt.issued_at, APPROVAL_LIFETIME);
    assert_eq!(
        receipt.deployment_context,
        approval.message.deployment_context
    );
}

#[test]
fn the_policy_refuses_what_exceeds_its_limits() {
    let mut fixture = funded(MAX_REMOTE_PRINCIPAL);
    let policy = TestRiskPolicy::new();

    assert_eq!(
        policy.evaluate(&fixture, 0).err(),
        Some(Refusal::ZeroAmount)
    );
    assert_eq!(
        policy.evaluate(&fixture, MAX_SINGLE_DEPLOYMENT + 1).err(),
        Some(Refusal::AboveSingleLimit)
    );

    for _ in 0..2 {
        let approval = policy.evaluate(&fixture, MAX_SINGLE_DEPLOYMENT).unwrap();
        relay_approved(&mut fixture, &approval).expect("deployment within the limits lands");
    }
    assert_eq!(
        policy.evaluate(&fixture, 1_000_001).err(),
        Some(Refusal::AboveExposureLimit)
    );
    let last = policy.evaluate(&fixture, 1_000_000).unwrap();
    relay_approved(&mut fixture, &last).expect("the exposure limit is reachable");
    assert_eq!(
        fixture.position().deployed_principal,
        MAX_REMOTE_PRINCIPAL / 2
    );
}

#[test]
fn the_policy_refuses_what_could_not_land() {
    let fixture = funded(1_000);
    let policy = TestRiskPolicy::new();
    assert_eq!(
        policy.evaluate(&fixture, 1_001).err(),
        Some(Refusal::AboveAttributedPrincipal)
    );
}

#[test]
fn the_policy_refuses_a_frozen_leg_and_an_unknown_adapter() {
    let mut fixture = funded(1_000_000);
    let mut policy = TestRiskPolicy::new();
    policy.allowed_adapters = vec![Pubkey::new_unique()];
    assert_eq!(
        policy.evaluate(&fixture, 1_000_000).err(),
        Some(Refusal::UnknownAdapter)
    );

    let policy = TestRiskPolicy::new();
    let administrator = fixture.administrator.insecure_clone();
    fixture.freeze(&administrator).unwrap();
    assert_eq!(
        policy.evaluate(&fixture, 1_000_000).err(),
        Some(Refusal::Frozen)
    );
}

#[test]
fn a_refused_request_leaves_the_relayer_with_nothing_to_spend() {
    let mut fixture = funded(MAX_REMOTE_PRINCIPAL);
    let policy = TestRiskPolicy::new();
    let refused = MAX_SINGLE_DEPLOYMENT + 1;
    assert!(policy.evaluate(&fixture, refused).is_err());

    let receipt = fixture.receipt_key(&[0; 16]);
    let instruction = fixture.deploy_instruction(fixture.strategy_accounts(), refused, receipt);
    assert_eq!(
        instruction_failure(fixture.relay(&[instruction])),
        (
            0,
            InstructionError::Custom(anchor_lang::error::ErrorCode::AccountNotInitialized as u32)
        )
    );

    let relayer_key = SyntheticSigner::from_secret(&[0x33; 32]).unwrap();
    let message = fixture.approval_message(refused);
    let self_signed = SignedApproval::sign(&relayer_key, message);
    assert_eq!(
        instruction_failure(relay_approved(&mut fixture, &self_signed)),
        (
            CONSUME_INDEX,
            InstructionError::Custom(u32::from(VerifierError::PublicKeyMismatch))
        )
    );
    assert_eq!(fixture.position().deployed_principal, 0);
}

#[test]
fn an_approval_answers_only_for_the_state_it_evaluated() {
    let mut fixture = funded(2 * MAX_SINGLE_DEPLOYMENT);
    let policy = TestRiskPolicy::new();
    let first = policy.evaluate(&fixture, MAX_SINGLE_DEPLOYMENT).unwrap();
    fixture.set_time(fixture.now() + 1);
    let second = policy.evaluate(&fixture, MAX_SINGLE_DEPLOYMENT).unwrap();
    assert_ne!(first.nonce(), second.nonce());

    relay_approved(&mut fixture, &first).expect("the first approval lands");
    assert_eq!(
        instruction_failure(relay_approved(&mut fixture, &second)),
        (
            DEPLOY_INDEX,
            InstructionError::Custom(code_of(RemoteLegError::AuthorizationContextMismatch))
        )
    );

    let fresh = policy.evaluate(&fixture, MAX_SINGLE_DEPLOYMENT).unwrap();
    relay_approved(&mut fixture, &fresh).expect("a re-evaluated approval lands");
    assert_eq!(
        fixture.position().deployed_principal,
        2 * MAX_SINGLE_DEPLOYMENT
    );
}

#[test]
fn the_policy_answers_the_same_question_the_same_way() {
    let fixture = funded(1_000_000);
    let policy = TestRiskPolicy::new();

    let first = policy.evaluate(&fixture, 1_000_000).unwrap();
    let second = policy.evaluate(&fixture, 1_000_000).unwrap();
    assert_eq!(first.signing_input, second.signing_input);
    assert_eq!(first.public_key, fixture.risk_public_key());
    assert_eq!(
        policy.evaluate(&fixture, 2_000_000).err(),
        Some(Refusal::AboveAttributedPrincipal)
    );
}

#[test]
fn asking_twice_cannot_double_the_approved_exposure() {
    let mut fixture = funded(2_000_000);
    let policy = TestRiskPolicy::new();
    let first = policy.evaluate(&fixture, 1_000_000).unwrap();
    let again = policy.evaluate(&fixture, 1_000_000).unwrap();

    relay_approved(&mut fixture, &first).expect("the approval lands");
    let (index, error) = instruction_failure(relay_approved(&mut fixture, &again));
    assert_eq!(index, CONSUME_INDEX);
    assert_eq!(
        error,
        InstructionError::Custom(0),
        "the receipt already exists"
    );
    assert_eq!(fixture.position().deployed_principal, 1_000_000);
}
