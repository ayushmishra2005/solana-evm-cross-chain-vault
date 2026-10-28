#![allow(
    clippy::unwrap_used,
    clippy::panic,
    clippy::arithmetic_side_effects,
    clippy::indexing_slicing
)]

mod common;

use anchor_lang::error::ErrorCode as AnchorError;
use anchor_lang::prelude::Pubkey;
use anchor_lang::solana_program::instruction::{AccountMeta, Instruction};
use common::deployment::*;
use common::*;
use es256_authorization_verifier::VerifierError;
use es256_authorization_verifier::deployment::{
    DEPLOY_ACCOUNT_COUNT, DEPLOY_RECEIPT_INDEX, DEPLOY_REMOTE_CONFIG_INDEX,
    DeploymentAuthorization, MAX_LIFETIME_SECONDS,
};
use es256_jws_adapter::{SyntheticSigner, high_s_twin};
use solana_instruction::error::InstructionError;
use solana_precompile_error::PrecompileError;
use solana_signer::Signer;

/// Sends `[precompile, consume, following]` relayed by the outsider.
fn send_with_following(
    fixture: &mut Fixture,
    profile: &Profile,
    approval: &SignedApproval,
    following: Instruction,
) -> TxResult {
    let instructions = [
        approval.precompile(),
        consume_deployment_instruction(fixture.outsider.pubkey(), profile, approval.message.nonce),
        following,
    ];
    let outsider = fixture.outsider.insecure_clone();
    fixture.send(&instructions, &[&outsider])
}

/// The SPL Token program LiteSVM loads by default.
fn spl_token_id() -> Pubkey {
    Pubkey::from_str_const("TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA")
}

fn approved(fixture: &Fixture, profile: &Profile) -> SignedApproval {
    fixture.approve(valid_message(profile))
}

fn with(
    fixture: &Fixture,
    profile: &Profile,
    change: impl FnOnce(&mut DeploymentAuthorization),
) -> SignedApproval {
    let mut message = valid_message(profile);
    change(&mut message);
    fixture.approve(message)
}

fn rejects(
    fixture: &mut Fixture,
    profile: &Profile,
    approval: &SignedApproval,
    error: VerifierError,
) {
    let result = fixture.consume_deployment(profile, approval);
    assert_verifier_error(result, 1, error);
    fixture.assert_no_deployment_receipt(profile, &approval.message.nonce);
}

#[test]
fn a_valid_approval_passes_every_verifier_check() {
    let (mut fixture, profile) = Fixture::deployment_profile();
    let approval = approved(&fixture, &profile);
    assert_consume_passed(fixture.consume_deployment(&profile, &approval));
    // The stand-in deployment failed, so nothing persisted.
    fixture.assert_no_deployment_receipt(&profile, &DEPLOY_NONCE);
}

#[test]
fn consume_without_a_following_deployment_is_rejected() {
    let (mut fixture, profile) = Fixture::deployment_profile();
    let approval = approved(&fixture, &profile);
    let instructions = [
        approval.precompile(),
        consume_deployment_instruction(fixture.outsider.pubkey(), &profile, DEPLOY_NONCE),
    ];
    let outsider = fixture.outsider.insecure_clone();
    assert_verifier_error(
        fixture.send(&instructions, &[&outsider]),
        1,
        VerifierError::MissingDeploymentInstruction,
    );
    fixture.assert_no_deployment_receipt(&profile, &DEPLOY_NONCE);
}

#[test]
fn the_following_instruction_must_target_the_bound_remote_leg() {
    let (mut fixture, profile) = Fixture::deployment_profile();
    let approval = approved(&fixture, &profile);
    for program_id in [anchor_lang::system_program::ID, spl_token_id()] {
        let mut following = deploy_instruction(&profile, DEPLOY_NONCE, DEPLOY_AMOUNT);
        following.program_id = program_id;
        assert_verifier_error(
            send_with_following(&mut fixture, &profile, &approval, following),
            1,
            VerifierError::InvalidDeploymentInstruction,
        );
    }
    fixture.assert_no_deployment_receipt(&profile, &DEPLOY_NONCE);
}

