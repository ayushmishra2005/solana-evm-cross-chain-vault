//! Token lifetime and clock boundaries, trusted-key windows, rotation and
//! environment separation, all on a deterministic LiteSVM clock.

#![allow(
    clippy::unwrap_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)]

mod common;

use anchor_lang::error::ErrorCode as AnchorError;
use common::*;
use es256_authorization_verifier::{Environment, VerifierError};
use es256_jws_adapter::SyntheticSigner;
use solana_instruction::error::InstructionError;
use solana_transaction_error::TransactionError;

fn claims_with_nonce(iat: i64, exp: i64, nonce: &str) -> VerdictClaims {
    VerdictClaims {
        iat,
        exp,
        nonce: nonce.into(),
        ..VerdictClaims::default()
    }
}

fn accepts(fixture: &mut Fixture, claims: &VerdictClaims, now: i64) {
    fixture.set_clock(now);
    let verdict = fixture.sign(claims);
    fixture
        .consume(&verdict)
        .unwrap_or_else(|error| panic!("{claims:?} at {now}: {:?}", error.err));
}

fn refuses(fixture: &mut Fixture, claims: &VerdictClaims, now: i64, expected: VerifierError) {
    fixture.set_clock(now);
    let verdict = fixture.sign(claims);
    assert_verifier_error(fixture.consume(&verdict), 1, expected);
    fixture.assert_no_receipt(&verdict.nonce_hash());
}

// ---- Lifetime and clock -----------------------------------------------------

#[test]
fn lifetime_must_be_exactly_300_seconds() {
    let mut fixture = Fixture::new();
    for exp in [NOW + 299, NOW + 301, NOW, NOW - 1, NOW + 3_600] {
        refuses(
            &mut fixture,
            &claims_with_nonce(NOW, exp, NONCE),
            NOW,
            VerifierError::InvalidLifetime,
        );
    }
    accepts(&mut fixture, &claims_with_nonce(NOW, NOW + 300, NONCE), NOW);
}

#[test]
fn issued_at_may_lead_the_cluster_clock_by_at_most_60_seconds() {
    let mut fixture = Fixture::new();
    refuses(
        &mut fixture,
        &claims_with_nonce(NOW + 61, NOW + 361, "n_future"),
        NOW,
        VerifierError::IssuedInFuture,
    );
    accepts(
        &mut fixture,
        &claims_with_nonce(NOW + 60, NOW + 360, "n_future"),
        NOW,
    );
}

#[test]
fn verdict_is_valid_until_but_not_at_expiry() {
    let mut fixture = Fixture::new();
    let claims = claims_with_nonce(NOW, NOW + 300, "n_expiry");
    refuses(&mut fixture, &claims, NOW + 300, VerifierError::Expired);
    refuses(&mut fixture, &claims, NOW + 301, VerifierError::Expired);
    refuses(&mut fixture, &claims, NOW + 86_400, VerifierError::Expired);
    accepts(&mut fixture, &claims, NOW + 299);
}

#[test]
fn timestamp_extremes_fail_deterministically() {
    let mut fixture = Fixture::new();
    // The largest accepted timestamp: `exp` cannot be 300 seconds later.
    let max = es256_authorization_verifier::jwt::MAX_TIMESTAMP;
    refuses(
        &mut fixture,
        &claims_with_nonce(max, max, NONCE),
        NOW,
        VerifierError::InvalidLifetime,
    );
    // A cluster clock at the top of i64 overflows the skew bound.
    refuses(
        &mut fixture,
        &claims_with_nonce(NOW, NOW + 300, NONCE),
        i64::MAX - 10,
        VerifierError::IssuedInFuture,
    );
    // A clock before the epoch leaves every token issued in the future.
    refuses(
        &mut fixture,
        &claims_with_nonce(NOW, NOW + 300, NONCE),
        -1,
        VerifierError::IssuedInFuture,
    );
}

// ---- Key windows ------------------------------------------------------------

