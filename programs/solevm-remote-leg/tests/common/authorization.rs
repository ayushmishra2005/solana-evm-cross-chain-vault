//! The risk approval every strategy deployment spends, and the verifier
//! state behind it.
//!
//! The approvals here come from a synthetic P-256 key with a fixed secret.
//! It stands in for a risk service only so the leg can be exercised; see
//! `tests/risk_approval.rs` for the deterministic test risk policy.

use anchor_lang::solana_program::bpf_loader_upgradeable;
use anchor_lang::solana_program::instruction::Instruction;
use anchor_lang::{AccountDeserialize, InstructionData, ToAccountMetas};
use anchor_spl::token::spl_token;
use es256_authorization_verifier::deployment::{
    DeploymentAuthorization, DeploymentContext, MAX_LIFETIME_SECONDS,
};
use es256_authorization_verifier::{
    DEPLOYMENT_AUTHORITY_SEED, DEPLOYMENT_RECEIPT_SEED, DeploymentAuthority, DeploymentReceipt,
    InitializeDeploymentAuthorityParams, RISK_KEY_SEED, RegisterRiskKeyParams,
};
use es256_jws_adapter::{SyntheticSigner, normalize_signature, secp256r1_instruction};
use solana_instruction::error::InstructionError;
use solana_keypair::Keypair;
use solana_signer::Signer;
use solana_transaction_error::TransactionError;
use solevm_remote_leg::{DEPLOY_AUTHORIZATION_SEED, DeployAuthorizationPolicy};

use super::{Fixture, Pubkey, StrategyAccounts, TxResult, built_program, program_data};

/// The cluster the test deployment authority is bound to.
pub const NETWORK_DOMAIN: [u8; 32] = [0x5E; 32];
/// Private scalar of the test risk key. Test material only.
pub const RISK_SECRET: [u8; 32] = [0x52; 32];
/// Position of `deploy_to_strategy` in an authorized transaction.
pub const DEPLOY_INDEX: u8 = 2;
/// Position of the verifier's consume in an authorized transaction.
pub const CONSUME_INDEX: u8 = 1;

/// Account positions in the verifier's consume instruction.
pub mod consume_account {
    pub const PAYER: usize = 0;
    pub const CONFIG: usize = 1;
    pub const RISK_KEY: usize = 2;
    pub const RECEIPT: usize = 3;
}

/// Account positions in `deploy_to_strategy`.
pub mod deploy_account {
    pub const REMOTE_CONFIG: usize = 0;
    pub const CUSTODY_TOKEN_ACCOUNT: usize = 4;
    pub const AUTHORIZATION_POLICY: usize = 12;
    pub const AUTHORIZATION_RECEIPT: usize = 13;
    pub const INSTRUCTIONS_SYSVAR: usize = 14;
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
        let signing_input = message.encode();
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

    pub fn nonce(&self) -> [u8; 16] {
        self.message.nonce
    }
}

/// Address the verifier's test-only CPI forwarder is loaded at.
pub const FORWARDER_ID: Pubkey = Pubkey::new_from_array([0xF0; 32]);

/// Reads the authorization verifier built by cargo build-sbf.
pub fn verifier_bytes() -> Vec<u8> {
    built_program("es256_authorization_verifier", "SBF_OUT_DIR", "deploy")
}

/// Every program binary the leg's tests load, by name.
pub fn loaded_programs() -> Vec<(&'static str, Vec<u8>)> {
    let mut programs: Vec<(&'static str, Vec<u8>)> = [
        "solevm_remote_leg",
        "solevm_test_strategy",
        "es256_authorization_verifier",
        "es256_cpi_forwarder",
    ]
    .into_iter()
    .map(|name| (name, built_program(name, "SBF_OUT_DIR", "deploy")))
    .collect();
    programs.push((
        "solevm_test_strategy (hostile)",
        super::hostile_adapter_bytes(),
    ));
    programs
}

/// `instruction` relayed by the CPI forwarder, which invokes its first
/// account with the rest.
pub fn forwarded(instruction: Instruction) -> Instruction {
    let mut accounts = vec![
        anchor_lang::solana_program::instruction::AccountMeta::new_readonly(
            instruction.program_id,
            false,
        ),
    ];
    accounts.extend(instruction.accounts);
    Instruction {
        program_id: FORWARDER_ID,
        accounts,
        data: instruction.data,
    }
}

pub fn verifier_program_data_address() -> Pubkey {
    Pubkey::find_program_address(
        &[es256_authorization_verifier::ID.as_ref()],
        &bpf_loader_upgradeable::ID,
    )
    .0
}