#[test]
fn the_following_instruction_must_deploy_exactly_the_signed_amount() {
    let (mut fixture, profile) = Fixture::deployment_profile();
    let approval = approved(&fixture, &profile);
    let canonical = deploy_instruction(&profile, DEPLOY_NONCE, DEPLOY_AMOUNT);
    let mut variants = Vec::new();
    for amount in [DEPLOY_AMOUNT - 1, DEPLOY_AMOUNT + 1, u64::MAX, 0] {
        variants.push(deploy_instruction(&profile, DEPLOY_NONCE, amount).data);
    }
    let mut other_selector = canonical.data.clone();
    other_selector[0] ^= 1;
    variants.push(other_selector);
    let mut trailing = canonical.data.clone();
    trailing.push(0);
    variants.push(trailing);
    variants.push(canonical.data[..15].to_vec());
    variants.push(Vec::new());
    for data in variants {
        let following = Instruction {
            data,
            ..canonical.clone()
        };
        assert_verifier_error(
            send_with_following(&mut fixture, &profile, &approval, following),
            1,
            VerifierError::InvalidDeploymentInstruction,
        );
    }
}

#[test]
fn the_following_instruction_must_name_this_namespace_and_receipt() {
    let (mut fixture, profile) = Fixture::deployment_profile();
    let approval = approved(&fixture, &profile);
    let canonical = deploy_instruction(&profile, DEPLOY_NONCE, DEPLOY_AMOUNT);
    let receipt = canonical.accounts[DEPLOY_RECEIPT_INDEX].clone();

    let mut cases: Vec<Vec<AccountMeta>> = Vec::new();
    let mut other_config = canonical.accounts.clone();
    other_config[DEPLOY_REMOTE_CONFIG_INDEX].pubkey = Pubkey::new_unique();
    cases.push(other_config);
    let mut other_receipt = canonical.accounts.clone();
    other_receipt[DEPLOY_RECEIPT_INDEX].pubkey =
        deployment_receipt_address(profile.config, &[0x99; 16]);
    cases.push(other_receipt);
    let mut moved_receipt = canonical.accounts.clone();
    moved_receipt[DEPLOY_RECEIPT_INDEX] = AccountMeta::new_readonly(Pubkey::new_unique(), false);
    moved_receipt[DEPLOY_RECEIPT_INDEX - 1] = receipt.clone();
    cases.push(moved_receipt);
    let mut short = canonical.accounts.clone();
    short.truncate(DEPLOY_ACCOUNT_COUNT - 1);
    cases.push(short);
    let mut long = canonical.accounts.clone();
    long.push(AccountMeta::new_readonly(Pubkey::new_unique(), false));
    cases.push(long);

    for accounts in cases {
        let following = Instruction {
            accounts,
            ..canonical.clone()
        };
        assert_verifier_error(
            send_with_following(&mut fixture, &profile, &approval, following),
            1,
            VerifierError::InvalidDeploymentInstruction,
        );
    }
}

#[test]
fn the_deployment_must_immediately_follow_consume() {
    let (mut fixture, profile) = Fixture::deployment_profile();
    let approval = approved(&fixture, &profile);
    let outsider = fixture.outsider.insecure_clone();
    let spacer = anchor_lang::solana_program::system_instruction::transfer(
        &outsider.pubkey(),
        &fixture.administrator.pubkey(),
        1,
    );
    let instructions = [
        approval.precompile(),
        consume_deployment_instruction(outsider.pubkey(), &profile, DEPLOY_NONCE),
        spacer,
        deploy_instruction(&profile, DEPLOY_NONCE, DEPLOY_AMOUNT),
    ];
    assert_verifier_error(
        fixture.send(&instructions, &[&outsider]),
        1,
        VerifierError::InvalidDeploymentInstruction,
    );
}