#[test]
fn key_window_bounds_issue_time_and_expiry_inclusively() {
    let mut fixture = Fixture::new();
    let first = KEY_NOT_BEFORE;
    let last = KEY_NOT_AFTER - 300;

    refuses(
        &mut fixture,
        &claims_with_nonce(first - 1, first + 299, "n_window_start"),
        first - 1,
        VerifierError::OutsideKeyWindow,
    );
    accepts(
        &mut fixture,
        &claims_with_nonce(first, first + 300, "n_window_start"),
        first,
    );
    refuses(
        &mut fixture,
        &claims_with_nonce(last + 1, last + 301, "n_window_end"),
        last + 1,
        VerifierError::OutsideKeyWindow,
    );
    accepts(
        &mut fixture,
        &claims_with_nonce(last, last + 300, "n_window_end"),
        last,
    );
}

/// `not_before` is when the key starts authorizing. The 60-second `iat`
/// skew must not let a token signed at activation be consumed earlier.
#[test]
fn key_cannot_authorize_before_its_activation_time() {
    let mut fixture = Fixture::new();
    let first = KEY_NOT_BEFORE;
    let at_activation = claims_with_nonce(first, first + 300, "n_activation");
    for now in [first - 60, first - 30, first - 1] {
        refuses(
            &mut fixture,
            &at_activation,
            now,
            VerifierError::OutsideKeyWindow,
        );
    }
    accepts(&mut fixture, &at_activation, first);

    // The furthest a token can lead the clock at `first - 1` and pass skew.
    let led = claims_with_nonce(first + 59, first + 359, "n_activation_skew");
    refuses(
        &mut fixture,
        &led,
        first - 1,
        VerifierError::OutsideKeyWindow,
    );
    accepts(&mut fixture, &led, first);
}

/// The last second a key can authorize is `not_after - 1`: tokens must
/// expire by `not_after`, and a token is dead at its expiry.
#[test]
fn key_cannot_authorize_at_or_after_its_end() {
    let mut fixture = Fixture::new();
    let last = KEY_NOT_AFTER - 300;
    let final_token = claims_with_nonce(last, last + 300, "n_deactivation");
    refuses(
        &mut fixture,
        &final_token,
        KEY_NOT_AFTER,
        VerifierError::Expired,
    );
    accepts(&mut fixture, &final_token, KEY_NOT_AFTER - 1);
}

#[test]
fn registration_rejects_empty_or_negative_key_windows() {
    let mut fixture = Fixture::new();
    let public_key = SyntheticSigner::generate().unwrap().public_key().unwrap();
    for (index, (not_before, not_after)) in
        [(NOW, NOW), (NOW, NOW - 1), (-1, NOW), (i64::MIN, i64::MAX)]
            .into_iter()
            .enumerate()
    {
        let failure = fixture
            .register_key_with(
                &format!("window-{index}"),
                public_key,
                not_before,
                not_after,
            )
            .expect_err("window is refused");
        assert_eq!(
            failure.err,
            TransactionError::InstructionError(
                0,
                InstructionError::Custom(u32::from(VerifierError::InvalidKeyWindow))
            ),
            "{not_before}..{not_after}"
        );
    }
}

// ---- Revocation and rotation ------------------------------------------------

#[test]
fn revoked_key_stays_revoked() {
    let mut fixture = Fixture::new();
    fixture.revoke_key(fixture.trusted_key).unwrap();
    assert_verifier_error(
        fixture.revoke_key(fixture.trusted_key),
        0,
        VerifierError::AlreadyRevoked,
    );

    // The kid's account is a tombstone: registering it again fails.
    let replacement = SyntheticSigner::generate().unwrap().public_key().unwrap();
    let failure = fixture
        .register_key(KID, replacement)
        .expect_err("a revoked kid cannot be registered again");
    assert_eq!(
        failure.err,
        TransactionError::InstructionError(0, InstructionError::Custom(ACCOUNT_ALREADY_IN_USE))
    );
    refuses(
        &mut fixture,
        &VerdictClaims::default(),
        NOW,
        VerifierError::KeyRevoked,
    );
}