pub fn deployment_authority_address(remote_config: &Pubkey) -> Pubkey {
    Pubkey::find_program_address(
        &[
            DEPLOYMENT_AUTHORITY_SEED,
            solevm_remote_leg::ID.as_ref(),
            remote_config.as_ref(),
        ],
        &es256_authorization_verifier::ID,
    )
    .0
}

pub fn risk_key_address(authority: &Pubkey, public_key: &[u8; 33]) -> Pubkey {
    Pubkey::find_program_address(
        &[
            RISK_KEY_SEED,
            authority.as_ref(),
            &public_key[..1],
            &public_key[1..],
        ],
        &es256_authorization_verifier::ID,
    )
    .0
}

pub fn receipt_address(authority: &Pubkey, nonce: &[u8; 16]) -> Pubkey {
    Pubkey::find_program_address(
        &[DEPLOYMENT_RECEIPT_SEED, authority.as_ref(), nonce],
        &es256_authorization_verifier::ID,
    )
    .0
}

pub fn authorization_policy_address(remote_config: &Pubkey) -> Pubkey {
    Pubkey::find_program_address(
        &[DEPLOY_AUTHORIZATION_SEED, remote_config.as_ref()],
        &solevm_remote_leg::ID,
    )
    .0
}

/// Fails if anything before the deployment refused the transaction.
#[track_caller]
pub fn refused_by_deployment(result: TxResult) -> TxResult {
    if let Err(failure) = &result
        && let TransactionError::InstructionError(index, error) = &failure.err
    {
        assert_eq!(
            *index, DEPLOY_INDEX,
            "the approval was refused before the deployment ran: {error:?}"
        );
    }
    result
}

/// Compute units the runtime charged one top-level invocation of `program`,
/// from its `consumed N of M compute units` log line. Inner invocations of
/// other programs are included, as the runtime counts them.
#[track_caller]
pub fn consumed_units(logs: &[String], program: &Pubkey) -> u64 {
    let prefix = format!("Program {program} consumed ");
    let lines: Vec<u64> = logs
        .iter()
        .filter_map(|line| line.strip_prefix(&prefix))
        .filter_map(|rest| rest.split_once(' '))
        .map(|(units, _)| units.parse().expect("compute units parse"))
        .collect();
    assert_eq!(lines.len(), 1, "expected one invocation of {program}");
    lines[0]
}

/// The instruction index and error a transaction failed with.
#[track_caller]
pub fn instruction_failure(result: TxResult) -> (u8, InstructionError) {
    let failure = result.expect_err("transaction should have failed");
    match failure.err {
        TransactionError::InstructionError(index, error) => (index, error),
        other => panic!("expected an instruction error, got {other:?}"),
    }
}

impl Fixture {
    /// Loads the verifier, binds this namespace to a deployment authority
    /// with the test risk key, and adopts it, as an operator would.
    pub fn install_deploy_authorization(&mut self) {
        self.load_verifier();
        self.initialize_deployment_authority()
            .expect("deployment authority initializes");
        let public_key = self.risk_public_key();
        self.register_risk_key(public_key, 0, i64::MAX)
            .expect("risk key registers");
        self.adopt_deploy_authorization()
            .expect("deploy authorization is adopted");
    }

    /// The verifier with `risk_deployer` as upgrade authority, without any
    /// deployment authority.
    pub fn load_verifier(&mut self) {
        self.svm
            .add_program(es256_authorization_verifier::ID, &verifier_bytes())
            .expect("verifier loads");
        self.write_owned_account(
            verifier_program_data_address(),
            program_data(Some(self.risk_deployer.pubkey())),
            bpf_loader_upgradeable::ID,
        );
    }

    /// Loads the verifier's test-only CPI forwarder at [`FORWARDER_ID`].
    pub fn load_cpi_forwarder(&mut self) {
        self.svm
            .add_program(
                FORWARDER_ID,
                &built_program("es256_cpi_forwarder", "SBF_OUT_DIR", "deploy"),
            )
            .expect("forwarder loads");
    }

    pub fn risk_public_key(&self) -> [u8; 33] {
        self.risk_signer.public_key().unwrap()
    }

    pub fn initialize_deployment_authority(&mut self) -> TxResult {
        let params = InitializeDeploymentAuthorityParams {
            administrator: self.risk_administrator.pubkey(),
            network_domain: NETWORK_DOMAIN,
        };
        let deployer = self.risk_deployer.insecure_clone();
        let instruction = self.initialize_deployment_authority_instruction(
            deployer.pubkey(),
            self.config,
            params,
        );
        self.send(instruction, &[&deployer])
    }

