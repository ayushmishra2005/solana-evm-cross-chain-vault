//! Fixture for the SolEVM deployment-authorization profile. The CPI
//! forwarder stands in for the remote leg: it owns the remote config, and
//! the instruction after consume targets it. With accounts, the forwarder
//! relays into its first account, the remote config, which is not a program,
//! so a transaction that passes every verifier check fails at that
//! instruction instead. The remote-leg suite runs the real program.

use anchor_lang::prelude::Pubkey;
use anchor_lang::solana_program::instruction::{AccountMeta, Instruction};
use anchor_lang::{AccountDeserialize, InstructionData, ToAccountMetas};
use es256_authorization_verifier::deployment::{
    DEPLOY_ACCOUNT_COUNT, DEPLOY_RECEIPT_INDEX, DEPLOY_REMOTE_CONFIG_INDEX,
    DEPLOY_TO_STRATEGY_DISCRIMINATOR, DeploymentAuthorization, MAX_LIFETIME_SECONDS,
};
use es256_authorization_verifier::{
    DEPLOYMENT_AUTHORITY_SEED, DEPLOYMENT_NONCE_LEN, DEPLOYMENT_RECEIPT_SEED, DeploymentAuthority,
    InitializeDeploymentAuthorityParams, RISK_KEY_SEED, RegisterRiskKeyParams, RiskKey,
};
use es256_jws_adapter::{SyntheticSigner, normalize_signature, secp256r1_instruction};
use litesvm::types::FailedTransactionMetadata;
use solana_keypair::Keypair;
use solana_signer::Signer;

use super::*;

pub const NETWORK_DOMAIN: [u8; 32] = [0x5E; 32];
pub const DEPLOY_AMOUNT: u64 = 25_000_000;
pub const DEPLOY_NONCE: [u8; DEPLOYMENT_NONCE_LEN] = [0x4E; DEPLOYMENT_NONCE_LEN];
pub const DEPLOYMENT_CONTEXT: [u8; 32] = [0xC7; 32];

/// Account positions in `consume_deployment_authorization`.
pub mod deployment_account {
    pub const PAYER: usize = 0;
    pub const CONFIG: usize = 1;
    pub const RISK_KEY: usize = 2;
    pub const RECEIPT: usize = 3;
    pub const INSTRUCTIONS_SYSVAR: usize = 4;
}

/// One namespace's deployment authority and its registered risk key.
#[derive(Clone, Copy, Debug)]
pub struct Profile {
    pub remote_leg: Pubkey,
    pub remote_config: Pubkey,
    pub config: Pubkey,
    pub risk_key: Pubkey,
}

/// A signed approval and the precompile key it is presented with.
#[derive(Clone, Debug)]
pub struct SignedApproval {
    pub message: DeploymentAuthorization,
    pub signing_input: Vec<u8>,
    pub signature: [u8; 64],
    pub public_key: [u8; 33],
}

impl SignedApproval {
    pub fn sign(signer: &SyntheticSigner, message: DeploymentAuthorization) -> Self {
        Self::sign_bytes(signer, message, message.encode())
    }

    /// Signs arbitrary bytes; `message` only selects the accounts and the
    /// following instruction the transaction is built with.
    pub fn sign_bytes(
        signer: &SyntheticSigner,
        message: DeploymentAuthorization,
        signing_input: Vec<u8>,
    ) -> Self {
        let signature = normalize_signature(&signer.sign(&signing_input).unwrap()).unwrap();
        Self {
            message,
            signing_input,
            signature,
            public_key: signer.public_key().unwrap(),
        }
    }

    pub fn precompile(&self) -> Instruction {
        secp256r1_instruction(&self.public_key, &self.signature, &self.signing_input).unwrap()
    }
}

impl Fixture {
    /// A deployed verifier, the CPI forwarder standing in for the remote leg,
    /// one deployment authority and the fixture signer as its risk key.
    pub fn deployment_profile() -> (Self, Profile) {
        let mut fixture = Self::deployed();
        fixture.add_cpi_forwarder();
        let remote_config = fixture.add_remote_config(FORWARDER_ID);
        let deployer = fixture.deployer.insecure_clone();
        let config = fixture
            .initialize_deployment_authority(&deployer, FORWARDER_ID, remote_config)
            .expect("deployment authority initializes");
        let public_key = fixture.signer.public_key().unwrap();
        let risk_key = fixture
            .register_risk_key(config, public_key, KEY_NOT_BEFORE, KEY_NOT_AFTER)
            .expect("risk key registers");
        let profile = Profile {
            remote_leg: FORWARDER_ID,
            remote_config,
            config,
            risk_key,
        };
        (fixture, profile)
    }

