//! Strategy deployment spends a fresh risk approval, or nothing moves.
//!
//! Every test here calls `deploy_to_strategy` itself, alone or behind a
//! hostile variant of the authorized transaction, rather than trusting the
//! fixture helper that assembles the honest one.

#![allow(clippy::unwrap_used, clippy::panic, clippy::arithmetic_side_effects)]

mod common;

use anchor_lang::error::ErrorCode as AnchorError;
use anchor_lang::solana_program::instruction::{AccountMeta, Instruction};
use anchor_lang::{AccountSerialize, InstructionData, ToAccountMetas};
use common::{
    ALLOCATE_TRANSFER_ID, CONSUME_INDEX, DEPLOY_INDEX, FORWARDER_ID, Fixture, NETWORK_DOMAIN,
    Pubkey, SignedApproval, StrategyAccounts, TxResult, code_of, deploy_account,
    deployment_authority_address, forwarded, instruction_failure, last_event, receipt_address,
    risk_key_address,
};
use es256_authorization_verifier::deployment::{DeploymentContext, MAX_LIFETIME_SECONDS};
use es256_authorization_verifier::{
    ConsumedAuthorization, DeploymentReceipt, InitializeDeploymentAuthorityParams,
    RegisterRiskKeyParams, VerifierError,
};
use es256_jws_adapter::SyntheticSigner;
use solana_instruction::error::InstructionError;
use solana_signer::Signer;
use solevm_remote_leg::{DeployAuthorizationAdopted, RemoteLegError};
use solevm_test_strategy::AdapterError;

const AMOUNT: u64 = 1_000_000;

/// Fails unless instruction `index` refused the transaction with `code`.
#[track_caller]
fn expect_at(result: TxResult, index: u8, code: u32) {
    assert_eq!(
        instruction_failure(result),
        (index, InstructionError::Custom(code))
    );
}

fn leg(error: RemoteLegError) -> u32 {
    code_of(error)
}

fn verifier(error: VerifierError) -> u32 {
    u32::from(error)
}

fn anchor(error: AnchorError) -> u32 {
    error as u32
}

fn adapter(error: AdapterError) -> u32 {
    u32::from(error)
}

/// A deployed leg holding `funded` of attributed principal.
fn funded(funded: u64) -> Fixture {
    let mut fixture = Fixture::deployed();
    fixture.fund_position(ALLOCATE_TRANSFER_ID, funded);
    fixture
}

/// Fails unless nothing left custody and no strategy exposure was recorded.
#[track_caller]
fn assert_nothing_deployed(fixture: &Fixture, custody: u64, deployed: u64) {
    assert_eq!(fixture.token_amount(fixture.custody), custody);
    assert_eq!(fixture.position().deployed_principal, deployed);
}

impl Fixture {
    fn honest_deploy(&self, approval: &SignedApproval, amount: u64) -> Vec<Instruction> {
        self.authorized_deploy(approval, self.strategy_accounts(), amount)
    }

    fn deploy_alone(&self, amount: u64, receipt: Pubkey) -> Instruction {
        self.deploy_instruction(self.strategy_accounts(), amount, receipt)
    }

    /// Spends one honest approval of `amount` and returns its receipt.
    fn spend_approval(&mut self, amount: u64) -> Pubkey {
        let approval = self.approve(amount);
        let instructions = self.honest_deploy(&approval, amount);
        self.relay(&instructions).expect("honest deployment lands");
        self.receipt_key(&approval.nonce())
    }

    /// Signs an approval of `amount` over `context` instead of the leg's.
    fn approve_context(&mut self, amount: u64, context: &DeploymentContext) -> SignedApproval {
        let mut message = self.approval_message(amount);
        message.deployment_context = context.commitment();
        self.sign_approval(message)
    }
}

// Adoption

#[test]
fn a_leg_that_never_adopted_a_policy_deploys_nothing() {
    let mut fixture = Fixture::ready();
    fixture.install_adapter();
    fixture
        .initialize_strategy_state(common::MAX_REMOTE_PRINCIPAL)
        .expect("strategy state initializes");
    fixture.load_verifier();
    fixture
        .initialize_deployment_authority()
        .expect("deployment authority initializes");
    let public_key = fixture.risk_public_key();
    fixture.register_risk_key(public_key, 0, i64::MAX).unwrap();
    fixture.fund_position(ALLOCATE_TRANSFER_ID, AMOUNT);

    let approval = fixture.approve(AMOUNT);
    let instructions = fixture.honest_deploy(&approval, AMOUNT);
    expect_at(
        fixture.relay(&instructions),
        DEPLOY_INDEX,
        anchor(AnchorError::AccountNotInitialized),
    );
    assert_nothing_deployed(&fixture, AMOUNT, 0);
    assert!(fixture.receipt(&approval.nonce()).is_none());
}