    pub fn initialize_deployment_authority_instruction(
        &self,
        upgrade_authority: Pubkey,
        remote_config: Pubkey,
        params: InitializeDeploymentAuthorityParams,
    ) -> Instruction {
        Instruction {
            program_id: es256_authorization_verifier::ID,
            accounts: es256_authorization_verifier::accounts::InitializeDeploymentAuthority {
                upgrade_authority,
                program_data: verifier_program_data_address(),
                remote_leg_program: solevm_remote_leg::ID,
                remote_config,
                config: deployment_authority_address(&remote_config),
                system_program: anchor_lang::system_program::ID,
            }
            .to_account_metas(None),
            data: es256_authorization_verifier::instruction::InitializeDeploymentAuthority {
                params,
            }
            .data(),
        }
    }

    pub fn register_risk_key(
        &mut self,
        public_key: [u8; 33],
        not_before: i64,
        not_after: i64,
    ) -> TxResult {
        let authority = self.deployment_authority();
        let administrator = self.risk_administrator.insecure_clone();
        let instruction = Instruction {
            program_id: es256_authorization_verifier::ID,
            accounts: es256_authorization_verifier::accounts::RegisterRiskKey {
                administrator: administrator.pubkey(),
                config: authority,
                risk_key: risk_key_address(&authority, &public_key),
                system_program: anchor_lang::system_program::ID,
            }
            .to_account_metas(None),
            data: es256_authorization_verifier::instruction::RegisterRiskKey {
                params: RegisterRiskKeyParams {
                    public_key,
                    not_before,
                    not_after,
                },
            }
            .data(),
        };
        self.send(instruction, &[&administrator])
    }

    pub fn revoke_risk_key(&mut self, public_key: [u8; 33]) -> TxResult {
        let authority = self.deployment_authority();
        let administrator = self.risk_administrator.insecure_clone();
        let instruction = Instruction {
            program_id: es256_authorization_verifier::ID,
            accounts: es256_authorization_verifier::accounts::RevokeRiskKey {
                administrator: administrator.pubkey(),
                config: authority,
                risk_key: risk_key_address(&authority, &public_key),
            }
            .to_account_metas(None),
            data: es256_authorization_verifier::instruction::RevokeRiskKey {}.data(),
        };
        self.send(instruction, &[&administrator])
    }

    pub fn advance_policy_generation(&mut self) -> TxResult {
        let administrator = self.risk_administrator.insecure_clone();
        let instruction = Instruction {
            program_id: es256_authorization_verifier::ID,
            accounts: es256_authorization_verifier::accounts::AdvancePolicyGeneration {
                administrator: administrator.pubkey(),
                config: self.deployment_authority(),
            }
            .to_account_metas(None),
            data: es256_authorization_verifier::instruction::AdvancePolicyGeneration {}.data(),
        };
        self.send(instruction, &[&administrator])
    }

    pub fn adopt_deploy_authorization(&mut self) -> TxResult {
        let administrator = self.administrator.insecure_clone();
        let instruction = self.adopt_instruction(
            administrator.pubkey(),
            self.config,
            self.deployment_authority(),
        );
        self.send(instruction, &[&administrator])
    }

    pub fn adopt_instruction(
        &self,
        administrator: Pubkey,
        remote_config: Pubkey,
        authorization_config: Pubkey,
    ) -> Instruction {
        Instruction {
            program_id: solevm_remote_leg::ID,
            accounts: solevm_remote_leg::accounts::InitializeDeployAuthorization {
                administrator,
                remote_config,
                authorization_config,
                authorization_policy: authorization_policy_address(&remote_config),
                system_program: anchor_lang::system_program::ID,
            }
            .to_account_metas(None),
            data: solevm_remote_leg::instruction::InitializeDeployAuthorization {}.data(),
        }
    }

    pub fn deployment_authority(&self) -> Pubkey {
        deployment_authority_address(&self.config)
    }

    pub fn deployment_authority_state(&self) -> DeploymentAuthority {
        let account = self
            .svm
            .get_account(&self.deployment_authority())
            .expect("deployment authority exists");
        DeploymentAuthority::try_deserialize(&mut account.data.as_slice())
            .expect("deployment authority decodes")
    }

    pub fn authorization_policy(&self) -> Pubkey {
        authorization_policy_address(&self.config)
    }

    pub fn authorization_policy_state(&self) -> DeployAuthorizationPolicy {
        let account = self
            .svm
            .get_account(&self.authorization_policy())
            .expect("authorization policy exists");
        DeployAuthorizationPolicy::try_deserialize(&mut account.data.as_slice())
            .expect("authorization policy decodes")
    }