    /// A non-empty account owned by `owner`, standing in for its config.
    pub fn add_remote_config(&mut self, owner: Pubkey) -> Pubkey {
        let address = Pubkey::new_unique();
        self.svm
            .set_account(
                address,
                solana_account::Account {
                    lamports: 1_000_000,
                    data: vec![0xAB; 64],
                    owner,
                    executable: false,
                    rent_epoch: 0,
                },
            )
            .unwrap();
        address
    }

    pub fn initialize_deployment_authority(
        &mut self,
        authority: &Keypair,
        remote_leg: Pubkey,
        remote_config: Pubkey,
    ) -> Result<Pubkey, Box<FailedTransactionMetadata>> {
        let params = InitializeDeploymentAuthorityParams {
            administrator: self.administrator.pubkey(),
            network_domain: NETWORK_DOMAIN,
        };
        let instruction = initialize_deployment_authority_instruction(
            authority.pubkey(),
            program_data_address(),
            remote_leg,
            remote_config,
            params,
        );
        self.send(&[instruction], &[authority])?;
        Ok(deployment_authority_address(remote_leg, remote_config))
    }

    pub fn register_risk_key(
        &mut self,
        config: Pubkey,
        public_key: [u8; 33],
        not_before: i64,
        not_after: i64,
    ) -> Result<Pubkey, Box<FailedTransactionMetadata>> {
        let params = RegisterRiskKeyParams {
            public_key,
            not_before,
            not_after,
        };
        let administrator = self.administrator.insecure_clone();
        let instruction = register_risk_key_instruction(administrator.pubkey(), config, params);
        self.send(&[instruction], &[&administrator])?;
        Ok(risk_key_address(config, &public_key))
    }

    pub fn revoke_risk_key(&mut self, config: Pubkey, risk_key: Pubkey) -> TxResult {
        let administrator = self.administrator.insecure_clone();
        let instruction = revoke_risk_key_instruction(administrator.pubkey(), config, risk_key);
        self.send(&[instruction], &[&administrator])
    }

    pub fn advance_policy_generation(&mut self, config: Pubkey) -> TxResult {
        let administrator = self.administrator.insecure_clone();
        let instruction = advance_policy_generation_instruction(administrator.pubkey(), config);
        self.send(&[instruction], &[&administrator])
    }

    pub fn deployment_authority(&self, config: Pubkey) -> DeploymentAuthority {
        let account = self.svm.get_account(&config).unwrap();
        DeploymentAuthority::try_deserialize(&mut account.data.as_slice()).unwrap()
    }

    pub fn risk_key_account(&self, risk_key: Pubkey) -> RiskKey {
        let account = self.svm.get_account(&risk_key).unwrap();
        RiskKey::try_deserialize(&mut account.data.as_slice()).unwrap()
    }

    /// The fixture signer's approval of `message`.
    pub fn approve(&self, message: DeploymentAuthorization) -> SignedApproval {
        SignedApproval::sign(&self.signer, message)
    }

    /// The approved shape: precompile, consume, then the deployment it
    /// names, relayed by the outsider.
    pub fn consume_deployment(&mut self, profile: &Profile, approval: &SignedApproval) -> TxResult {
        let instructions = [
            approval.precompile(),
            consume_deployment_instruction(self.outsider.pubkey(), profile, approval.message.nonce),
            deploy_instruction(profile, approval.message.nonce, approval.message.amount),
        ];
        let outsider = self.outsider.insecure_clone();
        self.send(&instructions, &[&outsider])
    }

    pub fn assert_no_deployment_receipt(&self, profile: &Profile, nonce: &[u8; 16]) {
        self.assert_no_account(&deployment_receipt_address(profile.config, nonce));
    }
}

/// An approval the profile accepts at `NOW`.
pub fn valid_message(profile: &Profile) -> DeploymentAuthorization {
    DeploymentAuthorization {
        authority: profile.config,
        network_domain: NETWORK_DOMAIN,
        policy_generation: 0,
        deployment_context: DEPLOYMENT_CONTEXT,
        amount: DEPLOY_AMOUNT,
        nonce: DEPLOY_NONCE,
        issued_at: NOW,
        expires_at: NOW + MAX_LIFETIME_SECONDS,
    }
}

pub fn deployment_authority_address(remote_leg: Pubkey, remote_config: Pubkey) -> Pubkey {
    Pubkey::find_program_address(
        &[
            DEPLOYMENT_AUTHORITY_SEED,
            remote_leg.as_ref(),
            remote_config.as_ref(),
        ],
        &es256_authorization_verifier::ID,
    )
    .0
}

pub fn risk_key_address(config: Pubkey, public_key: &[u8; 33]) -> Pubkey {
    Pubkey::find_program_address(
        &[
            RISK_KEY_SEED,
            config.as_ref(),
            &public_key[..1],
            &public_key[1..],
        ],
        &es256_authorization_verifier::ID,
    )
    .0
}