#[test]
fn consume_invoked_by_cpi_is_rejected() {
    let (mut fixture, profile) = Fixture::deployment_profile();
    let approval = approved(&fixture, &profile);
    let consume = consume_deployment_instruction(fixture.outsider.pubkey(), &profile, DEPLOY_NONCE);
    let mut accounts = vec![AccountMeta::new_readonly(
        es256_authorization_verifier::ID,
        false,
    )];
    accounts.extend(consume.accounts);
    let forwarded = Instruction {
        program_id: FORWARDER_ID,
        accounts,
        data: consume.data,
    };
    let instructions = [
        approval.precompile(),
        forwarded,
        deploy_instruction(&profile, DEPLOY_NONCE, DEPLOY_AMOUNT),
    ];
    let outsider = fixture.outsider.insecure_clone();
    assert_verifier_error(
        fixture.send(&instructions, &[&outsider]),
        1,
        VerifierError::NotTopLevel,
    );
    fixture.assert_no_deployment_receipt(&profile, &DEPLOY_NONCE);
}

#[test]
fn two_approvals_in_one_transaction_are_rejected() {
    let (mut fixture, profile) = Fixture::deployment_profile();
    let first = approved(&fixture, &profile);
    let second = with(&fixture, &profile, |message| message.nonce = [0x77; 16]);
    let outsider = fixture.outsider.insecure_clone();
    let instructions = [
        first.precompile(),
        consume_deployment_instruction(outsider.pubkey(), &profile, first.message.nonce),
        deploy_instruction(&profile, first.message.nonce, DEPLOY_AMOUNT),
        second.precompile(),
        consume_deployment_instruction(outsider.pubkey(), &profile, second.message.nonce),
        deploy_instruction(&profile, second.message.nonce, DEPLOY_AMOUNT),
    ];
    assert_verifier_error(
        fixture.send(&instructions, &[&outsider]),
        1,
        VerifierError::InvalidInstructionOrder,
    );
}

#[test]
fn the_signed_authority_must_be_this_config() {
    let (mut fixture, profile) = Fixture::deployment_profile();
    let other_config = fixture.add_remote_config(FORWARDER_ID);
    let deployer = fixture.deployer.insecure_clone();
    let other = fixture
        .initialize_deployment_authority(&deployer, FORWARDER_ID, other_config)
        .unwrap();
    for authority in [other, fixture.config, Pubkey::default()] {
        let approval = with(&fixture, &profile, |message| message.authority = authority);
        rejects(
            &mut fixture,
            &profile,
            &approval,
            VerifierError::AuthorityMismatch,
        );
    }
}

#[test]
fn an_approval_for_another_namespace_is_rejected_even_with_the_same_key() {
    let (mut fixture, profile) = Fixture::deployment_profile();
    let other_remote_config = fixture.add_remote_config(FORWARDER_ID);
    let deployer = fixture.deployer.insecure_clone();
    let other_config = fixture
        .initialize_deployment_authority(&deployer, FORWARDER_ID, other_remote_config)
        .unwrap();
    let public_key = fixture.signer.public_key().unwrap();
    let other_key = fixture
        .register_risk_key(other_config, public_key, KEY_NOT_BEFORE, KEY_NOT_AFTER)
        .unwrap();
    let other = Profile {
        remote_leg: FORWARDER_ID,
        remote_config: other_remote_config,
        config: other_config,
        risk_key: other_key,
    };

    // Signed for the first namespace, presented to the second.
    let approval = approved(&fixture, &profile);
    rejects(
        &mut fixture,
        &other,
        &approval,
        VerifierError::AuthorityMismatch,
    );

    // The first namespace's key presented with the second config.
    let crossed = Profile {
        risk_key: profile.risk_key,
        ..other
    };
    let approval = fixture.approve(valid_message(&other));
    assert_verifier_error(
        fixture.consume_deployment(&crossed, &approval),
        1,
        VerifierError::Unauthorized,
    );
    assert_consume_passed(fixture.consume_deployment(&other, &approval));
}

