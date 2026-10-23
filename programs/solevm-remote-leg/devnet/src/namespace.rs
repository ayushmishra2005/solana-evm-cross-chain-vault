//! Every address one deployment namespace uses, derived from public inputs.

use anchor_lang::prelude::Pubkey;
use anchor_lang::solana_program::bpf_loader_upgradeable;
use anchor_spl::token::spl_token;
use solana_signer::Signer;
use solevm_remote_leg::{
    CONSUMED_MESSAGE_SEED, CUSTODY_AUTHORITY_SEED, OutboundLane, REMOTE_CONFIG_SEED,
    REMOTE_POSITION_SEED, REPLAY_LANE_SEED, RISK_CONFIG_SEED, ReportRecord, STRATEGY_CONFIG_SEED,
    TRANSFER_RECORD_SEED, TransportLedger,
};
use solevm_test_strategy::{ADAPTER_AUTHORITY_SEED, ADAPTER_STATE_SEED};
use spl_associated_token_account_interface::address::get_associated_token_address_with_program_id;

use crate::keys::Keys;
use crate::messages::{CONTROL_LANE_ID, REPORT_LANE_ID};

#[derive(Clone, Debug)]
pub(crate) struct Namespace {
    pub label: String,
    pub deployment_id: [u8; 32],
    pub vault_id: [u8; 32],
    pub config: Pubkey,
    pub custody_authority: Pubkey,
    pub risk_config: Pubkey,
    pub control_lane: Pubkey,
    pub strategy_config: Pubkey,
    pub position: Pubkey,
    pub ledger: Pubkey,
    pub outbound_lane: Pubkey,
    pub adapter_state: Pubkey,
    pub adapter_authority: Pubkey,
    pub program_data: Pubkey,
    pub mint: Pubkey,
    pub custody: Pubkey,
    pub escrow: Pubkey,
    pub adapter_vault: Pubkey,
    pub transport_source: Pubkey,
}

impl Namespace {
    pub(crate) fn derive(label: &str, keys: &Keys) -> Self {
        let leg = solevm_remote_leg::ID;
        let adapter = solevm_test_strategy::ID;
        let deployment_id = tag(label, "deployment");
        let vault_id = tag(label, "vault");
        let pda =
            |seeds: &[&[u8]], program: &Pubkey| Pubkey::find_program_address(seeds, program).0;

        let config = pda(&[REMOTE_CONFIG_SEED, &deployment_id, &vault_id], &leg);
        let custody_authority = pda(&[CUSTODY_AUTHORITY_SEED, config.as_ref()], &leg);
        let adapter_state = pda(&[ADAPTER_STATE_SEED, config.as_ref()], &adapter);
        let adapter_authority = pda(&[ADAPTER_AUTHORITY_SEED, adapter_state.as_ref()], &adapter);
        let mint = keys.mint.pubkey();
        let ata = |owner: &Pubkey| {
            get_associated_token_address_with_program_id(owner, &mint, &spl_token::ID)
        };

        Self {
            label: label.to_owned(),
            deployment_id,
            vault_id,
            config,
            custody_authority,
            risk_config: pda(&[RISK_CONFIG_SEED, config.as_ref()], &leg),
            control_lane: pda(
                &[
                    REPLAY_LANE_SEED,
                    config.as_ref(),
                    &CONTROL_LANE_ID.to_le_bytes(),
                ],
                &leg,
            ),
            strategy_config: pda(&[STRATEGY_CONFIG_SEED, config.as_ref()], &leg),
            position: pda(&[REMOTE_POSITION_SEED, config.as_ref()], &leg),
            ledger: TransportLedger::address(&config).0,
            outbound_lane: OutboundLane::address(&config, REPORT_LANE_ID).0,
            adapter_state,
            adapter_authority,
            program_data: pda(&[leg.as_ref()], &bpf_loader_upgradeable::ID),
            mint,
            custody: ata(&custody_authority),
            escrow: ata(&keys.escrow_owner),
            adapter_vault: ata(&adapter_authority),
            transport_source: ata(&keys.transport.pubkey()),
        }
    }

    pub(crate) fn transfer_id(&self, name: &str) -> [u8; 32] {
        tag(&self.label, &format!("transfer/{name}"))
    }

    pub(crate) fn transfer_record(&self, transfer_id: &[u8; 32]) -> Pubkey {
        Pubkey::find_program_address(
            &[TRANSFER_RECORD_SEED, self.config.as_ref(), transfer_id],
            &solevm_remote_leg::ID,
        )
        .0
    }

    pub(crate) fn consumed(&self, sequence: u64) -> Pubkey {
        Pubkey::find_program_address(
            &[
                CONSUMED_MESSAGE_SEED,
                self.config.as_ref(),
                &CONTROL_LANE_ID.to_le_bytes(),
                &sequence.to_le_bytes(),
            ],
            &solevm_remote_leg::ID,
        )
        .0
    }

    pub(crate) fn report_record(&self, sequence: u64) -> Pubkey {
        ReportRecord::address(&self.config, REPORT_LANE_ID, sequence).0
    }

    /// Every named address, for the evidence and the address listing.
    pub(crate) fn roles(&self, keys: &Keys) -> Vec<(&'static str, Pubkey)> {
        vec![
            ("leg_program", solevm_remote_leg::ID),
            ("adapter_program", solevm_test_strategy::ID),
            ("leg_program_data", self.program_data),
            ("deployer_and_upgrade_authority", keys.deployer.pubkey()),
            ("administrator", keys.administrator.pubkey()),
            ("emergency_guardian", keys.guardian.pubkey()),
            ("transport_verifier", keys.verifier.pubkey()),
            (
                "asset_transport_and_mint_authority",
                keys.transport.pubkey(),
            ),
            ("attacker", keys.attacker.pubkey()),
            ("escrow_owner", keys.escrow_owner),
            ("test_mint", self.mint),
            ("remote_config", self.config),
            ("custody_authority", self.custody_authority),
            ("custody_token_account", self.custody),
            ("outbound_escrow", self.escrow),
            ("risk_config", self.risk_config),
            ("control_lane", self.control_lane),
            ("strategy_config", self.strategy_config),
            ("remote_position", self.position),
            ("transport_ledger", self.ledger),
            ("outbound_lane", self.outbound_lane),
            ("adapter_state", self.adapter_state),
            ("adapter_authority", self.adapter_authority),
            ("adapter_token_vault", self.adapter_vault),
            ("transport_source_token_account", self.transport_source),
        ]
    }
}

/// A 32-byte identifier bound to this run label.
fn tag(label: &str, name: &str) -> [u8; 32] {
    protocol_types::keccak256(format!("solevm/devnet/{label}/{name}").as_bytes())
}