pub fn deployment_receipt_address(config: Pubkey, nonce: &[u8; 16]) -> Pubkey {
    Pubkey::find_program_address(
        &[DEPLOYMENT_RECEIPT_SEED, config.as_ref(), nonce],
        &es256_authorization_verifier::ID,
    )
    .0
}

pub fn initialize_deployment_authority_instruction(
    upgrade_authority: Pubkey,
    program_data: Pubkey,
    remote_leg: Pubkey,
    remote_config: Pubkey,
    params: InitializeDeploymentAuthorityParams,
) -> Instruction {
    Instruction {
        program_id: es256_authorization_verifier::ID,
        accounts: es256_authorization_verifier::accounts::InitializeDeploymentAuthority {
            upgrade_authority,
            program_data,
            remote_leg_program: remote_leg,
            remote_config,
            config: deployment_authority_address(remote_leg, remote_config),
            system_program: anchor_lang::system_program::ID,
        }
        .to_account_metas(None),
        data: es256_authorization_verifier::instruction::InitializeDeploymentAuthority { params }
            .data(),
    }
}

pub fn register_risk_key_instruction(
    administrator: Pubkey,
    config: Pubkey,
    params: RegisterRiskKeyParams,
) -> Instruction {
    Instruction {
        program_id: es256_authorization_verifier::ID,
        accounts: es256_authorization_verifier::accounts::RegisterRiskKey {
            administrator,
            config,
            risk_key: risk_key_address(config, &params.public_key),
            system_program: anchor_lang::system_program::ID,
        }
        .to_account_metas(None),
        data: es256_authorization_verifier::instruction::RegisterRiskKey { params }.data(),
    }
}

pub fn revoke_risk_key_instruction(
    administrator: Pubkey,
    config: Pubkey,
    risk_key: Pubkey,
) -> Instruction {
    Instruction {
        program_id: es256_authorization_verifier::ID,
        accounts: es256_authorization_verifier::accounts::RevokeRiskKey {
            administrator,
            config,
            risk_key,
        }
        .to_account_metas(None),
        data: es256_authorization_verifier::instruction::RevokeRiskKey {}.data(),
    }
}

pub fn advance_policy_generation_instruction(administrator: Pubkey, config: Pubkey) -> Instruction {
    Instruction {
        program_id: es256_authorization_verifier::ID,
        accounts: es256_authorization_verifier::accounts::AdvancePolicyGeneration {
            administrator,
            config,
        }
        .to_account_metas(None),
        data: es256_authorization_verifier::instruction::AdvancePolicyGeneration {}.data(),
    }
}

pub fn consume_deployment_instruction(
    payer: Pubkey,
    profile: &Profile,
    nonce: [u8; DEPLOYMENT_NONCE_LEN],
) -> Instruction {
    Instruction {
        program_id: es256_authorization_verifier::ID,
        accounts: es256_authorization_verifier::accounts::ConsumeDeploymentAuthorization {
            payer,
            config: profile.config,
            risk_key: profile.risk_key,
            receipt: deployment_receipt_address(profile.config, &nonce),
            instructions_sysvar: solana_sdk_ids::sysvar::instructions::ID,
            system_program: anchor_lang::system_program::ID,
        }
        .to_account_metas(None),
        data: es256_authorization_verifier::instruction::ConsumeDeploymentAuthorization { nonce }
            .data(),
    }
}

/// A `deploy_to_strategy` instruction in the shape the verifier pins: the
/// selector and amount, the remote config first and the receipt at its
/// index. The other accounts are placeholders the verifier never reads.
pub fn deploy_instruction(profile: &Profile, nonce: [u8; 16], amount: u64) -> Instruction {
    let mut accounts: Vec<AccountMeta> = (0..DEPLOY_ACCOUNT_COUNT)
        .map(|_| AccountMeta::new_readonly(Pubkey::new_unique(), false))
        .collect();
    accounts[DEPLOY_REMOTE_CONFIG_INDEX] = AccountMeta::new_readonly(profile.remote_config, false);
    accounts[DEPLOY_RECEIPT_INDEX] =
        AccountMeta::new_readonly(deployment_receipt_address(profile.config, &nonce), false);
    let mut data = DEPLOY_TO_STRATEGY_DISCRIMINATOR.to_vec();
    data.extend_from_slice(&amount.to_le_bytes());
    Instruction {
        program_id: profile.remote_leg,
        accounts,
        data,
    }
}

/// Drops the address a setup helper returns, keeping its failure.
pub fn outcome(result: Result<Pubkey, Box<FailedTransactionMetadata>>) -> TxResult {
    result.map(|_| litesvm::types::TransactionMetadata::default())
}

/// Every verifier check passed: consume at index 1 succeeded and the
/// stand-in deployment at index 2 failed, rolling the transaction back. The
/// forwarder relayed into the remote config, which is not a program.
pub fn assert_consume_passed(result: TxResult) {
    assert_eq!(
        instruction_error(result),
        (2, InstructionError::UnsupportedProgramId)
    );
}