#[test]
fn the_network_domain_must_match() {
    let (mut fixture, profile) = Fixture::deployment_profile();
    let mut devnet = NETWORK_DOMAIN;
    devnet[31] ^= 1;
    for domain in [devnet, [0; 32], [0xFF; 32]] {
        let approval = with(&fixture, &profile, |message| {
            message.network_domain = domain
        });
        rejects(
            &mut fixture,
            &profile,
            &approval,
            VerifierError::NetworkDomainMismatch,
        );
    }
}

#[test]
fn advancing_the_policy_generation_voids_outstanding_approvals() {
    let (mut fixture, profile) = Fixture::deployment_profile();
    let outstanding = approved(&fixture, &profile);
    let early = with(&fixture, &profile, |message| message.policy_generation = 1);
    rejects(
        &mut fixture,
        &profile,
        &early,
        VerifierError::PolicyGenerationMismatch,
    );

    fixture.advance_policy_generation(profile.config).unwrap();
    rejects(
        &mut fixture,
        &profile,
        &outstanding,
        VerifierError::PolicyGenerationMismatch,
    );
    let current = with(&fixture, &profile, |message| message.policy_generation = 1);
    assert_consume_passed(fixture.consume_deployment(&profile, &current));
}

#[test]
fn a_zero_amount_is_rejected() {
    let (mut fixture, profile) = Fixture::deployment_profile();
    let approval = with(&fixture, &profile, |message| message.amount = 0);
    rejects(
        &mut fixture,
        &profile,
        &approval,
        VerifierError::InvalidAuthorizationAmount,
    );
}

#[test]
fn the_nonce_argument_must_be_the_signed_nonce() {
    let (mut fixture, profile) = Fixture::deployment_profile();
    let approval = approved(&fixture, &profile);
    let presented = [0x99; 16];
    let outsider = fixture.outsider.insecure_clone();
    let instructions = [
        approval.precompile(),
        consume_deployment_instruction(outsider.pubkey(), &profile, presented),
        deploy_instruction(&profile, presented, DEPLOY_AMOUNT),
    ];
    assert_verifier_error(
        fixture.send(&instructions, &[&outsider]),
        1,
        VerifierError::NonceMismatch,
    );
    fixture.assert_no_deployment_receipt(&profile, &presented);
}

#[test]
fn the_lifetime_is_positive_and_at_most_five_minutes() {
    let (mut fixture, profile) = Fixture::deployment_profile();
    for (issued_at, expires_at) in [
        (NOW, NOW),
        (NOW, NOW - 1),
        (NOW, NOW + MAX_LIFETIME_SECONDS + 1),
        (NOW - 10, NOW + MAX_LIFETIME_SECONDS),
        (i64::MAX - 10, i64::MAX),
    ] {
        let approval = with(&fixture, &profile, |message| {
            message.issued_at = issued_at;
            message.expires_at = expires_at;
        });
        rejects(
            &mut fixture,
            &profile,
            &approval,
            VerifierError::InvalidAuthorizationLifetime,
        );
    }
    let shortest = with(&fixture, &profile, |message| message.expires_at = NOW + 1);
    assert_consume_passed(fixture.consume_deployment(&profile, &shortest));
}

#[test]
fn issue_time_may_lead_the_clock_by_at_most_sixty_seconds() {
    let (mut fixture, profile) = Fixture::deployment_profile();
    let lead = |seconds: i64| {
        move |message: &mut DeploymentAuthorization| {
            message.issued_at = NOW + seconds;
            message.expires_at = NOW + seconds + MAX_LIFETIME_SECONDS;
        }
    };
    let ahead = with(&fixture, &profile, lead(61));
    rejects(
        &mut fixture,
        &profile,
        &ahead,
        VerifierError::IssuedInFuture,
    );
    let edge = with(&fixture, &profile, lead(60));
    assert_consume_passed(fixture.consume_deployment(&profile, &edge));
}

#[test]
fn an_approval_expires_at_its_expiry_second() {
    let (mut fixture, profile) = Fixture::deployment_profile();
    let approval = approved(&fixture, &profile);
    fixture.set_clock(approval.message.expires_at - 1);
    assert_consume_passed(fixture.consume_deployment(&profile, &approval));
    fixture.set_clock(approval.message.expires_at);
    rejects(&mut fixture, &profile, &approval, VerifierError::Expired);
}