#[test]
fn only_the_administrator_adopts_a_policy() {
    let mut fixture = Fixture::ready();
    fixture.install_adapter();
    fixture.load_verifier();
    fixture.initialize_deployment_authority().unwrap();
    let authority = fixture.deployment_authority();

    for signer in [
        fixture.guardian.insecure_clone(),
        fixture.outsider.insecure_clone(),
        fixture.risk_administrator.insecure_clone(),
    ] {
        let instruction = fixture.adopt_instruction(signer.pubkey(), fixture.config, authority);
        expect_at(
            fixture.send(instruction, &[&signer]),
            0,
            leg(RemoteLegError::Unauthorized),
        );
    }
    assert!(
        fixture
            .svm
            .get_account(&fixture.authorization_policy())
            .is_none()
    );
}

#[test]
fn a_policy_is_adopted_once_and_announced() {
    let mut fixture = Fixture::ready();
    fixture.load_verifier();
    fixture.initialize_deployment_authority().unwrap();
    fixture.adopt_deploy_authorization().unwrap();

    let policy = fixture.authorization_policy_state();
    assert_eq!(policy.state_version, solevm_remote_leg::STATE_VERSION);
    assert_eq!(policy.authorization_config, fixture.deployment_authority());
    assert_eq!(policy.adopted_at, fixture.now());
    assert_eq!(policy.reserved, [0; 32]);
    let event: DeployAuthorizationAdopted = last_event(&fixture.last_logs());
    assert_eq!(event.remote_config, fixture.config);
    assert_eq!(event.authorization_policy, fixture.authorization_policy());
    assert_eq!(event.authorization_config, fixture.deployment_authority());
    assert_eq!(event.network_domain, NETWORK_DOMAIN);

    let (index, error) = instruction_failure(fixture.adopt_deploy_authorization());
    assert_eq!(index, 0);
    assert_eq!(error, InstructionError::Custom(0), "account already in use");
    let unchanged = fixture.authorization_policy_state();
    assert_eq!(unchanged.authorization_config, policy.authorization_config);
    assert_eq!(unchanged.adopted_at, policy.adopted_at);
}

#[test]
fn only_the_authority_for_this_namespace_can_be_adopted() {
    let mut fixture = Fixture::ready();
    fixture.load_verifier();
    let other_config = fixture.add_second_vault([0x99; 32]);
    let deployer = fixture.risk_deployer.insecure_clone();
    let instruction = fixture.initialize_deployment_authority_instruction(
        deployer.pubkey(),
        other_config,
        InitializeDeploymentAuthorityParams {
            administrator: fixture.risk_administrator.pubkey(),
            network_domain: NETWORK_DOMAIN,
        },
    );
    fixture.send(instruction, &[&deployer]).unwrap();
    let administrator = fixture.administrator.insecure_clone();

    let foreign = deployment_authority_address(&other_config);
    let instruction = fixture.adopt_instruction(administrator.pubkey(), fixture.config, foreign);
    expect_at(
        fixture.send(instruction, &[&administrator]),
        0,
        anchor(AnchorError::ConstraintSeeds),
    );

    let missing = fixture.deployment_authority();
    let instruction = fixture.adopt_instruction(administrator.pubkey(), fixture.config, missing);
    expect_at(
        fixture.send(instruction, &[&administrator]),
        0,
        anchor(AnchorError::AccountNotInitialized),
    );

    let leg_owned = fixture.risk();
    let instruction = fixture.adopt_instruction(administrator.pubkey(), fixture.config, leg_owned);
    expect_at(
        fixture.send(instruction, &[&administrator]),
        0,
        anchor(AnchorError::AccountOwnedByWrongProgram),
    );
}

// The old path, called directly

#[test]
fn a_deployment_without_any_approval_is_refused() {
    let mut fixture = funded(AMOUNT);
    let never_consumed = fixture.receipt_key(&[0xAB; 16]);

    for receipt in [never_consumed, Pubkey::new_unique()] {
        let instruction = fixture.deploy_alone(AMOUNT, receipt);
        expect_at(
            fixture.relay(&[instruction]),
            0,
            anchor(AnchorError::AccountNotInitialized),
        );
    }
    assert_nothing_deployed(&fixture, AMOUNT, 0);
}

#[test]
fn a_spent_receipt_never_authorizes_another_deployment() {
    let mut fixture = funded(2 * AMOUNT);
    let spent = fixture.spend_approval(AMOUNT);

    let instruction = fixture.deploy_alone(AMOUNT, spent);
    expect_at(
        fixture.relay(&[instruction]),
        0,
        leg(RemoteLegError::DeploymentAuthorizationRequired),
    );
    assert_nothing_deployed(&fixture, AMOUNT, AMOUNT);
}