    pub fn receipt_key(&self, nonce: &[u8; 16]) -> Pubkey {
        receipt_address(&self.deployment_authority(), nonce)
    }

    pub fn receipt(&self, nonce: &[u8; 16]) -> Option<DeploymentReceipt> {
        let account = self.svm.get_account(&self.receipt_key(nonce))?;
        if account.data.is_empty() {
            return None;
        }
        Some(DeploymentReceipt::try_deserialize(&mut account.data.as_slice()).unwrap())
    }

    pub fn now(&self) -> i64 {
        self.svm
            .get_sysvar::<anchor_lang::solana_program::clock::Clock>()
            .unix_timestamp
    }

    /// The deployment a risk authority would be asked to approve, assembled
    /// from the fixture's own records and the accounts as they are now,
    /// without the program's helper.
    pub fn observed_context(&self) -> DeploymentContext {
        let config = self.config();
        let strategy = self.strategy_config();
        DeploymentContext {
            remote_leg_program: solevm_remote_leg::ID,
            remote_config: self.config,
            deployment_id: self.params.deployment_id,
            vault_id: self.params.vault_id,
            config_version: config.config_version,
            strategy_config: self.strategy(),
            adapter_program: solevm_test_strategy::ID,
            adapter_state: self.adapter_state,
            adapter_authority: self.adapter_authority,
            adapter_token_vault: self.adapter_vault,
            max_remote_principal: strategy.max_remote_principal,
            asset_mint: self.mint,
            token_program: spl_token::ID,
            custody_authority: self.custody_authority,
            custody_token_account: self.custody,
            deployed_principal: self.position().deployed_principal,
        }
    }

    /// A nonce no earlier approval of this fixture used.
    pub fn fresh_nonce(&mut self) -> [u8; 16] {
        let mut nonce = [0x4E; 16];
        nonce[..8].copy_from_slice(&self.next_nonce.to_le_bytes());
        self.next_nonce += 1;
        nonce
    }

    /// An approval of `amount` for the leg as it is now, valid for the
    /// longest lifetime the verifier accepts.
    pub fn approval_message(&mut self, amount: u64) -> DeploymentAuthorization {
        let now = self.now();
        DeploymentAuthorization {
            authority: self.deployment_authority(),
            network_domain: NETWORK_DOMAIN,
            policy_generation: self.deployment_authority_state().policy_generation,
            deployment_context: self.observed_context().commitment(),
            amount,
            nonce: self.fresh_nonce(),
            issued_at: now,
            expires_at: now + MAX_LIFETIME_SECONDS,
        }
    }

    pub fn sign_approval(&self, message: DeploymentAuthorization) -> SignedApproval {
        SignedApproval::sign(&self.risk_signer, message)
    }

    /// The test risk key's approval of `amount` for the leg as it is now.
    pub fn approve(&mut self, amount: u64) -> SignedApproval {
        let message = self.approval_message(amount);
        self.sign_approval(message)
    }

    pub fn consume_approval_instruction(&self, payer: Pubkey, nonce: [u8; 16]) -> Instruction {
        let authority = self.deployment_authority();
        Instruction {
            program_id: es256_authorization_verifier::ID,
            accounts: es256_authorization_verifier::accounts::ConsumeDeploymentAuthorization {
                payer,
                config: authority,
                risk_key: risk_key_address(&authority, &self.risk_public_key()),
                receipt: receipt_address(&authority, &nonce),
                instructions_sysvar: solana_instructions_sysvar::ID,
                system_program: anchor_lang::system_program::ID,
            }
            .to_account_metas(None),
            data: es256_authorization_verifier::instruction::ConsumeDeploymentAuthorization {
                nonce,
            }
            .data(),
        }
    }

    /// `[secp256r1, consume, deploy_to_strategy]`, relayed by the outsider.
    pub fn authorized_deploy(
        &self,
        approval: &SignedApproval,
        accounts: StrategyAccounts,
        amount: u64,
    ) -> Vec<Instruction> {
        let receipt = self.receipt_key(&approval.nonce());
        vec![
            approval.precompile(),
            self.consume_approval_instruction(self.outsider.pubkey(), approval.nonce()),
            self.deploy_instruction(accounts, amount, receipt),
        ]
    }

    /// Sends `instructions` paid and signed by the outsider, the permissionless relayer.
    pub fn relay(&mut self, instructions: &[Instruction]) -> TxResult {
        let relayer: Keypair = self.outsider.insecure_clone();
        self.send_instructions(instructions, &relayer, &[&relayer])
    }
}