#[test]
fn the_risk_key_window_bounds_the_approval_and_the_clock() {
    let (mut fixture, _) = Fixture::deployment_profile();
    let remote_config = fixture.add_remote_config(FORWARDER_ID);
    let deployer = fixture.deployer.insecure_clone();
    let config = fixture
        .initialize_deployment_authority(&deployer, FORWARDER_ID, remote_config)
        .unwrap();
    let public_key = fixture.signer.public_key().unwrap();
    let not_before = NOW + 30;
    let not_after = NOW + 30 + MAX_LIFETIME_SECONDS;
    let risk_key = fixture
        .register_risk_key(config, public_key, not_before, not_after)
        .unwrap();
    let profile = Profile {
        remote_leg: FORWARDER_ID,
        remote_config,
        config,
        risk_key,
    };

    // Issued inside the window, but the clock has not reached it.
    let early = with(&fixture, &profile, |message| {
        message.issued_at = not_before;
        message.expires_at = not_before + 10;
    });
    rejects(
        &mut fixture,
        &profile,
        &early,
        VerifierError::OutsideKeyWindow,
    );

    fixture.set_clock(not_before);
    let before_window = with(&fixture, &profile, |message| {
        message.issued_at = not_before - 1;
        message.expires_at = not_before + 10;
    });
    rejects(
        &mut fixture,
        &profile,
        &before_window,
        VerifierError::OutsideKeyWindow,
    );
    let past_window = with(&fixture, &profile, |message| {
        message.issued_at = not_before + 10;
        message.expires_at = not_after + 1;
    });
    rejects(
        &mut fixture,
        &profile,
        &past_window,
        VerifierError::OutsideKeyWindow,
    );
    let inside = with(&fixture, &profile, |message| {
        message.issued_at = not_before;
        message.expires_at = not_after;
    });
    assert_consume_passed(fixture.consume_deployment(&profile, &inside));
}

#[test]
fn a_revoked_risk_key_cannot_approve() {
    let (mut fixture, profile) = Fixture::deployment_profile();
    let approval = approved(&fixture, &profile);
    fixture
        .revoke_risk_key(profile.config, profile.risk_key)
        .unwrap();
    rejects(&mut fixture, &profile, &approval, VerifierError::KeyRevoked);
}

#[test]
fn a_signature_by_an_unregistered_key_is_rejected() {
    let (mut fixture, profile) = Fixture::deployment_profile();
    let stranger = SyntheticSigner::generate().unwrap();
    let approval = SignedApproval::sign(&stranger, valid_message(&profile));
    rejects(
        &mut fixture,
        &profile,
        &approval,
        VerifierError::PublicKeyMismatch,
    );
}

#[test]
fn a_payment_profile_key_or_config_cannot_stand_in() {
    let (mut fixture, profile) = Fixture::deployment_profile();
    let public_key = fixture.signer.public_key().unwrap();
    let deployer = fixture.deployer.insecure_clone();
    fixture
        .initialize_config(
            &deployer,
            es256_authorization_verifier::Environment::Production,
        )
        .unwrap();
    let trusted_key = fixture.register_key(KID, public_key).unwrap();
    let approval = approved(&fixture, &profile);

    let with_trusted_key = Profile {
        risk_key: trusted_key,
        ..profile
    };
    assert_anchor_error(
        fixture.consume_deployment(&with_trusted_key, &approval),
        1,
        AnchorError::AccountDiscriminatorMismatch,
    );

    let outsider = fixture.outsider.insecure_clone();
    let mut consume = consume_deployment_instruction(outsider.pubkey(), &profile, DEPLOY_NONCE);
    consume.accounts[deployment_account::CONFIG].pubkey = fixture.config;
    let instructions = [
        approval.precompile(),
        consume,
        deploy_instruction(&profile, DEPLOY_NONCE, DEPLOY_AMOUNT),
    ];
    assert_anchor_error(
        fixture.send(&instructions, &[&outsider]),
        1,
        AnchorError::AccountDiscriminatorMismatch,
    );
}