#[test]
fn a_forged_receipt_never_authorizes_a_deployment() {
    let mut fixture = funded(AMOUNT);
    let nonce = [0xF6; 16];
    let receipt_key = fixture.receipt_key(&nonce);
    let bump = Pubkey::find_program_address(
        &[
            es256_authorization_verifier::DEPLOYMENT_RECEIPT_SEED,
            fixture.deployment_authority().as_ref(),
            &nonce,
        ],
        &es256_authorization_verifier::ID,
    )
    .1;
    let now = fixture.now();
    let forged = DeploymentReceipt {
        version: 2,
        bump,
        config: fixture.deployment_authority(),
        remote_config: fixture.config,
        risk_key: risk_key_address(&fixture.deployment_authority(), &fixture.risk_public_key()),
        nonce,
        deployment_context: fixture.observed_context().commitment(),
        amount: AMOUNT,
        policy_generation: 0,
        issued_at: now,
        expires_at: now + MAX_LIFETIME_SECONDS,
        consumed_at: now,
        message_hash: [0; 32],
    };
    let mut data = Vec::new();
    forged.try_serialize(&mut data).unwrap();

    fixture.write_owned_account(receipt_key, data.clone(), es256_authorization_verifier::ID);
    let instruction = fixture.deploy_alone(AMOUNT, receipt_key);
    expect_at(
        fixture.relay(&[instruction]),
        0,
        leg(RemoteLegError::DeploymentAuthorizationRequired),
    );

    let elsewhere = Pubkey::new_unique();
    fixture.write_owned_account(elsewhere, data, solevm_remote_leg::ID);
    let instruction = fixture.deploy_alone(AMOUNT, elsewhere);
    expect_at(
        fixture.relay(&[instruction]),
        0,
        anchor(AnchorError::AccountOwnedByWrongProgram),
    );
    assert_nothing_deployed(&fixture, AMOUNT, 0);
}

#[test]
fn a_v1_payment_receipt_is_not_a_deployment_receipt() {
    let mut fixture = funded(AMOUNT);
    let v1 = ConsumedAuthorization {
        config: fixture.deployment_authority(),
        nonce_hash: [1; 32],
        signing_input_hash: [2; 32],
        jti_hash: [3; 32],
        mandate: fixture.config,
        consumed_at: fixture.now(),
        expires_at: fixture.now() + 60,
        bump: 255,
    };
    let mut data = Vec::new();
    v1.try_serialize(&mut data).unwrap();
    let key = Pubkey::new_unique();
    fixture.write_owned_account(key, data, es256_authorization_verifier::ID);

    let instruction = fixture.deploy_alone(AMOUNT, key);
    expect_at(
        fixture.relay(&[instruction]),
        0,
        anchor(AnchorError::AccountDiscriminatorMismatch),
    );
    assert_nothing_deployed(&fixture, AMOUNT, 0);
}

#[test]
fn a_spent_receipt_cannot_ride_behind_a_fresh_approval() {
    let mut fixture = funded(2 * AMOUNT);
    let spent = fixture.spend_approval(AMOUNT);

    let approval = fixture.approve(AMOUNT);
    let mut instructions = fixture.honest_deploy(&approval, AMOUNT);
    instructions[usize::from(DEPLOY_INDEX)].accounts[deploy_account::AUTHORIZATION_RECEIPT]
        .pubkey = spent;
    expect_at(
        fixture.relay(&instructions),
        CONSUME_INDEX,
        verifier(VerifierError::InvalidDeploymentInstruction),
    );
    assert_nothing_deployed(&fixture, AMOUNT, AMOUNT);
}

#[test]
fn one_approval_pays_for_one_deployment() {
    let mut fixture = funded(2 * AMOUNT);
    let approval = fixture.approve(AMOUNT);
    let mut instructions = fixture.honest_deploy(&approval, AMOUNT);
    instructions.push(instructions[usize::from(DEPLOY_INDEX)].clone());

    expect_at(
        fixture.relay(&instructions),
        DEPLOY_INDEX + 1,
        leg(RemoteLegError::DeploymentAuthorizationRequired),
    );
    assert_nothing_deployed(&fixture, 2 * AMOUNT, 0);
    assert!(fixture.receipt(&approval.nonce()).is_none());
}