#[test]
fn unknown_key_ids_are_rejected() {
    let mut fixture = Fixture::new();
    refuses(
        &mut fixture,
        &VerdictClaims {
            kid: "verdict-unknown".into(),
            ..VerdictClaims::default()
        },
        NOW,
        VerifierError::KeyIdMismatch,
    );

    fixture.trusted_key = trusted_key_address(fixture.config, "verdict-unknown");
    let verdict = fixture.sign(&VerdictClaims {
        kid: "verdict-unknown".into(),
        ..VerdictClaims::default()
    });
    assert_anchor_error(
        fixture.consume(&verdict),
        1,
        AnchorError::AccountNotInitialized,
    );
    fixture.assert_no_receipt(&verdict.nonce_hash());
}

#[test]
fn rotation_keeps_the_new_key_working_after_the_old_one_is_revoked() {
    let mut fixture = Fixture::new();
    let old_key = fixture.trusted_key;
    let old_signer = std::mem::replace(&mut fixture.signer, SyntheticSigner::generate().unwrap());
    let new_key = fixture
        .register_key("verdict-test-02", fixture.signer.public_key().unwrap())
        .unwrap();
    let new_claims = |nonce: &str| VerdictClaims {
        kid: "verdict-test-02".into(),
        nonce: nonce.into(),
        ..VerdictClaims::default()
    };

    // Both keys are live during the overlap.
    fixture.trusted_key = new_key;
    accepts(&mut fixture, &new_claims("n_rotation_1"), NOW);
    fixture.trusted_key = old_key;
    let old_verdict = SignedVerdict::sign(
        &old_signer,
        &VerdictClaims {
            nonce: "n_rotation_2".into(),
            ..VerdictClaims::default()
        },
    );
    fixture
        .consume(&old_verdict)
        .expect("old key still trusted");

    fixture.revoke_key(old_key).unwrap();
    let late_old = SignedVerdict::sign(
        &old_signer,
        &VerdictClaims {
            nonce: "n_rotation_3".into(),
            ..VerdictClaims::default()
        },
    );
    assert_verifier_error(fixture.consume(&late_old), 1, VerifierError::KeyRevoked);
    fixture.assert_no_receipt(&late_old.nonce_hash());

    fixture.trusted_key = new_key;
    accepts(&mut fixture, &new_claims("n_rotation_4"), NOW);
}

// ---- Environment separation -------------------------------------------------

#[test]
fn production_and_sandbox_keys_do_not_cross_environments() {
    let mut fixture = Fixture::new();
    let production = fixture.config;
    let production_key = fixture.trusted_key;
    let public_key = fixture.signer.public_key().unwrap();

    fixture.switch_to_new_config(Environment::Sandbox);
    let failure = fixture
        .register_key(KID, public_key)
        .expect_err("production kid refused in sandbox");
    assert_eq!(
        failure.err,
        TransactionError::InstructionError(
            0,
            InstructionError::Custom(u32::from(VerifierError::InvalidKeyId))
        )
    );
    let sandbox_key = fixture.register_key("sandbox-test-01", public_key).unwrap();
    fixture.mandate = fixture.register_mandate(approved_mandate()).unwrap();
    let sandbox_mandate = fixture.mandate;

    // A production verdict presented to the sandbox config.
    fixture.trusted_key = sandbox_key;
    refuses(
        &mut fixture,
        &VerdictClaims::default(),
        NOW,
        VerifierError::KeyIdMismatch,
    );

    // A sandbox verdict presented to the production config.
    let sandbox_claims = VerdictClaims {
        kid: "sandbox-test-01".into(),
        ..VerdictClaims::default()
    };
    fixture.config = production;
    fixture.trusted_key = production_key;
    fixture.mandate = mandate_address(production, &approved_mandate().mandate_hash);
    refuses(
        &mut fixture,
        &sandbox_claims,
        NOW,
        VerifierError::KeyIdMismatch,
    );

    // The sandbox verdict in the sandbox config is accepted.
    fixture.config = config_address(Environment::Sandbox);
    fixture.trusted_key = sandbox_key;
    fixture.mandate = sandbox_mandate;
    accepts(&mut fixture, &sandbox_claims, NOW);
}