#[test]
fn a_payment_verdict_is_not_a_deployment_approval_and_vice_versa() {
    let (mut fixture, profile) = Fixture::deployment_profile();
    let verdict = fixture.sign(&VerdictClaims::default());
    let as_approval = SignedApproval {
        message: valid_message(&profile),
        signing_input: verdict.signing_input.clone(),
        signature: verdict.signature,
        public_key: verdict.public_key,
    };
    rejects(
        &mut fixture,
        &profile,
        &as_approval,
        VerifierError::InvalidAuthorizationMessage,
    );

    // A deployment approval presented to the payment profile's consume.
    let deployer = fixture.deployer.insecure_clone();
    fixture
        .initialize_config(
            &deployer,
            es256_authorization_verifier::Environment::Production,
        )
        .unwrap();
    let public_key = fixture.signer.public_key().unwrap();
    fixture.trusted_key = fixture.register_key(KID, public_key).unwrap();
    fixture.mandate = fixture.register_mandate(approved_mandate()).unwrap();
    let approval = approved(&fixture, &profile);
    let as_verdict = SignedVerdict {
        signing_input: approval.signing_input.clone(),
        signature: approval.signature,
        public_key: approval.public_key,
        nonce: NONCE.into(),
    };
    let (index, error) = instruction_error(fixture.consume(&as_verdict));
    assert_eq!(index, 1);
    assert!(
        [
            VerifierError::MalformedSigningInput,
            VerifierError::InvalidBase64Url
        ]
        .iter()
        .any(|parse| error == InstructionError::Custom(u32::from(*parse))),
        "{error:?}"
    );
    fixture.assert_no_receipt(&as_verdict.nonce_hash());
}

#[test]
fn only_the_one_profile_encoding_is_accepted() {
    let (mut fixture, profile) = Fixture::deployment_profile();
    let canonical = valid_message(&profile).encode();
    let mut cases = vec![
        (
            canonical[..153].to_vec(),
            VerifierError::InvalidAuthorizationMessage,
        ),
        (
            [canonical.clone(), vec![0]].concat(),
            VerifierError::InvalidAuthorizationMessage,
        ),
    ];
    for (index, value, error) in [
        (0, b'X', VerifierError::InvalidAuthorizationMessage),
        (8, 1, VerifierError::InvalidAuthorizationMessage),
        (8, 3, VerifierError::InvalidAuthorizationMessage),
        (9, 0, VerifierError::UnsupportedOperation),
        (9, 2, VerifierError::UnsupportedOperation),
    ] {
        let mut bytes = canonical.clone();
        bytes[index] = value;
        cases.push((bytes, error));
    }
    for (bytes, error) in cases {
        let approval = SignedApproval::sign_bytes(&fixture.signer, valid_message(&profile), bytes);
        rejects(&mut fixture, &profile, &approval, error);
    }
}

#[test]
fn a_message_altered_after_signing_fails_natively() {
    let (mut fixture, profile) = Fixture::deployment_profile();
    let approval = approved(&fixture, &profile);
    let mut raised = valid_message(&profile);
    raised.amount = DEPLOY_AMOUNT * 2;
    let forged = SignedApproval {
        message: raised,
        signing_input: raised.encode(),
        ..approval
    };
    let (index, error) = instruction_error(fixture.consume_deployment(&profile, &forged));
    assert_eq!(
        (index, error),
        (
            0,
            InstructionError::Custom(PrecompileError::InvalidSignature as u32)
        )
    );
}

#[test]
fn a_high_s_signature_fails_natively() {
    let (mut fixture, profile) = Fixture::deployment_profile();
    let approval = approved(&fixture, &profile);
    let high = SignedApproval {
        signature: high_s_twin(&approval.signature).unwrap(),
        ..approval.clone()
    };
    assert_precompile_error(
        fixture.consume_deployment(&profile, &high),
        0,
        PrecompileError::InvalidSignature,
    );
    assert_consume_passed(fixture.consume_deployment(&profile, &approval));
}