#[test]
fn an_approval_is_spent_by_the_first_relay() {
    let mut fixture = funded(2 * AMOUNT);
    let approval = fixture.approve(AMOUNT);
    let instructions = fixture.honest_deploy(&approval, AMOUNT);
    fixture.relay(&instructions).expect("first relay lands");

    let (index, error) = instruction_failure(fixture.relay(&instructions));
    assert_eq!(index, CONSUME_INDEX);
    assert_eq!(error, InstructionError::Custom(0), "receipt already exists");
    assert_nothing_deployed(&fixture, AMOUNT, AMOUNT);
}

/// A deposit the strategy refuses rolls the whole transaction back, the
/// receipt included, so the same signed approval can be relayed again while
/// it is live, and is spent only by the attempt that lands.
#[test]
fn a_refused_deposit_leaves_the_same_approval_for_one_retry() {
    let mut fixture = funded(AMOUNT);
    let approval = fixture.approve(AMOUNT);
    let instructions = fixture.honest_deploy(&approval, AMOUNT);
    fixture
        .configure_adapter(u64::MAX, 0, true)
        .expect("the strategy pauses deposits");
    let attributed = fixture.position().attributed_principal;

    expect_at(
        fixture.relay(&instructions),
        DEPLOY_INDEX,
        adapter(AdapterError::DepositsPaused),
    );
    let refused_blockhash = fixture.svm.latest_blockhash();
    assert!(fixture.receipt(&approval.nonce()).is_none());
    assert_nothing_deployed(&fixture, AMOUNT, 0);
    assert_eq!(fixture.position().attributed_principal, attributed);
    assert_eq!(fixture.token_amount(fixture.adapter_vault), 0);

    fixture
        .configure_adapter(u64::MAX, 0, false)
        .expect("the strategy accepts deposits again");
    fixture.set_time(fixture.now() + 60);
    assert!(fixture.now() < approval.message.expires_at);
    fixture
        .relay(&instructions)
        .expect("the same signed approval lands on retry");
    assert_ne!(fixture.svm.latest_blockhash(), refused_blockhash);
    assert_nothing_deployed(&fixture, 0, AMOUNT);
    assert_eq!(fixture.token_amount(fixture.adapter_vault), AMOUNT);
    let receipt = fixture
        .receipt(&approval.nonce())
        .expect("the retry spent it");
    let signed = approval.message;
    assert_eq!(
        (receipt.nonce, receipt.amount, receipt.policy_generation),
        (signed.nonce, signed.amount, signed.policy_generation)
    );
    assert_eq!(
        (receipt.issued_at, receipt.expires_at),
        (signed.issued_at, signed.expires_at)
    );
    assert_eq!(receipt.deployment_context, signed.deployment_context);
    assert_eq!(receipt.consumed_at, fixture.now());

    let (index, error) = instruction_failure(fixture.relay(&instructions));
    assert_eq!(index, CONSUME_INDEX);
    assert_eq!(error, InstructionError::Custom(0), "receipt already exists");
    assert_nothing_deployed(&fixture, 0, AMOUNT);
}

#[test]
fn a_deployment_must_follow_its_approval_directly() {
    let mut fixture = funded(AMOUNT);
    fixture.load_cpi_forwarder();
    let approval = fixture.approve(AMOUNT);
    let mut instructions = fixture.honest_deploy(&approval, AMOUNT);
    let spacer = Instruction {
        program_id: FORWARDER_ID,
        accounts: vec![],
        data: vec![],
    };
    instructions.insert(usize::from(DEPLOY_INDEX), spacer);

    expect_at(
        fixture.relay(&instructions),
        CONSUME_INDEX,
        verifier(VerifierError::InvalidDeploymentInstruction),
    );
    assert_nothing_deployed(&fixture, AMOUNT, 0);
}

#[test]
fn a_deployment_reached_through_cpi_is_refused() {
    let mut fixture = funded(2 * AMOUNT);
    fixture.load_cpi_forwarder();

    let approval = fixture.approve(AMOUNT);
    let mut instructions = fixture.honest_deploy(&approval, AMOUNT);
    let deploy = instructions.pop().unwrap();
    instructions.push(forwarded(deploy));
    expect_at(
        fixture.relay(&instructions),
        CONSUME_INDEX,
        verifier(VerifierError::InvalidDeploymentInstruction),
    );

    let spent = fixture.spend_approval(AMOUNT);
    let instruction = forwarded(fixture.deploy_alone(AMOUNT, spent));
    expect_at(
        fixture.relay(&[instruction]),
        0,
        leg(RemoteLegError::DeploymentNotTopLevel),
    );
    assert_nothing_deployed(&fixture, AMOUNT, AMOUNT);
}

// What the approval names

#[test]
fn the_deployed_amount_is_exactly_the_approved_amount() {
    let mut fixture = funded(2 * AMOUNT);
    let approval = fixture.approve(AMOUNT);

    for amount in [AMOUNT - 1, AMOUNT + 1, 2 * AMOUNT] {
        let instructions = fixture.honest_deploy(&approval, amount);
        expect_at(
            fixture.relay(&instructions),
            CONSUME_INDEX,
            verifier(VerifierError::InvalidDeploymentInstruction),
        );
    }
    assert_nothing_deployed(&fixture, 2 * AMOUNT, 0);

    let instructions = fixture.honest_deploy(&approval, AMOUNT);
    fixture
        .relay(&instructions)
        .expect("the approved amount lands");
    assert_nothing_deployed(&fixture, AMOUNT, AMOUNT);
}

#[test]
fn an_approval_for_another_namespace_cannot_deploy_here() {
    let mut fixture = funded(AMOUNT);
    let other_config = fixture.add_second_vault([0x99; 32]);
    let deployer = fixture.risk_deployer.insecure_clone();
    let administrator = fixture.risk_administrator.insecure_clone();
    let instruction = fixture.initialize_deployment_authority_instruction(
        deployer.pubkey(),
        other_config,
        InitializeDeploymentAuthorityParams {
            administrator: administrator.pubkey(),
            network_domain: NETWORK_DOMAIN,
        },
    );
    fixture.send(instruction, &[&deployer]).unwrap();
    let other = deployment_authority_address(&other_config);
    let public_key = fixture.risk_public_key();
    let risk_key = risk_key_address(&other, &public_key);
    let register = Instruction {
        program_id: es256_authorization_verifier::ID,
        accounts: es256_authorization_verifier::accounts::RegisterRiskKey {
            administrator: administrator.pubkey(),
            config: other,
            risk_key,
            system_program: anchor_lang::system_program::ID,
        }
        .to_account_metas(None),
        data: es256_authorization_verifier::instruction::RegisterRiskKey {
            params: RegisterRiskKeyParams {
                public_key,
                not_before: 0,
                not_after: i64::MAX,
            },
        }
        .data(),
    };
    fixture.send(register, &[&administrator]).unwrap();

    let mut message = fixture.approval_message(AMOUNT);
    message.authority = other;
    let approval = fixture.sign_approval(message);
    let receipt = receipt_address(&other, &approval.nonce());
    let consume = Instruction {
        program_id: es256_authorization_verifier::ID,
        accounts: es256_authorization_verifier::accounts::ConsumeDeploymentAuthorization {
            payer: fixture.outsider.pubkey(),
            config: other,
            risk_key,
            receipt,
            instructions_sysvar: solana_instructions_sysvar::ID,
            system_program: anchor_lang::system_program::ID,
        }
        .to_account_metas(None),
        data: es256_authorization_verifier::instruction::ConsumeDeploymentAuthorization {
            nonce: approval.nonce(),
        }
        .data(),
    };
    let deploy = fixture.deploy_alone(AMOUNT, receipt);
    expect_at(
        fixture.relay(&[approval.precompile(), consume, deploy]),
        CONSUME_INDEX,
        verifier(VerifierError::InvalidDeploymentInstruction),
    );
    assert_nothing_deployed(&fixture, AMOUNT, 0);
}

#[test]
fn every_field_of_the_deployment_context_is_bound() {
    type Mutation = fn(&mut DeploymentContext);
    let mutations: [(&str, Mutation); 16] = [
        ("remote_leg_program", |c| {
            c.remote_leg_program = FORWARDER_ID
        }),
        ("remote_config", |c| c.remote_config = Pubkey::new_unique()),
        ("deployment_id", |c| c.deployment_id[0] ^= 1),
        ("vault_id", |c| c.vault_id[31] ^= 1),
        ("config_version", |c| c.config_version += 1),
        ("strategy_config", |c| {
            c.strategy_config = Pubkey::new_unique()
        }),
        ("adapter_program", |c| {
            c.adapter_program = Pubkey::new_unique()
        }),
        ("adapter_state", |c| c.adapter_state = Pubkey::new_unique()),
        ("adapter_authority", |c| {
            c.adapter_authority = Pubkey::new_unique()
        }),
        ("adapter_token_vault", |c| {
            c.adapter_token_vault = Pubkey::new_unique()
        }),
        ("max_remote_principal", |c| c.max_remote_principal += 1),
        ("asset_mint", |c| c.asset_mint = Pubkey::new_unique()),
        ("token_program", |c| c.token_program = Pubkey::new_unique()),
        ("custody_authority", |c| {
            c.custody_authority = Pubkey::new_unique()
        }),
        ("custody_token_account", |c| {
            c.custody_token_account = Pubkey::new_unique()
        }),
        ("deployed_principal", |c| c.deployed_principal += 1),
    ];

    let mut fixture = funded(AMOUNT);
    for (field, mutate) in mutations {
        let mut context = fixture.observed_context();
        mutate(&mut context);
        let approval = fixture.approve_context(AMOUNT, &context);
        let instructions = fixture.honest_deploy(&approval, AMOUNT);
        assert_eq!(
            instruction_failure(fixture.relay(&instructions)),
            (
                DEPLOY_INDEX,
                InstructionError::Custom(leg(RemoteLegError::AuthorizationContextMismatch))
            ),
            "an approval naming another {field} deployed"
        );
    }
    assert_nothing_deployed(&fixture, AMOUNT, 0);

    let honest = fixture.observed_context();
    let approval = fixture.approve_context(AMOUNT, &honest);
    let instructions = fixture.honest_deploy(&approval, AMOUNT);
    fixture
        .relay(&instructions)
        .expect("the honest context lands");
}

#[test]
fn an_approval_dies_with_the_configuration_it_named() {
    let mut fixture = funded(AMOUNT);
    let approval = fixture.approve(AMOUNT);
    fixture.accept_next_update();

    let instructions = fixture.honest_deploy(&approval, AMOUNT);
    expect_at(
        fixture.relay(&instructions),
        DEPLOY_INDEX,
        leg(RemoteLegError::AuthorizationContextMismatch),
    );
    assert_nothing_deployed(&fixture, AMOUNT, 0);
}

#[test]
fn an_approval_dies_once_exposure_has_moved() {
    let mut fixture = funded(2 * AMOUNT);
    let stale = fixture.approve(AMOUNT);
    fixture.spend_approval(AMOUNT);

    let instructions = fixture.honest_deploy(&stale, AMOUNT);
    expect_at(
        fixture.relay(&instructions),
        DEPLOY_INDEX,
        leg(RemoteLegError::AuthorizationContextMismatch),
    );
    assert_nothing_deployed(&fixture, AMOUNT, AMOUNT);
}

#[test]
fn an_approval_cannot_reroute_the_deployment_accounts() {
    let mut fixture = funded(AMOUNT);
    let approval = fixture.approve(AMOUNT);
    let mut accounts: StrategyAccounts = fixture.strategy_accounts();
    accounts.adapter_token_vault = Pubkey::new_unique();
    let instructions = fixture.authorized_deploy(&approval, accounts, AMOUNT);

    let (index, _) = instruction_failure(fixture.relay(&instructions));
    assert_eq!(index, DEPLOY_INDEX);
    assert_nothing_deployed(&fixture, AMOUNT, 0);
}

#[test]
fn the_gate_accounts_cannot_be_substituted() {
    let mut fixture = funded(AMOUNT);
    let cases = [
        (
            deploy_account::AUTHORIZATION_POLICY,
            fixture.risk(),
            AnchorError::AccountDiscriminatorMismatch,
        ),
        (
            deploy_account::AUTHORIZATION_POLICY,
            Pubkey::new_unique(),
            AnchorError::AccountNotInitialized,
        ),
        (
            deploy_account::INSTRUCTIONS_SYSVAR,
            Pubkey::new_unique(),
            AnchorError::ConstraintAddress,
        ),
    ];
    for (position, substitute, expected) in cases {
        let approval = fixture.approve(AMOUNT);
        let mut instructions = fixture.honest_deploy(&approval, AMOUNT);
        instructions[usize::from(DEPLOY_INDEX)].accounts[position] =
            AccountMeta::new_readonly(substitute, false);
        expect_at(fixture.relay(&instructions), DEPLOY_INDEX, anchor(expected));
    }
    assert_nothing_deployed(&fixture, AMOUNT, 0);
}

// Freshness and the risk authority's controls

#[test]
fn an_expired_approval_cannot_deploy() {
    let mut fixture = funded(AMOUNT);
    let approval = fixture.approve(AMOUNT);
    let expires_at = approval.message.expires_at;
    fixture.set_time(expires_at);

    let instructions = fixture.honest_deploy(&approval, AMOUNT);
    expect_at(
        fixture.relay(&instructions),
        CONSUME_INDEX,
        verifier(VerifierError::Expired),
    );
    assert_nothing_deployed(&fixture, AMOUNT, 0);
}

#[test]
fn advancing_the_policy_generation_voids_outstanding_approvals() {
    let mut fixture = funded(AMOUNT);
    let outstanding = fixture.approve(AMOUNT);
    fixture.advance_policy_generation().unwrap();

    let instructions = fixture.honest_deploy(&outstanding, AMOUNT);
    expect_at(
        fixture.relay(&instructions),
        CONSUME_INDEX,
        verifier(VerifierError::PolicyGenerationMismatch),
    );
    assert_nothing_deployed(&fixture, AMOUNT, 0);

    let current = fixture.approve(AMOUNT);
    let instructions = fixture.honest_deploy(&current, AMOUNT);
    fixture
        .relay(&instructions)
        .expect("a current approval lands");
}

#[test]
fn an_approval_is_good_only_for_the_generation_it_names() {
    let mut fixture = funded(2 * AMOUNT);
    let current = fixture.approve(AMOUNT);
    assert_eq!(current.message.policy_generation, 0);
    let instructions = fixture.honest_deploy(&current, AMOUNT);
    fixture
        .relay(&instructions)
        .expect("an approval of the current generation lands");

    let outstanding = fixture.approve(AMOUNT);
    let mut early = fixture.approval_message(AMOUNT);
    early.policy_generation = 1;
    let early = fixture.sign_approval(early);
    let instructions = fixture.honest_deploy(&early, AMOUNT);
    expect_at(
        fixture.relay(&instructions),
        CONSUME_INDEX,
        verifier(VerifierError::PolicyGenerationMismatch),
    );

    fixture.advance_policy_generation().unwrap();
    assert_eq!(fixture.deployment_authority_state().policy_generation, 1);
    let instructions = fixture.honest_deploy(&outstanding, AMOUNT);
    expect_at(
        fixture.relay(&instructions),
        CONSUME_INDEX,
        verifier(VerifierError::PolicyGenerationMismatch),
    );
    assert_nothing_deployed(&fixture, AMOUNT, AMOUNT);
    assert!(fixture.receipt(&outstanding.nonce()).is_none());

    let renewed = fixture.approve(AMOUNT);
    assert_eq!(renewed.message.policy_generation, 1);
    let instructions = fixture.honest_deploy(&renewed, AMOUNT);
    fixture
        .relay(&instructions)
        .expect("an approval of the new generation lands");
    assert_nothing_deployed(&fixture, 0, 2 * AMOUNT);
}

/// The generation check is exact, so a key that signed for a later generation
/// before it was compromised is stopped by revoking the key, not by advancing
/// the generation alone.
#[test]
fn approvals_signed_ahead_of_the_generation_die_with_their_key() {
    let signed_ahead = |fixture: &mut Fixture| {
        let mut message = fixture.approval_message(AMOUNT);
        message.policy_generation = 1;
        let approval = fixture.sign_approval(message);
        fixture.honest_deploy(&approval, AMOUNT)
    };

    let mut fixture = funded(AMOUNT);
    let instructions = signed_ahead(&mut fixture);
    fixture.advance_policy_generation().unwrap();
    let public_key = fixture.risk_public_key();
    fixture.revoke_risk_key(public_key).unwrap();
    expect_at(
        fixture.relay(&instructions),
        CONSUME_INDEX,
        verifier(VerifierError::KeyRevoked),
    );
    assert_nothing_deployed(&fixture, AMOUNT, 0);

    let mut unrevoked = funded(AMOUNT);
    let instructions = signed_ahead(&mut unrevoked);
    unrevoked.advance_policy_generation().unwrap();
    unrevoked
        .relay(&instructions)
        .expect("advancing alone leaves an approval for the new generation valid");
    assert_nothing_deployed(&unrevoked, 0, AMOUNT);
}

#[test]
fn a_revoked_key_signs_nothing_new_and_cannot_be_registered_again() {
    let mut fixture = funded(AMOUNT);
    let public_key = fixture.risk_public_key();
    fixture.revoke_risk_key(public_key).unwrap();

    for _ in 0..2 {
        let approval = fixture.approve(AMOUNT);
        let instructions = fixture.honest_deploy(&approval, AMOUNT);
        expect_at(
            fixture.relay(&instructions),
            CONSUME_INDEX,
            verifier(VerifierError::KeyRevoked),
        );
        fixture.advance_policy_generation().unwrap();
    }

    let (index, error) = instruction_failure(fixture.register_risk_key(public_key, 0, i64::MAX));
    assert_eq!(index, 0);
    assert_eq!(
        error,
        InstructionError::Custom(0),
        "the revoked key's account remains"
    );
    assert_nothing_deployed(&fixture, AMOUNT, 0);
}

/// The approval binds the adapter's program id, state, authority and vault,
/// not its code. Replacing the adapter's code under the same id leaves an
/// outstanding approval valid until the risk administrator advances the
/// generation.
#[test]
fn an_adapter_code_upgrade_voids_approvals_only_through_the_generation() {
    let mut fixture = funded(2 * AMOUNT);
    let before_upgrade = fixture.approve(AMOUNT);
    fixture.install_hostile_adapter();
    assert_eq!(
        fixture.observed_context().commitment(),
        before_upgrade.message.deployment_context,
        "nothing the approval names changed"
    );
    let instructions = fixture.honest_deploy(&before_upgrade, AMOUNT);
    fixture
        .relay(&instructions)
        .expect("the approval outlives the adapter's code change");
    assert_nothing_deployed(&fixture, AMOUNT, AMOUNT);

    let mut fixture = funded(AMOUNT);
    let before_upgrade = fixture.approve(AMOUNT);
    fixture.install_hostile_adapter();
    fixture.advance_policy_generation().unwrap();
    let instructions = fixture.honest_deploy(&before_upgrade, AMOUNT);
    expect_at(
        fixture.relay(&instructions),
        CONSUME_INDEX,
        verifier(VerifierError::PolicyGenerationMismatch),
    );
    assert_nothing_deployed(&fixture, AMOUNT, 0);

    let reviewed = fixture.approve(AMOUNT);
    let instructions = fixture.honest_deploy(&reviewed, AMOUNT);
    fixture
        .relay(&instructions)
        .expect("an approval issued after the upgrade lands");
    assert_nothing_deployed(&fixture, 0, AMOUNT);
}

#[test]
fn a_revoked_risk_key_approves_nothing() {
    let mut fixture = funded(AMOUNT);
    let approval = fixture.approve(AMOUNT);
    let public_key = fixture.risk_public_key();
    fixture.revoke_risk_key(public_key).unwrap();

    let instructions = fixture.honest_deploy(&approval, AMOUNT);
    expect_at(
        fixture.relay(&instructions),
        CONSUME_INDEX,
        verifier(VerifierError::KeyRevoked),
    );
    assert_nothing_deployed(&fixture, AMOUNT, 0);
}

#[test]
fn a_relayer_cannot_sign_its_own_approval() {
    let mut fixture = funded(AMOUNT);
    let impostor = SyntheticSigner::from_secret(&[0x17; 32]).unwrap();
    let message = fixture.approval_message(AMOUNT);
    let approval = SignedApproval::sign(&impostor, message);

    let instructions = fixture.honest_deploy(&approval, AMOUNT);
    expect_at(
        fixture.relay(&instructions),
        CONSUME_INDEX,
        verifier(VerifierError::PublicKeyMismatch),
    );
    assert_nothing_deployed(&fixture, AMOUNT, 0);
}

#[test]
fn a_relayer_cannot_alter_a_signed_approval() {
    let mut fixture = funded(2 * AMOUNT);
    let mut approval = fixture.approve(AMOUNT);
    let mut altered = approval.message;
    altered.amount = 2 * AMOUNT;
    approval.signing_input = altered.encode();

    let instructions = fixture.honest_deploy(&approval, 2 * AMOUNT);
    let (index, _) = instruction_failure(fixture.relay(&instructions));
    assert_eq!(index, 0, "the precompile refuses the altered message");
    assert_nothing_deployed(&fixture, 2 * AMOUNT, 0);
}

#[test]
fn a_frozen_leg_refuses_even_an_approved_deployment() {
    let mut fixture = funded(AMOUNT);
    let approval = fixture.approve(AMOUNT);
    let administrator = fixture.administrator.insecure_clone();
    fixture.freeze(&administrator).unwrap();

    let instructions = fixture.honest_deploy(&approval, AMOUNT);
    expect_at(
        fixture.relay(&instructions),
        DEPLOY_INDEX,
        leg(RemoteLegError::Frozen),
    );
    assert_nothing_deployed(&fixture, AMOUNT, 0);
}

#[test]
fn any_relayer_lands_exactly_the_approved_deployment() {
    let mut fixture = funded(AMOUNT);
    let approval = fixture.approve(AMOUNT);
    let instructions = fixture.honest_deploy(&approval, AMOUNT);
    let mut relayed = instructions.clone();
    let relayer = fixture.guardian.insecure_clone();
    relayed[usize::from(CONSUME_INDEX)].accounts[0].pubkey = relayer.pubkey();

    fixture
        .send_instructions(&relayed, &relayer, &[&relayer])
        .expect("an unrelated relayer may pay for the approved deployment");
    assert_nothing_deployed(&fixture, 0, AMOUNT);

    let receipt = fixture.receipt(&approval.nonce()).unwrap();
    assert_eq!(receipt.amount, AMOUNT);
    assert_eq!(receipt.remote_config, fixture.config);
    assert_eq!(receipt.consumed_at, fixture.now());
}
