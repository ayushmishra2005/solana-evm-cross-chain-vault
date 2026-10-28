//! The V2 scenario on one fresh namespace: setup, a deployment authority with
//! a P-256 risk key, adoption, approved deployments, each refusal the brief
//! lists, a failed strategy deposit that rolls its approval back and a retry
//! of that same approval, a recall that needs no approval, and a frozen leg.
//!
//! Every step is checked against the state it leaves behind. An expected
//! failure is simulated first and submitted only when the node predicts the
//! exact failure; the record then shows what the cluster itself executed.

use std::collections::BTreeMap;
use std::fs::{self, DirBuilder};
use std::os::unix::fs::DirBuilderExt as _;
use std::path::PathBuf;
use std::str::FromStr;
use std::thread;
use std::time::{Duration, Instant};

use anchor_lang::error::{ERROR_CODE_OFFSET, ErrorCode as AnchorError};
use anchor_lang::prelude::Pubkey;
use anchor_lang::solana_program::bpf_loader_upgradeable;
use anchor_lang::solana_program::program_pack::Pack;
use anchor_lang::{InstructionData, ToAccountMetas, system_program};
use anchor_spl::token::spl_token;
use es256_authorization_verifier::deployment::{
    DeploymentAuthorization, DeploymentContext, MAX_LIFETIME_SECONDS,
};
use es256_authorization_verifier::{
    DeploymentAuthority, DeploymentReceipt, InitializeDeploymentAuthorityParams,
    RegisterRiskKeyParams, RiskKey as RiskKeyAccount, VerifierError,
};
use serde_json::{Value, json};
use solana_instruction::Instruction;
use solana_keypair::Keypair;
use solana_precompile_error::PrecompileError;
use solana_signer::Signer;
use solevm_remote_leg::{
    AllocateAccepted, AssetsDeployed, ControlStateParams, DeployAuthorizationPolicy,
    InitializeParams, OutboundLane, RecallAccepted, RemoteConfig, RemoteLegError, RemotePosition,
    ReplayLane, StrategyConfig, TransferKind, TransferRecord, TransferStatus,
};
use solevm_test_strategy::{AdapterError, AdapterState};
use spl_associated_token_account_interface::instruction::create_associated_token_account_idempotent;

use super::approval::{Approval, RiskKey};
use super::evidence::{
    Authorization, Evidence, ExpectedError, Feature, KIND, Measurement, Receipt, SCHEMA,
    Simulation, Step,
};
use super::keys::Keys;
use super::namespace::V2Namespace;
use super::report;
use super::settings::{Cluster, Settings};
use super::{
    ADAPTER_AUTHORITY, ADAPTER_SHA256, DISABLE_SBPF_V0_EXECUTION_FEATURE,
    PRECOMPILE_IN_SVM_FEATURE, SECP256R1_FEATURE, check_compiled_ids,
};
use crate::Res;
use crate::chain::{Chain, Landed, wire};
use crate::evidence::{Build, Check, Mint, Program, Source, hex, read_program, sha256_hex};
use crate::messages::{
    self, CONFIG_VERSION, CONTROL_LANE_ID, DESTINATION_CHAIN_ID, Domain, REPORT_LANE_ID,
    SOURCE_CHAIN_ID,
};
use crate::scenario::{code_of, decode, decode_at, event};
use crate::time::{now_utc, utc};

const MEMO_PROGRAM: Pubkey = Pubkey::from_str_const("MemoSq4gqABAXKb96qnH8TysNcWxMyWCqXgDLGmfcHr");
const CLOCK_SYSVAR: Pubkey = Pubkey::from_str_const("SysvarC1ock11111111111111111111111111111111");
const INSTRUCTIONS_SYSVAR: Pubkey =
    Pubkey::from_str_const("Sysvar1nstructions1111111111111111111111111");
const SECP256R1_PROGRAM: Pubkey =
    Pubkey::from_str_const("Secp256r1SigVerify1111111111111111111111111");
const FEATURE_PROGRAM: Pubkey =
    Pubkey::from_str_const("Feature111111111111111111111111111111111111");
const MINT_LABEL: &str = "SolEVM V2 devnet test token: 6 decimals, no value, not USDC";
const DECIMALS: u8 = 6;
/// Base units minted to the wallet that plays the bridge.
const MINTED: u64 = 5_000_000;
const MAX_REMOTE_PRINCIPAL: u64 = 10_000_000;
/// Lamports for the roles that pay rent for the records they create.
const ADMINISTRATOR_FUNDING: u64 = 50_000_000;
const VERIFIER_FUNDING: u64 = 30_000_000;
const RISK_ADMINISTRATOR_FUNDING: u64 = 10_000_000;
/// Enough for the attacker to pay the rent its impostor attempts ask for, so
/// they fail on the program's authority check, not on an empty wallet.
const ATTACKER_FUNDING: u64 = 10_000_000;
/// Role funding, the mint, four token accounts, the deployment authority,
/// two receipts, the recall records and every fee, with headroom.
const MIN_RUN_BALANCE: u64 = 200_000_000;
const FINALIZE_TIMEOUT: Duration = Duration::from_secs(300);
/// The largest legacy transaction a validator accepts.
const PACKET_LIMIT: usize = 1_232;
/// The risk key is valid from an hour before registration for a week.
const KEY_LEAD: i64 = 3_600;
const KEY_LIFETIME: i64 = 7 * 86_400;

// Scenario amounts in base units, with every derived value spelled out.
/// Allocation A arrives in full and is attributed.
const ALLOCATED: u64 = 1_000_000;
/// The first approved deployment.
const FIRST_DEPLOY: u64 = 400_000;
const ATTRIBUTED_AFTER_FIRST: u64 = 600_000;
/// The deployment whose strategy deposit fails, then succeeds on retry.
const RETRIED_DEPLOY: u64 = 200_000;
const ATTRIBUTED_AFTER_RETRY: u64 = 400_000;
const DEPLOYED_AFTER_RETRY: u64 = 600_000;
/// An approval of one amount presented with the next amount up.
const APPROVED_AMOUNT: u64 = 150_000;
const INFLATED_AMOUNT: u64 = 150_001;
/// An approval signed before the first deployment changed the leg.
const STALE_AMOUNT: u64 = 100_000;
const EXPIRED_AMOUNT: u64 = 100_000;
/// The recall takes the 400000 still in custody and 400000 from the strategy.
const RECALLED: u64 = 800_000;
const RECALLED_FROM_CUSTODY: u64 = 400_000;
const WITHDRAWN: u64 = 400_000;
const DEPLOYED_AFTER_RECALL: u64 = 200_000;
/// Allocation C, attributed and then refused deployment on a frozen leg.
const SECOND_ALLOCATION: u64 = 300_000;

pub(crate) fn print_addresses() -> Res<()> {
    check_compiled_ids()?;
    let settings = Settings::from_env()?;
    let keys = Keys::load(&settings.key_dir(), &settings.label)?;
    let ns = V2Namespace::derive(&settings.label, &keys);
    for (role, key) in keys.roles().into_iter().chain(ns.addresses()) {
        println!("{role:<42} {key}");
    }
    Ok(())
}

pub(crate) fn run() -> Res<()> {
    check_compiled_ids()?;
    let settings = Settings::from_env()?;
    settings.require_write_approval()?;
    let chain = Chain::new(&settings.rpc_url);
    let genesis = settings.cluster.confirm(&chain)?;
    require_runtime(&chain)?;
    let keys = Keys::load(&settings.key_dir(), &settings.label)?;
    let ns = V2Namespace::derive(&settings.label, &keys);
    if chain.account(&ns.base.config, "confirmed")?.is_some() {
        return Err(format!(
            "namespace '{}' already exists at {}; set a new SOLEVM_RUN_LABEL",
            settings.label, ns.base.config
        )
        .into());
    }
    // A local validator restarts from genesis, so only devnet records are
    // kept from being replaced, even by a run that stopped before S04.
    let record = settings.evidence_path();
    if settings.cluster == Cluster::Devnet && record.exists() {
        return Err(format!(
            "{} already exists; set a new SOLEVM_RUN_LABEL",
            record.display()
        )
        .into());
    }
    let balance = chain.balance(&keys.deployer.pubkey())?;
    if balance < MIN_RUN_BALANCE {
        return Err(format!(
            "deployer {} holds {balance} lamports; a run needs at least {MIN_RUN_BALANCE}",
            keys.deployer.pubkey()
        )
        .into());
    }

    let risk = RiskKey::generate()?;
    let mut driver = Driver::new(&chain, &keys, &settings, ns, risk, &genesis)?;
    let outcome = driver.execute();
    driver.summarize(outcome.is_ok());
    let path = driver.write()?;
    println!("evidence written to {}", path.display());
    outcome
}

/// The slot feature `id` activated at on this cluster, if it has.
fn activated_at(chain: &Chain, id: &str) -> Res<Option<u64>> {
    Ok(
        feature_account(chain, id)?.and_then(|data| match data.first() {
            Some(1) => data
                .get(1..9)
                .and_then(|bytes| <[u8; 8]>::try_from(bytes).ok().map(u64::from_le_bytes)),
            _ => None,
        }),
    )
}

/// The data of feature `id`'s account: absent if the gate was never
/// proposed, present but not yet active while it is pending.
fn feature_account(chain: &Chain, id: &str) -> Res<Option<Vec<u8>>> {
    let key = Pubkey::from_str(id).map_err(|error| format!("{id}: {error:?}"))?;
    Ok(chain
        .account(&key, "confirmed")?
        .filter(|account| account.owner == FEATURE_PROGRAM)
        .map(|account| account.data))
}

/// The gates the scenario depends on, read from the cluster before any key
/// is loaded, account read or record started, as `scripts/devnet-v2` does.
fn require_runtime(chain: &Chain) -> Res<()> {
    for (id, name) in [SECP256R1_FEATURE, PRECOMPILE_IN_SVM_FEATURE] {
        if activated_at(chain, id)?.is_none() {
            return Err(format!(
                "{} has not activated {name} ({id}); V2 needs it active",
                chain.url()
            )
            .into());
        }
    }
    let (id, name) = DISABLE_SBPF_V0_EXECUTION_FEATURE;
    if feature_account(chain, id)?.is_some() {
        return Err(format!(
            "{} has {name} ({id}) pending or active; the V2 builds and the reused adapter are SBPFv0",
            chain.url()
        )
        .into());
    }
    Ok(())
}

/// What a step must do on chain.
enum Expect {
    Success,
    /// The transaction fails in instruction `index` with exactly `error`.
    Fails {
        index: u8,
        error: Value,
        name: String,
    },
}

impl Expect {
    fn custom(index: u8, code: u32, name: String) -> Self {
        Self::Fails {
            index,
            error: json!({ "Custom": code }),
            name,
        }
    }

    fn leg(index: u8, error: RemoteLegError) -> Self {
        let code = code_of(error);
        Self::custom(index, code, format!("remote leg {error:?} ({code})"))
    }

    fn verifier(index: u8, error: VerifierError) -> Self {
        let code = (error as u32).saturating_add(ERROR_CODE_OFFSET);
        Self::custom(index, code, format!("verifier {error:?} ({code})"))
    }

    fn adapter(index: u8, error: AdapterError) -> Self {
        let code = (error as u32).saturating_add(ERROR_CODE_OFFSET);
        Self::custom(index, code, format!("test adapter {error:?} ({code})"))
    }

    fn anchor(index: u8, error: AnchorError) -> Self {
        let name = format!("Anchor {error:?}");
        let code = error as u32;
        Self::custom(index, code, format!("{name} ({code})"))
    }

    /// The system program refusing to create an account that exists.
    fn already_in_use(index: u8) -> Self {
        Self::custom(index, 0, String::from("system AccountAlreadyInUse (0)"))
    }

    fn precompile(index: u8, error: PrecompileError) -> Self {
        let name = format!("secp256r1 precompile {error:?}");
        let code = error as u32;
        Self::custom(index, code, format!("{name} ({code})"))
    }

    /// The transaction error the RPC reports for this expectation.
    fn transaction_error(&self) -> Option<Value> {
        match self {
            Self::Success => None,
            Self::Fails { index, error, .. } => Some(json!({ "InstructionError": [index, error] })),
        }
    }

    fn describe(&self) -> String {
        match self {
            Self::Success => String::from("success"),
            Self::Fails { index, name, .. } => format!("fails in instruction {index}: {name}"),
        }
    }
}

/// What one step sends and what it must prove.
struct Spec {
    id: &'static str,
    title: &'static str,
    program: Pubkey,
    instruction: &'static str,
    expect: Expect,
    accounts: Vec<(&'static str, Pubkey)>,
}

struct Outcome {
    landed: Landed,
    before: State,
    after: State,
}

/// Decoded state at one slot, plus the raw bytes behind it.
#[derive(Clone)]
struct State {
    slot: u64,
    raw: Vec<Option<Vec<u8>>>,
    payer_lamports: Option<u64>,
    config: Option<RemoteConfig>,
    lane: Option<ReplayLane>,
    outbound: Option<OutboundLane>,
    strategy: Option<StrategyConfig>,
    position: Option<RemotePosition>,
    adapter: Option<AdapterState>,
    authority: Option<DeploymentAuthority>,
    risk_key: Option<RiskKeyAccount>,
    policy: Option<DeployAuthorizationPolicy>,
    custody: Option<u64>,
    source: Option<u64>,
    escrow: Option<u64>,
    vault: Option<u64>,
    transfers: Vec<(String, Option<TransferRecord>)>,
    receipts: Vec<(String, Option<DeploymentReceipt>)>,
}

/// Positions of the tracked accounts in a state read.
const TRACKED: usize = 14;
const AUTHORIZATION_ACCOUNTS: [usize; 3] = [7, 8, 9];

impl State {
    fn config(&self) -> Res<&RemoteConfig> {
        self.config
            .as_ref()
            .ok_or_else(|| "remote config is missing".into())
    }
    fn lane(&self) -> Res<&ReplayLane> {
        self.lane
            .as_ref()
            .ok_or_else(|| "control lane is missing".into())
    }
    fn outbound(&self) -> Res<&OutboundLane> {
        self.outbound
            .as_ref()
            .ok_or_else(|| "outbound lane is missing".into())
    }
    fn strategy(&self) -> Res<&StrategyConfig> {
        self.strategy
            .as_ref()
            .ok_or_else(|| "strategy config is missing".into())
    }
    fn position(&self) -> Res<&RemotePosition> {
        self.position
            .as_ref()
            .ok_or_else(|| "remote position is missing".into())
    }
    fn adapter(&self) -> Res<&AdapterState> {
        self.adapter
            .as_ref()
            .ok_or_else(|| "adapter state is missing".into())
    }
    fn authority(&self) -> Res<&DeploymentAuthority> {
        self.authority
            .as_ref()
            .ok_or_else(|| "deployment authority is missing".into())
    }
    fn tokens(&self, value: Option<u64>, name: &str) -> Res<u64> {
        value.ok_or_else(|| format!("{name} token account is missing").into())
    }
    fn custody(&self) -> Res<u64> {
        self.tokens(self.custody, "custody")
    }
    fn vault(&self) -> Res<u64> {
        self.tokens(self.vault, "adapter vault")
    }
    fn escrow(&self) -> Res<u64> {
        self.tokens(self.escrow, "escrow")
    }
    fn transfer(&self, name: &str) -> Option<&TransferRecord> {
        self.transfers
            .iter()
            .find(|(known, _)| known == name)
            .and_then(|(_, record)| record.as_ref())
    }
    fn receipt(&self, name: &str) -> Option<&DeploymentReceipt> {
        self.receipts
            .iter()
            .find(|(known, _)| known == name)
            .and_then(|(_, receipt)| receipt.as_ref())
    }

    /// The deployment-authority, risk-key and policy bytes, and every receipt.
    fn authorization_raw(&self) -> Vec<Option<Vec<u8>>> {
        let receipts = self
            .raw
            .iter()
            .skip(TRACKED.saturating_add(self.transfers.len()))
            .cloned();
        AUTHORIZATION_ACCOUNTS
            .iter()
            .map(|index| self.raw.get(*index).cloned().flatten())
            .chain(receipts)
            .collect()
    }

    /// Custody is fully explained by the position's buckets.
    fn custody_explained(&self) -> Res<bool> {
        let position = self.position()?;
        let explained = position
            .attributed_principal
            .checked_add(position.recalled_custody)
            .and_then(|total| total.checked_add(position.unattributed_custody));
        Ok(explained == Some(self.custody()?))
    }

    fn json(&self) -> Value {
        json!({
            "context_slot": self.slot,
            "fee_payer_lamports": self.payer_lamports,
            "frozen": self.config.as_ref().map(|config| config.frozen),
            "control_lane_sequence": self.lane.as_ref().map(|lane| lane.highest_consumed_sequence),
            "outbound_next_sequence": self.outbound.as_ref().map(|lane| lane.next_sequence),
            "position": self.position.as_ref().map(|position| json!({
                "attributed_principal": position.attributed_principal,
                "deployed_principal": position.deployed_principal,
                "recalled_custody": position.recalled_custody,
                "unattributed_custody": position.unattributed_custody,
                "active_transfer_kind": format!("{:?}", position.active_transfer_kind),
                "active_transfer_status": format!("{:?}", position.active_transfer_status),
            })),
            "adapter": self.adapter.as_ref().map(|adapter| json!({
                "principal": adapter.principal,
                "deposits_paused": adapter.deposits_paused,
            })),
            "deployment_authority": self.authority.as_ref().map(|authority| json!({
                "policy_generation": authority.policy_generation,
                "network_domain": hex(&authority.network_domain),
            })),
            "risk_key_revoked": self.risk_key.as_ref().map(|key| key.revoked),
            "authorization_policy_adopted": self.policy.is_some(),
            "token_balances": {
                "custody": self.custody,
                "adapter_vault": self.vault,
                "outbound_escrow": self.escrow,
                "transport_source": self.source,
            },
            "transfers": self.transfers.iter().map(|(name, record)| (name.clone(), record.as_ref().map(|record| json!({
                "status": format!("{:?}", record.status),
                "kind": format!("{:?}", record.transfer_kind),
                "sequence": record.message_sequence,
                "authorized_amount": record.authorized_amount,
                "attributed_amount": record.attributed_amount,
            })))).collect::<BTreeMap<_, _>>(),
            "receipts": self.receipts.iter().map(|(name, receipt)| (name.clone(), receipt.as_ref().map(|receipt| json!({
                "amount": receipt.amount,
                "consumed_at": receipt.consumed_at,
            })))).collect::<BTreeMap<_, _>>(),
        })
    }
}

struct Driver<'a> {
    chain: &'a Chain,
    keys: &'a Keys,
    settings: &'a Settings,
    ns: V2Namespace,
    risk: RiskKey,
    domain: Domain,
    network_domain: [u8; 32],
    evidence: Evidence,
    /// Named transfers whose records every snapshot includes.
    transfers: Vec<(String, [u8; 32])>,
    /// Named approvals whose receipts every snapshot includes.
    receipts: Vec<(String, [u8; 16])>,
    /// Approvals that were spent, with the step that spent them.
    spent: Vec<(String, String, Approval)>,
    last_slot: u64,
}

impl<'a> Driver<'a> {
    fn new(
        chain: &'a Chain,
        keys: &'a Keys,
        settings: &'a Settings,
        ns: V2Namespace,
        risk: RiskKey,
        genesis: &str,
    ) -> Res<Self> {
        let network_domain = solana_hash::Hash::from_str(genesis)
            .map_err(|error| format!("genesis {genesis}: {error:?}"))?
            .to_bytes();
        let domain = Domain {
            source_application: protocol_types::keccak256(b"solevm/devnet-v2/simulated-evm-hub"),
            local_application: solevm_remote_leg::ID.to_bytes(),
            deployment_id: ns.base.deployment_id,
            vault_id: ns.base.vault_id,
        };
        let validation = match settings.cluster {
            Cluster::Localnet => {
                "localnet rehearsal on a throwaway validator; not devnet validation"
            }
            Cluster::Devnet => "public devnet run",
        };
        let evidence = Evidence {
            schema: SCHEMA,
            kind: KIND.to_owned(),
            cluster: settings.cluster.name().to_owned(),
            validation: validation.to_owned(),
            rpc_url: chain.url().to_owned(),
            genesis_hash: genesis.to_owned(),
            run_label: ns.base.label.clone(),
            started_at_utc: now_utc(),
            source: Source {
                commit: settings.source_commit.clone(),
                tree: settings.source_tree.clone(),
                dirty: settings.source_dirty,
            },
            build: Build {
                toolchain: String::from("cargo-build-sbf 4.4.0, platform-tools v1.58"),
                arch: String::from("SBPFv0"),
                rustflags: String::from("--cfg solevm_devnet"),
            },
            mint: Mint {
                address: ns.base.mint.to_string(),
                decimals: DECIMALS,
                mint_authority: keys.transport.pubkey().to_string(),
                label: MINT_LABEL.to_owned(),
            },
            authorities: keys
                .roles()
                .into_iter()
                .map(|(role, key)| (role.to_owned(), key.to_string()))
                .chain([(
                    String::from("adapter_upgrade_authority_v1_deployer"),
                    ADAPTER_AUTHORITY.to_string(),
                )])
                .collect(),
            namespace: ns
                .addresses()
                .into_iter()
                .map(|(role, key)| (role.to_owned(), key.to_string()))
                .collect(),
            ..Evidence::default()
        };
        let transfers = ["A", "R", "C"]
            .iter()
            .map(|name| ((*name).to_owned(), ns.base.transfer_id(name)))
            .collect();
        Ok(Self {
            chain,
            keys,
            settings,
            ns,
            risk,
            domain,
            network_domain,
            evidence,
            transfers,
            receipts: Vec::new(),
            spent: Vec::new(),
            last_slot: 0,
        })
    }

    fn execute(&mut self) -> Res<()> {
        self.record_node()?;
        self.verify_programs()?;
        self.record_balances(true)?;
        self.setup()?;
        self.authorization_setup()?;
        self.fund_position()?;
        self.adoption()?;
        self.deployments()?;
        self.failed_deposit_and_retry()?;
        self.recall()?;
        self.frozen()?;
        self.finalize()?;
        self.record_balances(false)
    }

    // The cluster and the programs

    fn record_node(&mut self) -> Res<()> {
        self.evidence.node_version = self.chain.version()?;
        if let Some(path) = &self.settings.feature_mirror {
            let mirror: Value = serde_json::from_str(
                &fs::read_to_string(path)
                    .map_err(|error| format!("{}: {error}", path.display()))?,
            )?;
            let deactivated = mirror["deactivated"]
                .as_array()
                .ok_or("the feature mirror lists no deactivated features")?;
            for id in deactivated {
                let id = id
                    .as_str()
                    .ok_or("a deactivated feature id is not a string")?;
                if let Some(slot) = activated_at(self.chain, id)? {
                    return Err(format!(
                        "feature {id} is inactive on devnet but active here since slot {slot}"
                    )
                    .into());
                }
            }
            println!(
                "{} features inactive on devnet are inactive here too",
                deactivated.len()
            );
            self.evidence.localnet_feature_mirror = Some(mirror);
        }
        for (id, name) in [SECP256R1_FEATURE, PRECOMPILE_IN_SVM_FEATURE] {
            let activated_at_slot = activated_at(self.chain, id)?;
            println!(
                "feature {id} ({name}): {}",
                activated_at_slot.map_or_else(
                    || String::from("NOT ACTIVE"),
                    |slot| format!("active since slot {slot}")
                )
            );
            self.evidence.features.push(Feature {
                id: id.to_owned(),
                name: name.to_owned(),
                activated_at_slot,
            });
        }
        Ok(())
    }

    fn verify_programs(&mut self) -> Res<()> {
        let deployer = self.keys.deployer.pubkey();
        let binaries = [
            (
                "solevm_remote_leg",
                solevm_remote_leg::ID,
                self.settings.leg_binary.clone(),
                deployer,
            ),
            (
                "es256_authorization_verifier",
                es256_authorization_verifier::ID,
                self.settings.verifier_binary.clone(),
                deployer,
            ),
            (
                "solevm_test_strategy",
                solevm_test_strategy::ID,
                self.settings.adapter_binary.clone(),
                ADAPTER_AUTHORITY,
            ),
        ];
        for (name, id, path, authority) in binaries {
            let local = fs::read(&path).map_err(|error| format!("{}: {error}", path.display()))?;
            // The record states SBPFv0; an ELF's e_flags carry its SBPF version.
            if local.get(48..52) != Some(&[0, 0, 0, 0][..]) {
                return Err(format!("{} is not an SBPFv0 build", path.display()).into());
            }
            let embeds = |key: &Pubkey| local.windows(32).any(|window| window == key.as_ref());
            if !embeds(&id) {
                return Err(format!(
                    "{} does not declare {id}; build it with scripts/devnet-v2/build.sh",
                    path.display()
                )
                .into());
            }
            if id == solevm_remote_leg::ID && !embeds(&es256_authorization_verifier::ID) {
                return Err(format!(
                    "{} does not embed the V2 verifier {}",
                    path.display(),
                    es256_authorization_verifier::ID
                )
                .into());
            }
            let local_sha256 = sha256_hex(&local);
            if id == solevm_test_strategy::ID && local_sha256 != ADAPTER_SHA256 {
                return Err(format!(
                    "{} is not the reused V1 adapter binary {ADAPTER_SHA256}",
                    path.display()
                )
                .into());
            }
            let deploy = if id == solevm_test_strategy::ID {
                None
            } else {
                self.deploy_record(name, &id)?
            };
            if deploy.is_none() {
                self.await_finalized_account(&id)?;
            }
            let found = read_program(self.chain, &id, local.len())?;
            let onchain_sha256 = sha256_hex(&found.code);
            let upgrade_authority = found
                .upgrade_authority
                .map(|key| key.to_string())
                .unwrap_or_default();
            println!(
                "{name} {id}: on-chain sha256 {onchain_sha256}, upgrade authority {upgrade_authority}"
            );
            self.evidence.programs.push(Program {
                name: name.to_owned(),
                program_id: id.to_string(),
                owner: found.owner.to_string(),
                executable: found.executable,
                program_data: found.program_data.to_string(),
                upgrade_authority,
                last_deploy_slot: found.last_deploy_slot,
                binary_len: u64::try_from(local.len())?,
                local_sha256: local_sha256.clone(),
                onchain_sha256: onchain_sha256.clone(),
                deploy_signature: deploy.as_ref().map(|landed| landed.signature.clone()),
                deploy_slot: deploy.as_ref().map(|landed| landed.slot),
                deploy_time_utc: deploy
                    .as_ref()
                    .and_then(|landed| landed.block_time)
                    .map(utc),
                explorer_url: report::address_url(&self.evidence, &id.to_string()),
            });
            if found.owner != bpf_loader_upgradeable::ID || !found.executable {
                return Err(format!("{name} is not an executable upgradeable program").into());
            }
            if found.upgrade_authority != Some(authority) {
                return Err(format!("{name} upgrade authority is not {authority}").into());
            }
            if local_sha256 != onchain_sha256 {
                return Err(format!("{name} on chain differs from {}", path.display()).into());
            }
        }
        Ok(())
    }

    fn await_finalized_account(&self, key: &Pubkey) -> Res<()> {
        let deadline = Instant::now()
            .checked_add(FINALIZE_TIMEOUT)
            .ok_or("deadline overflow")?;
        while self.chain.account(key, "finalized")?.is_none() {
            if Instant::now() >= deadline {
                return Err(format!("{key} is not visible at finalized commitment").into());
            }
            thread::sleep(Duration::from_secs(2));
        }
        Ok(())
    }

    /// The deploy transaction the V2 deploy script recorded, once finalized.
    fn deploy_record(&self, name: &str, id: &Pubkey) -> Res<Option<Landed>> {
        let path = self
            .settings
            .state_dir()
            .join(format!("deploy-{name}.json"));
        let Ok(text) = fs::read_to_string(&path) else {
            return Ok(None);
        };
        let record: Value = serde_json::from_str(&text)?;
        if record["programId"].as_str() != Some(id.to_string().as_str()) {
            return Ok(None);
        }
        let Some(signature) = record["signature"].as_str() else {
            return Ok(None);
        };
        self.chain
            .wait_all(&[signature.to_owned()], "finalized", FINALIZE_TIMEOUT)?;
        self.chain.transaction(signature, "finalized")
    }

    fn record_balances(&mut self, start: bool) -> Res<()> {
        let keys = self.keys;
        let roles = [
            ("deployer", keys.deployer.pubkey()),
            ("administrator", keys.administrator.pubkey()),
            ("transport_verifier", keys.verifier.pubkey()),
            ("risk_administrator", keys.risk_administrator.pubkey()),
            ("asset_transport", keys.transport.pubkey()),
            ("emergency_guardian", keys.guardian.pubkey()),
            ("attacker", keys.attacker.pubkey()),
        ];
        let mut balances = BTreeMap::new();
        for (role, key) in roles {
            balances.insert(role.to_owned(), self.chain.balance(&key)?);
        }
        if start {
            self.evidence.balances.before = balances;
        } else {
            self.evidence.balances.after = balances;
            let total = |map: &BTreeMap<String, u64>| {
                map.values()
                    .try_fold(0u64, |sum, value| sum.checked_add(*value))
            };
            self.evidence.balances.net_spent_lamports = total(&self.evidence.balances.before)
                .zip(total(&self.evidence.balances.after))
                .and_then(|(before, after)| before.checked_sub(after));
        }
        Ok(())
    }

    // Setup

    fn setup(&mut self) -> Res<()> {
        let keys = self.keys;
        let ns = self.ns.base.clone();
        let deployer = keys.deployer.pubkey();

        let fund = [
            (keys.administrator.pubkey(), ADMINISTRATOR_FUNDING),
            (keys.verifier.pubkey(), VERIFIER_FUNDING),
            (keys.risk_administrator.pubkey(), RISK_ADMINISTRATOR_FUNDING),
            (keys.attacker.pubkey(), ATTACKER_FUNDING),
        ]
        .map(|(to, lamports)| {
            solana_system_interface::instruction::transfer(&deployer, &to, lamports)
        });
        self.step(
            Spec {
                id: "S01",
                title: "Fund the administrator, transport verifier and risk administrator, who pay rent for what they create, and the attacker, so its attempts fail on authority rather than an empty wallet",
                program: system_program::ID,
                instruction: "system::transfer",
                expect: Expect::Success,
                accounts: vec![
                    ("administrator", keys.administrator.pubkey()),
                    ("transport_verifier", keys.verifier.pubkey()),
                    ("risk_administrator", keys.risk_administrator.pubkey()),
                    ("attacker", keys.attacker.pubkey()),
                ],
            },
            &fund,
            &[],
        )?;

        let mint_len = spl_token::state::Mint::LEN;
        let rent = self.chain.rent_exempt_minimum(mint_len)?;
        let create_mint = [
            solana_system_interface::instruction::create_account(
                &deployer,
                &ns.mint,
                rent,
                u64::try_from(mint_len)?,
                &spl_token::ID,
            ),
            spl_token::instruction::initialize_mint2(
                &spl_token::ID,
                &ns.mint,
                &keys.transport.pubkey(),
                None,
                DECIMALS,
            )
            .map_err(|error| format!("initialize_mint2: {error:?}"))?,
            Instruction {
                program_id: MEMO_PROGRAM,
                accounts: Vec::new(),
                data: MINT_LABEL.as_bytes().to_vec(),
            },
        ];
        self.step(
            Spec {
                id: "S02",
                title: "Create the labelled 6-decimal test mint (valueless, not Circle USDC)",
                program: spl_token::ID,
                instruction: "spl_token::initialize_mint2 + memo",
                expect: Expect::Success,
                accounts: vec![
                    ("test_mint", ns.mint),
                    ("mint_authority", keys.transport.pubkey()),
                ],
            },
            &create_mint,
            &[&keys.mint],
        )?;
        let mint = self.token_mint(&ns.mint)?;
        self.check(
            "mint has 6 decimals, the transport key as mint authority and no freeze authority",
            mint.decimals == DECIMALS
                && mint.mint_authority == Some(keys.transport.pubkey()).into()
                && mint.freeze_authority.is_none(),
        )?;

        let accounts = [
            create_associated_token_account_idempotent(
                &deployer,
                &ns.custody_authority,
                &ns.mint,
                &spl_token::ID,
            ),
            create_associated_token_account_idempotent(
                &deployer,
                &keys.escrow_owner,
                &ns.mint,
                &spl_token::ID,
            ),
            create_associated_token_account_idempotent(
                &deployer,
                &ns.adapter_authority,
                &ns.mint,
                &spl_token::ID,
            ),
            create_associated_token_account_idempotent(
                &deployer,
                &keys.transport.pubkey(),
                &ns.mint,
                &spl_token::ID,
            ),
            spl_token::instruction::mint_to_checked(
                &spl_token::ID,
                &ns.mint,
                &ns.transport_source,
                &keys.transport.pubkey(),
                &[],
                MINTED,
                DECIMALS,
            )
            .map_err(|error| format!("mint_to_checked: {error:?}"))?,
        ];
        self.step(
            Spec {
                id: "S03",
                title: "Create custody, escrow, adapter vault and transport token accounts; mint test tokens to the transport",
                program: spl_token::ID,
                instruction: "associated_token::create_idempotent + spl_token::mint_to_checked",
                expect: Expect::Success,
                accounts: vec![
                    ("custody_token_account", ns.custody),
                    ("outbound_escrow", ns.escrow),
                    ("adapter_token_vault", ns.adapter_vault),
                    ("transport_source_token_account", ns.transport_source),
                ],
            },
            &accounts,
            &[&keys.transport],
        )?;
        let custody = self.token_account(&ns.custody)?;
        self.check(
            "custody is owned by the custody-authority PDA, empty, with no delegate or close authority",
            custody.owner == ns.custody_authority
                && custody.amount == 0
                && custody.delegate.is_none()
                && custody.close_authority.is_none(),
        )?;

        let forged = self.initialize_ix(keys.attacker.pubkey());
        self.step(
            Spec {
                id: "N01",
                title: "Namespace claim co-signed by a key that is not the leg's upgrade authority",
                program: solevm_remote_leg::ID,
                instruction: "initialize_remote_leg",
                expect: Expect::leg(0, RemoteLegError::Unauthorized),
                accounts: vec![
                    ("claimed_upgrade_authority", keys.attacker.pubkey()),
                    ("remote_config", ns.config),
                ],
            },
            &[forged],
            &[&keys.administrator, &keys.attacker],
        )?;

        let initialize = self.initialize_ix(deployer);
        let out = self.step(
            Spec {
                id: "S04",
                title: "Claim the namespace: initialize_remote_leg co-signed by the upgrade authority",
                program: solevm_remote_leg::ID,
                instruction: "initialize_remote_leg",
                expect: Expect::Success,
                accounts: vec![
                    ("administrator", keys.administrator.pubkey()),
                    ("upgrade_authority", deployer),
                    ("remote_config", ns.config),
                ],
            },
            &[initialize],
            &[&keys.administrator],
        )?;
        let config = out.after.config()?;
        self.check(
            "config binds administrator, guardian, verifier, mint, custody, escrow and domain, unfrozen",
            config.administrator == keys.administrator.pubkey()
                && config.emergency_guardian == keys.guardian.pubkey()
                && config.transport_verifier == keys.verifier.pubkey()
                && config.asset_mint == ns.mint
                && config.custody_token_account == ns.custody
                && config.outbound_escrow == ns.escrow
                && config.deployment_id == ns.deployment_id
                && config.vault_id == ns.vault_id
                && config.config_version == CONFIG_VERSION
                && !config.frozen,
        )?;

        let control = self.leg(
            solevm_remote_leg::accounts::InitializeControlState {
                administrator: keys.administrator.pubkey(),
                remote_config: ns.config,
                risk_config: ns.risk_config,
                control_lane: ns.control_lane,
                system_program: system_program::ID,
            },
            solevm_remote_leg::instruction::InitializeControlState {
                params: self.control_params(),
            },
        );
        self.step(
            Spec {
                id: "S05",
                title: "Create the risk config and the strict-next inbound control lane",
                program: solevm_remote_leg::ID,
                instruction: "initialize_control_state",
                expect: Expect::Success,
                accounts: vec![
                    ("risk_config", ns.risk_config),
                    ("control_lane", ns.control_lane),
                ],
            },
            &[control],
            &[&keys.administrator],
        )?;

        let adapter = Instruction {
            program_id: solevm_test_strategy::ID,
            accounts: solevm_test_strategy::accounts::InitializeAdapter {
                payer: keys.administrator.pubkey(),
                adapter_state: ns.adapter_state,
                mint: ns.mint,
                adapter_token_vault: ns.adapter_vault,
                token_program: spl_token::ID,
                system_program: system_program::ID,
            }
            .to_account_metas(None),
            data: solevm_test_strategy::instruction::InitializeAdapter {
                params: solevm_test_strategy::AdapterParams {
                    remote_leg_program: solevm_remote_leg::ID,
                    remote_config: ns.config,
                    test_authority: keys.administrator.pubkey(),
                    max_liquid_principal: u64::MAX,
                    loss_bps: 0,
                },
            }
            .data(),
        };
        let out = self.step(
            Spec {
                id: "S06",
                title: "Initialize the reused test strategy adapter for this namespace",
                program: solevm_test_strategy::ID,
                instruction: "initialize_adapter",
                expect: Expect::Success,
                accounts: vec![
                    ("adapter_program", solevm_test_strategy::ID),
                    ("adapter_state", ns.adapter_state),
                    ("adapter_token_vault", ns.adapter_vault),
                ],
            },
            &[adapter],
            &[&keys.administrator],
        )?;
        let adapter = out.after.adapter()?;
        self.check(
            "adapter is bound to the V2 leg and this namespace, with deposits open",
            adapter.remote_leg_program == solevm_remote_leg::ID
                && adapter.remote_config == ns.config
                && adapter.test_authority == keys.administrator.pubkey()
                && !adapter.deposits_paused
                && adapter.principal == 0,
        )?;

        let strategy = self.leg(
            solevm_remote_leg::accounts::InitializeStrategyState {
                administrator: keys.administrator.pubkey(),
                remote_config: ns.config,
                risk_config: ns.risk_config,
                strategy_config: ns.strategy_config,
                remote_position: ns.position,
                adapter_program: solevm_test_strategy::ID,
                adapter_state: ns.adapter_state,
                adapter_authority: ns.adapter_authority,
                adapter_token_vault: ns.adapter_vault,
                asset_mint: ns.mint,
                token_program: spl_token::ID,
                system_program: system_program::ID,
            },
            solevm_remote_leg::instruction::InitializeStrategyState {
                max_remote_principal: MAX_REMOTE_PRINCIPAL,
            },
        );
        let out = self.step(
            Spec {
                id: "S07",
                title: "Create the strategy config with the pinned adapter, and the remote position",
                program: solevm_remote_leg::ID,
                instruction: "initialize_strategy_state",
                expect: Expect::Success,
                accounts: vec![
                    ("strategy_config", ns.strategy_config),
                    ("remote_position", ns.position),
                ],
            },
            &[strategy],
            &[&keys.administrator],
        )?;
        let strategy = out.after.strategy()?;
        self.check(
            "strategy binds the adapter program, state, authority and vault with the principal ceiling",
            strategy.adapter_program == solevm_test_strategy::ID
                && strategy.adapter_state == ns.adapter_state
                && strategy.adapter_authority == ns.adapter_authority
                && strategy.adapter_token_vault == ns.adapter_vault
                && strategy.max_remote_principal == MAX_REMOTE_PRINCIPAL,
        )?;
        self.evidence.strategy = json!({
            "strategy_config": ns.strategy_config.to_string(),
            "adapter_program": strategy.adapter_program.to_string(),
            "adapter_state": strategy.adapter_state.to_string(),
            "adapter_authority": strategy.adapter_authority.to_string(),
            "adapter_token_vault": strategy.adapter_token_vault.to_string(),
            "max_remote_principal": strategy.max_remote_principal,
        });

        let outbound = self.leg(
            solevm_remote_leg::accounts::InitializeOutboundState {
                administrator: keys.administrator.pubkey(),
                remote_config: ns.config,
                outbound_lane: ns.outbound_lane,
                system_program: system_program::ID,
            },
            solevm_remote_leg::instruction::InitializeOutboundState {},
        );
        self.step(
            Spec {
                id: "S08",
                title: "Create the outbound reporting lane",
                program: solevm_remote_leg::ID,
                instruction: "initialize_outbound_state",
                expect: Expect::Success,
                accounts: vec![("outbound_lane", ns.outbound_lane)],
            },
            &[outbound],
            &[&keys.administrator],
        )?;
        Ok(())
    }

    // The deployment authority and its risk key

    fn authorization_setup(&mut self) -> Res<()> {
        let keys = self.keys;
        let ns = self.ns.clone();
        let params = InitializeDeploymentAuthorityParams {
            administrator: keys.risk_administrator.pubkey(),
            network_domain: self.network_domain,
        };

        let forged = self.initialize_authority_ix(keys.attacker.pubkey(), params);
        self.step(
            Spec {
                id: "N02",
                title: "Deployment authority initialized by a key that is not the verifier's upgrade authority",
                program: es256_authorization_verifier::ID,
                instruction: "initialize_deployment_authority",
                expect: Expect::verifier(0, VerifierError::Unauthorized),
                accounts: vec![
                    ("claimed_upgrade_authority", keys.attacker.pubkey()),
                    ("deployment_authority", ns.deployment_authority),
                ],
            },
            &[forged],
            &[&keys.attacker],
        )?;

        let initialize = self.initialize_authority_ix(keys.deployer.pubkey(), params);
        let out = self.step(
            Spec {
                id: "A01",
                title: "Bind the namespace to a deployment authority under the V2 verifier, on this cluster's genesis",
                program: es256_authorization_verifier::ID,
                instruction: "initialize_deployment_authority",
                expect: Expect::Success,
                accounts: vec![
                    ("verifier_upgrade_authority", keys.deployer.pubkey()),
                    ("verifier_program_data", ns.verifier_program_data),
                    ("deployment_authority", ns.deployment_authority),
                ],
            },
            &[initialize],
            &[],
        )?;
        let authority = out.after.authority()?;
        self.check(
            "authority binds the V2 leg, this remote config and the risk administrator at policy generation 0",
            authority.version == 2
                && authority.administrator == keys.risk_administrator.pubkey()
                && authority.remote_leg_program == solevm_remote_leg::ID
                && authority.remote_config == ns.base.config
                && authority.policy_generation == 0,
        )?;
        self.check(
            format!(
                "network domain is the genesis hash {}",
                self.evidence.genesis_hash
            ),
            authority.network_domain == self.network_domain,
        )?;

        let public_key = self.risk.public_key();
        let now = self.cluster_time()?;
        let window = RegisterRiskKeyParams {
            public_key,
            not_before: now.checked_sub(KEY_LEAD).ok_or("time underflow")?,
            not_after: now.checked_add(KEY_LIFETIME).ok_or("time overflow")?,
        };
        let forged = self.register_ix(keys.attacker.pubkey(), window);
        self.step(
            Spec {
                id: "N03",
                title: "Risk key registered by a key that is not the deployment authority's administrator",
                program: es256_authorization_verifier::ID,
                instruction: "register_risk_key",
                expect: Expect::verifier(0, VerifierError::Unauthorized),
                accounts: vec![("claimed_administrator", keys.attacker.pubkey())],
            },
            &[forged],
            &[&keys.attacker],
        )?;

        let register = self.register_ix(keys.risk_administrator.pubkey(), window);
        let out = self.step(
            Spec {
                id: "A02",
                title: "Register the run's P-256 risk key (generated in memory, never stored)",
                program: es256_authorization_verifier::ID,
                instruction: "register_risk_key",
                expect: Expect::Success,
                accounts: vec![
                    ("risk_administrator", keys.risk_administrator.pubkey()),
                    ("risk_key", ns.risk_key(&public_key)),
                ],
            },
            &[register],
            &[&keys.risk_administrator],
        )?;
        let stored = out.after.risk_key.as_ref().ok_or("risk key is missing")?;
        self.detail(json!({
            "risk_public_key": hex(&public_key),
            "not_before": window.not_before,
            "not_after": window.not_after,
        }))?;
        self.check(
            "risk key stores the compressed public key, its window and the authority, unrevoked",
            stored.public_key == public_key
                && stored.config == ns.deployment_authority
                && stored.not_before == window.not_before
                && stored.not_after == window.not_after
                && !stored.revoked,
        )?;
        Ok(())
    }

    // Allocation A, which every deployment draws on

    fn fund_position(&mut self) -> Res<()> {
        self.allocate(
            "T01",
            "Authenticated Allocate A (sequence 1) delivered by the transport verifier",
            "A",
            ALLOCATED,
        )?;
        self.deposit(
            "T02",
            "Bridge stand-in delivers allocation A's test tokens to custody",
            ALLOCATED,
        )?;
        let ns = self.ns.base.clone();
        let reconcile = self.leg(
            solevm_remote_leg::accounts::ReconcileCustody {
                remote_config: ns.config,
                remote_position: ns.position,
                custody_token_account: ns.custody,
            },
            solevm_remote_leg::instruction::ReconcileCustody {},
        );
        let out = self.step(
            Spec {
                id: "T03",
                title: "Permissionless reconcile observes the arrival as unattributed custody",
                program: solevm_remote_leg::ID,
                instruction: "reconcile_custody",
                expect: Expect::Success,
                accounts: vec![("custody_token_account", ns.custody)],
            },
            &[reconcile],
            &[],
        )?;
        self.check(
            format!("unattributed custody is {ALLOCATED}"),
            out.after.position()?.unattributed_custody == ALLOCATED,
        )?;
        let out = self.attribute("T04", "Attribute the arrival to allocation A", "A")?;
        let record = out.after.transfer("A").ok_or("transfer A is missing")?;
        self.check(
            format!("A is complete and {ALLOCATED} is attributed principal, none deployed"),
            record.status == TransferStatus::Complete
                && out.after.position()?.attributed_principal == ALLOCATED
                && out.after.position()?.deployed_principal == 0,
        )?;
        Ok(())
    }

    // Adoption

    fn adoption(&mut self) -> Res<()> {
        let keys = self.keys;
        let ns = self.ns.clone();

        let early = self.approve("pre-adoption", FIRST_DEPLOY)?;
        let instructions = self.authorized_deploy(&early, early.precompile()?, FIRST_DEPLOY);
        self.authorized_step(
            Spec {
                id: "N04",
                title: "Fully approved deployment before the leg adopted the deployment authority",
                program: solevm_remote_leg::ID,
                instruction: "secp256r1 + consume_deployment_authorization + deploy_to_strategy",
                expect: Expect::anchor(2, AnchorError::AccountNotInitialized),
                accounts: vec![("authorization_policy", ns.authorization_policy)],
            },
            &instructions,
            "pre-adoption",
        )?;

        let forged = self.adopt_ix(keys.attacker.pubkey());
        self.step(
            Spec {
                id: "N05",
                title: "Adoption signed by a key that is not the leg administrator",
                program: solevm_remote_leg::ID,
                instruction: "initialize_deploy_authorization",
                expect: Expect::leg(0, RemoteLegError::Unauthorized),
                accounts: vec![("claimed_administrator", keys.attacker.pubkey())],
            },
            &[forged],
            &[&keys.attacker],
        )?;

        let adopt = self.adopt_ix(keys.administrator.pubkey());
        let out = self.step(
            Spec {
                id: "A03",
                title: "The leg administrator adopts the deployment authority, once and permanently",
                program: solevm_remote_leg::ID,
                instruction: "initialize_deploy_authorization",
                expect: Expect::Success,
                accounts: vec![
                    ("deployment_authority", ns.deployment_authority),
                    ("authorization_policy", ns.authorization_policy),
                ],
            },
            &[adopt],
            &[&keys.administrator],
        )?;
        let policy = out.after.policy.as_ref().ok_or("policy is missing")?;
        self.check(
            "policy names this namespace's deployment authority",
            policy.authorization_config == ns.deployment_authority,
        )?;

        let again = self.adopt_ix(keys.administrator.pubkey());
        self.step(
            Spec {
                id: "N06",
                title: "A second adoption cannot replace the first",
                program: solevm_remote_leg::ID,
                instruction: "initialize_deploy_authorization",
                expect: Expect::already_in_use(0),
                accounts: vec![("authorization_policy", ns.authorization_policy)],
            },
            &[again],
            &[&keys.administrator],
        )?;
        Ok(())
    }

    // Approved deployments and the refusals around them

    fn deployments(&mut self) -> Res<()> {
        let ns = self.ns.clone();

        let never = ns.receipt(&ns.nonce("never-approved"));
        let bare = self.deploy_ix(FIRST_DEPLOY, never);
        self.step(
            Spec {
                id: "N07",
                title: "deploy_to_strategy with no risk approval in the transaction",
                program: solevm_remote_leg::ID,
                instruction: "deploy_to_strategy",
                expect: Expect::anchor(0, AnchorError::AccountNotInitialized),
                accounts: vec![("authorization_receipt", never)],
            },
            &[bare],
            &[],
        )?;

        let stale = self.approve("stale", STALE_AMOUNT)?;

        let first = self.approve("D01", FIRST_DEPLOY)?;
        let instructions = self.authorized_deploy(&first, first.precompile()?, FIRST_DEPLOY);
        let out = self.authorized_step(
            Spec {
                id: "D01",
                title: "Deploy 400000 under an ES256 risk approval: secp256r1, consume, deploy in one legacy transaction",
                program: solevm_remote_leg::ID,
                instruction: "secp256r1 + consume_deployment_authorization + deploy_to_strategy",
                expect: Expect::Success,
                accounts: vec![("authorization_receipt", ns.receipt(&first.nonce()))],
            },
            &instructions,
            "D01",
        )?;
        self.check_deployed(
            &out,
            &first,
            FIRST_DEPLOY,
            ATTRIBUTED_AFTER_FIRST,
            FIRST_DEPLOY,
        )?;

        let replay = self.authorized_deploy(&first, first.precompile()?, FIRST_DEPLOY);
        self.authorized_step(
            Spec {
                id: "N08",
                title: "Replay of D01's spent approval in a new transaction",
                program: es256_authorization_verifier::ID,
                instruction: "secp256r1 + consume_deployment_authorization + deploy_to_strategy",
                expect: Expect::already_in_use(1),
                accounts: vec![("authorization_receipt", ns.receipt(&first.nonce()))],
            },
            &replay,
            "D01",
        )?;

        let reuse = self.deploy_ix(FIRST_DEPLOY, ns.receipt(&first.nonce()));
        self.step(
            Spec {
                id: "N09",
                title: "deploy_to_strategy alone, pointing at D01's spent receipt",
                program: solevm_remote_leg::ID,
                instruction: "deploy_to_strategy",
                expect: Expect::leg(0, RemoteLegError::DeploymentAuthorizationRequired),
                accounts: vec![("authorization_receipt", ns.receipt(&first.nonce()))],
            },
            &[reuse],
            &[],
        )?;

        let instructions = self.authorized_deploy(&stale, stale.precompile()?, STALE_AMOUNT);
        self.authorized_step(
            Spec {
                id: "N10",
                title: "Approval signed before D01 changed the deployed principal",
                program: solevm_remote_leg::ID,
                instruction: "secp256r1 + consume_deployment_authorization + deploy_to_strategy",
                expect: Expect::leg(2, RemoteLegError::AuthorizationContextMismatch),
                accounts: vec![("authorization_receipt", ns.receipt(&stale.nonce()))],
            },
            &instructions,
            "stale",
        )?;

        let approved = self.approve("wrong-amount", APPROVED_AMOUNT)?;
        let instructions =
            self.authorized_deploy(&approved, approved.precompile()?, INFLATED_AMOUNT);
        self.authorized_step(
            Spec {
                id: "N11",
                title: "Approval of 150000 presented with a deployment of 150001",
                program: es256_authorization_verifier::ID,
                instruction: "secp256r1 + consume_deployment_authorization + deploy_to_strategy",
                expect: Expect::verifier(1, VerifierError::InvalidDeploymentInstruction),
                accounts: vec![("authorization_receipt", ns.receipt(&approved.nonce()))],
            },
            &instructions,
            "wrong-amount",
        )?;

        let signed = self.approve("tampered", APPROVED_AMOUNT)?;
        let tampered = signed.with_amount_altered(INFLATED_AMOUNT)?;
        let instructions = self.authorized_deploy(&signed, tampered, INFLATED_AMOUNT);
        self.authorized_step(
            Spec {
                id: "N12",
                title: "Signed message altered from 150000 to 150001 after signing",
                program: SECP256R1_PROGRAM,
                instruction: "secp256r1 + consume_deployment_authorization + deploy_to_strategy",
                expect: Expect::precompile(0, PrecompileError::InvalidSignature),
                accounts: vec![("authorization_receipt", ns.receipt(&signed.nonce()))],
            },
            &instructions,
            "tampered",
        )?;

        let now = self.cluster_time()?;
        let issued_at = now
            .checked_sub(MAX_LIFETIME_SECONDS)
            .and_then(|time| time.checked_sub(100))
            .ok_or("time underflow")?;
        let expired = self.approve_at(
            "expired",
            EXPIRED_AMOUNT,
            issued_at,
            issued_at
                .checked_add(MAX_LIFETIME_SECONDS)
                .ok_or("time overflow")?,
        )?;
        let instructions = self.authorized_deploy(&expired, expired.precompile()?, EXPIRED_AMOUNT);
        self.authorized_step(
            Spec {
                id: "N13",
                title: "Approval that expired 100 seconds before submission",
                program: es256_authorization_verifier::ID,
                instruction: "secp256r1 + consume_deployment_authorization + deploy_to_strategy",
                expect: Expect::verifier(1, VerifierError::Expired),
                accounts: vec![("authorization_receipt", ns.receipt(&expired.nonce()))],
            },
            &instructions,
            "expired",
        )?;
        Ok(())
    }

    // A strategy deposit that fails, and a retry of the same approval

    fn failed_deposit_and_retry(&mut self) -> Res<()> {
        let ns = self.ns.clone();

        let pause = self.conditions_ix(true);
        let out = self.step(
            Spec {
                id: "C01",
                title: "Test authority pauses the adapter's deposits",
                program: solevm_test_strategy::ID,
                instruction: "configure_adapter_test_conditions",
                expect: Expect::Success,
                accounts: vec![("adapter_state", ns.base.adapter_state)],
            },
            &[pause],
            &[&self.keys.administrator],
        )?;
        self.check(
            "adapter deposits are paused",
            out.after.adapter()?.deposits_paused,
        )?;

        let retried = self.approve("retried", RETRIED_DEPLOY)?;
        let instructions = self.authorized_deploy(&retried, retried.precompile()?, RETRIED_DEPLOY);
        self.authorized_step(
            Spec {
                id: "N14",
                title: "Approved deployment of 200000 whose strategy deposit fails: the receipt rolls back",
                program: solevm_test_strategy::ID,
                instruction: "secp256r1 + consume_deployment_authorization + deploy_to_strategy",
                expect: Expect::adapter(2, AdapterError::DepositsPaused),
                accounts: vec![("authorization_receipt", ns.receipt(&retried.nonce()))],
            },
            &instructions,
            "retried",
        )?;

        let resume = self.conditions_ix(false);
        let out = self.step(
            Spec {
                id: "C02",
                title: "Test authority resumes the adapter's deposits",
                program: solevm_test_strategy::ID,
                instruction: "configure_adapter_test_conditions",
                expect: Expect::Success,
                accounts: vec![("adapter_state", ns.base.adapter_state)],
            },
            &[resume],
            &[&self.keys.administrator],
        )?;
        self.check(
            "adapter deposits are open",
            !out.after.adapter()?.deposits_paused,
        )?;

        let instructions = self.authorized_deploy(&retried, retried.precompile()?, RETRIED_DEPLOY);
        let out = self.authorized_step(
            Spec {
                id: "D02",
                title: "Retry of the same signed approval in a new transaction succeeds",
                program: solevm_remote_leg::ID,
                instruction: "secp256r1 + consume_deployment_authorization + deploy_to_strategy",
                expect: Expect::Success,
                accounts: vec![("authorization_receipt", ns.receipt(&retried.nonce()))],
            },
            &instructions,
            "retried",
        )?;
        self.check_deployed(
            &out,
            &retried,
            RETRIED_DEPLOY,
            ATTRIBUTED_AFTER_RETRY,
            DEPLOYED_AFTER_RETRY,
        )?;

        let replay = self.authorized_deploy(&retried, retried.precompile()?, RETRIED_DEPLOY);
        self.authorized_step(
            Spec {
                id: "N15",
                title: "Replay of the retried approval after it was spent",
                program: es256_authorization_verifier::ID,
                instruction: "secp256r1 + consume_deployment_authorization + deploy_to_strategy",
                expect: Expect::already_in_use(1),
                accounts: vec![("authorization_receipt", ns.receipt(&retried.nonce()))],
            },
            &replay,
            "retried",
        )?;
        Ok(())
    }

    // Recall, withdrawal and return, with no risk approval

    fn recall(&mut self) -> Res<()> {
        let keys = self.keys;
        let ns = self.ns.clone();
        let base = ns.base.clone();
        let untouched = self.state()?.authorization_raw();
        let transfer_id = base.transfer_id("R");
        let record = base.transfer_record(&transfer_id);

        let lane = self.state()?.lane()?.clone();
        let sequence = lane
            .highest_consumed_sequence
            .checked_add(1)
            .ok_or("sequence overflow")?;
        let now = u64::try_from(self.cluster_time()?)?;
        let message = messages::recall(
            &self.domain,
            sequence,
            lane.message_commitment,
            now,
            transfer_id,
            RECALLED,
        )?;
        let instruction = self.leg(
            solevm_remote_leg::accounts::ProcessRecall {
                transport_verifier: keys.verifier.pubkey(),
                remote_config: base.config,
                risk_config: base.risk_config,
                remote_position: base.position,
                control_lane: base.control_lane,
                transfer_record: record,
                consumed_message: base.consumed(sequence),
                system_program: system_program::ID,
                emergency_guardian: None,
            },
            solevm_remote_leg::instruction::ProcessRecall {
                message_bytes: message.bytes.clone(),
            },
        );
        let out = self.step(
            Spec {
                id: "R01",
                title: "Authenticated Recall of 800000: reserves the 400000 still in custody",
                program: solevm_remote_leg::ID,
                instruction: "process_recall",
                expect: Expect::Success,
                accounts: vec![
                    ("transport_verifier", keys.verifier.pubkey()),
                    ("transfer_record", record),
                ],
            },
            &[instruction],
            &[&keys.verifier],
        )?;
        let accepted: RecallAccepted = event(&out.landed.logs).ok_or("no RecallAccepted event")?;
        self.detail(json!({
            "sequence": sequence,
            "message_id": hex(&message.id),
            "requested_amount": accepted.requested_amount,
        }))?;
        let position = out.after.position()?;
        self.check(
            format!("{RECALLED_FROM_CUSTODY} moved from attributed principal to recalled custody"),
            accepted.message_id == message.id
                && accepted.requested_amount == RECALLED
                && position.recalled_custody == RECALLED_FROM_CUSTODY
                && position.attributed_principal == 0
                && position.deployed_principal == DEPLOYED_AFTER_RETRY,
        )?;
        self.check_no_authorization(&out.landed)?;

        let withdraw = self.leg(
            solevm_remote_leg::accounts::WithdrawForRecall {
                remote_config: base.config,
                strategy_config: base.strategy_config,
                remote_position: base.position,
                transfer_record: record,
                custody_authority: base.custody_authority,
                custody_token_account: base.custody,
                adapter_caller: ns.adapter_caller,
                adapter_program: solevm_test_strategy::ID,
                adapter_state: base.adapter_state,
                adapter_authority: base.adapter_authority,
                adapter_token_vault: base.adapter_vault,
                asset_mint: base.mint,
                token_program: spl_token::ID,
            },
            solevm_remote_leg::instruction::WithdrawForRecall {
                maximum_principal: WITHDRAWN,
            },
        );
        let out = self.step(
            Spec {
                id: "R02",
                title: "Permissionless withdrawal of the other 400000 from the strategy",
                program: solevm_remote_leg::ID,
                instruction: "withdraw_for_recall",
                expect: Expect::Success,
                accounts: vec![
                    ("adapter_token_vault", base.adapter_vault),
                    ("custody_token_account", base.custody),
                ],
            },
            &[withdraw],
            &[],
        )?;
        let position = out.after.position()?;
        self.check(
            format!(
                "deployed principal fell to {DEPLOYED_AFTER_RECALL}; recalled custody is {RECALLED}"
            ),
            position.deployed_principal == DEPLOYED_AFTER_RECALL
                && position.recalled_custody == RECALLED
                && out.before.vault()?.checked_sub(WITHDRAWN) == Some(out.after.vault()?),
        )?;
        self.check_no_authorization(&out.landed)?;

        let send = self.leg(
            solevm_remote_leg::accounts::SendRecall {
                remote_config: base.config,
                remote_position: base.position,
                transfer_record: record,
                custody_authority: base.custody_authority,
                custody_token_account: base.custody,
                outbound_escrow: base.escrow,
                token_program: spl_token::ID,
            },
            solevm_remote_leg::instruction::SendRecall {
                maximum_amount: RECALLED,
            },
        );
        let out = self.step(
            Spec {
                id: "R03",
                title: "Permissionless return of the 800000 to the outbound escrow",
                program: solevm_remote_leg::ID,
                instruction: "send_recall",
                expect: Expect::Success,
                accounts: vec![("outbound_escrow", base.escrow)],
            },
            &[send],
            &[],
        )?;
        self.check(
            format!("escrow rose by {RECALLED} and no recalled custody is left"),
            out.before.escrow()?.checked_add(RECALLED) == Some(out.after.escrow()?)
                && out.after.position()?.recalled_custody == 0
                && out.after.custody_explained()?,
        )?;
        self.check_no_authorization(&out.landed)?;

        let report_sequence = self.state()?.outbound()?.next_sequence;
        let publish = self.leg(
            solevm_remote_leg::accounts::PublishRecallSent {
                payer: keys.deployer.pubkey(),
                remote_config: base.config,
                risk_config: base.risk_config,
                transfer_record: record,
                outbound_lane: base.outbound_lane,
                report_record: base.report_record(report_sequence),
                recall_sent_marker: ns.recall_marker(&transfer_id),
                system_program: system_program::ID,
            },
            solevm_remote_leg::instruction::PublishRecallSent {},
        );
        let out = self.step(
            Spec {
                id: "R04",
                title: "Publish RecallSent on the outbound lane",
                program: solevm_remote_leg::ID,
                instruction: "publish_recall_sent",
                expect: Expect::Success,
                accounts: vec![
                    ("report_record", base.report_record(report_sequence)),
                    ("recall_sent_marker", ns.recall_marker(&transfer_id)),
                ],
            },
            &[publish],
            &[],
        )?;
        let record = out.after.transfer("R").ok_or("recall record is missing")?;
        self.check(
            "the recall is complete and the outbound lane advanced by one",
            record.status == TransferStatus::Complete
                && record.transfer_kind == TransferKind::Recall
                && Some(out.after.outbound()?.next_sequence) == report_sequence.checked_add(1),
        )?;
        self.check_no_authorization(&out.landed)?;
        self.check(
            "deployment authority, risk key, policy and every receipt are byte-for-byte as before the recall",
            out.after.authorization_raw() == untouched,
        )?;
        Ok(())
    }

    // A frozen leg refuses even an approved deployment

    fn frozen(&mut self) -> Res<()> {
        let keys = self.keys;
        let ns = self.ns.clone();
        self.allocate(
            "T05",
            "Authenticated Allocate C after the recall completed",
            "C",
            SECOND_ALLOCATION,
        )?;
        self.deposit(
            "T06",
            "Allocation C's test tokens arrive in custody",
            SECOND_ALLOCATION,
        )?;
        let out = self.attribute("T07", "Attribute the arrival to allocation C", "C")?;
        self.check(
            format!("{SECOND_ALLOCATION} is attributed and could be deployed"),
            out.after.position()?.attributed_principal == SECOND_ALLOCATION,
        )?;

        let freeze = self.leg(
            solevm_remote_leg::accounts::FreezeRemoteLeg {
                authority: keys.guardian.pubkey(),
                remote_config: ns.base.config,
            },
            solevm_remote_leg::instruction::FreezeRemoteLeg {},
        );
        let out = self.step(
            Spec {
                id: "F01",
                title: "Emergency guardian freezes the leg",
                program: solevm_remote_leg::ID,
                instruction: "freeze_remote_leg",
                expect: Expect::Success,
                accounts: vec![("emergency_guardian", keys.guardian.pubkey())],
            },
            &[freeze],
            &[&keys.guardian],
        )?;
        self.check("the leg is frozen", out.after.config()?.frozen)?;

        let approval = self.approve("frozen", SECOND_ALLOCATION)?;
        let instructions =
            self.authorized_deploy(&approval, approval.precompile()?, SECOND_ALLOCATION);
        self.authorized_step(
            Spec {
                id: "N16",
                title: "Valid, unexpired approval of 300000 on the frozen leg",
                program: solevm_remote_leg::ID,
                instruction: "secp256r1 + consume_deployment_authorization + deploy_to_strategy",
                expect: Expect::leg(2, RemoteLegError::Frozen),
                accounts: vec![("authorization_receipt", ns.receipt(&approval.nonce()))],
            },
            &instructions,
            "frozen",
        )?;
        Ok(())
    }

    // Finality and the record

    fn finalize(&mut self) -> Res<()> {
        let signatures: Vec<String> = self
            .evidence
            .steps
            .iter()
            .map(|step| step.signature.clone())
            .collect();
        println!("waiting for {} transactions to finalize", signatures.len());
        self.chain
            .wait_all(&signatures, "finalized", FINALIZE_TIMEOUT)?;
        let mut all = true;
        for step in &mut self.evidence.steps {
            let landed = self
                .chain
                .transaction(&step.signature, "finalized")?
                .ok_or_else(|| format!("{} is not finalized", step.id))?;
            let same = landed.slot == step.slot && landed.error == step.observed_error;
            all = all && same;
            if same {
                step.confirmation = String::from("finalized");
            }
        }
        self.evidence.summary.all_finalized = all;
        if !all {
            return Err("a finalized transaction differs from its confirmed record".into());
        }

        let state = self.state()?;
        self.evidence.final_state = state.json();
        let authority = state.authority()?.clone();
        let risk_key = state.risk_key.clone().ok_or("risk key is missing")?;
        let policy = state.policy.clone().ok_or("policy is missing")?;
        let mut receipts = Vec::new();
        for (name, step, approval) in &self.spent {
            let address = self.ns.receipt(&approval.nonce());
            let account = self
                .chain
                .account(&address, "finalized")?
                .ok_or_else(|| format!("receipt {name} is missing"))?;
            let stored: DeploymentReceipt = decode(&account.data)?;
            receipts.push(Receipt {
                approval: name.clone(),
                step: step.clone(),
                address: address.to_string(),
                nonce_hex: hex(&stored.nonce),
                amount: stored.amount,
                issued_at: stored.issued_at,
                expires_at: stored.expires_at,
                consumed_at: stored.consumed_at,
                message_sha256: hex(&stored.message_hash),
                lamports: account.lamports,
                data_len: account.data.len(),
                data_sha256: sha256_hex(&account.data),
            });
        }
        self.evidence.authorization = Authorization {
            deployment_authority: self.ns.deployment_authority.to_string(),
            administrator: authority.administrator.to_string(),
            network_domain_hex: hex(&authority.network_domain),
            network_domain_is_genesis: authority.network_domain == self.network_domain,
            policy_generation: authority.policy_generation,
            risk_key_account: self.ns.risk_key(&risk_key.public_key).to_string(),
            risk_public_key_hex: hex(&risk_key.public_key),
            risk_key_not_before: risk_key.not_before,
            risk_key_not_after: risk_key.not_after,
            authorization_policy: self.ns.authorization_policy.to_string(),
            adopted_at: policy.adopted_at,
            receipts,
        };
        Ok(())
    }

    fn summarize(&mut self, completed: bool) {
        let steps = &self.evidence.steps;
        let checks = steps.iter().flat_map(|step| step.checks.iter());
        self.evidence.summary.transactions = steps.len();
        self.evidence.summary.expected_successes = steps
            .iter()
            .filter(|step| step.expected_error.is_none())
            .count();
        self.evidence.summary.expected_failures = steps
            .iter()
            .filter(|step| step.expected_error.is_some())
            .count();
        self.evidence.summary.checks = checks.clone().count();
        self.evidence.summary.checks_passed = checks.filter(|check| check.passed).count();
        self.evidence.summary.completed = completed;
        self.evidence.completed_at_utc = now_utc();
    }

    fn write(&self) -> Res<PathBuf> {
        let dir = self.settings.state_dir();
        DirBuilder::new().recursive(true).mode(0o700).create(&dir)?;
        let path = self.settings.evidence_path();
        fs::write(&path, serde_json::to_string_pretty(&self.evidence)?)?;
        Ok(path)
    }

    // Steps and checks

    fn step(
        &mut self,
        spec: Spec,
        instructions: &[Instruction],
        signers: &[&Keypair],
    ) -> Res<Outcome> {
        println!("{} {}", spec.id, spec.title);
        let before = self.state()?;
        let transaction = self
            .chain
            .sign(instructions, &self.keys.deployer, signers)?;
        let transaction_bytes = wire(&transaction)?.len();
        let wanted = spec.expect.transaction_error();

        let simulation = match &wanted {
            None => None,
            Some(error) => {
                let simulated = self.chain.simulate(&transaction)?;
                let matched = simulated.error.as_ref() == Some(error);
                if !matched {
                    return Err(format!(
                        "{}: simulation predicted {}, not {error}; nothing was submitted\n{}",
                        spec.id,
                        simulated
                            .error
                            .as_ref()
                            .map_or_else(|| String::from("success"), ToString::to_string),
                        simulated.logs.join("\n")
                    )
                    .into());
                }
                Some(Simulation {
                    error: simulated.error,
                    compute_units: simulated.compute_units,
                    matched_expectation: matched,
                })
            }
        };

        let landed = self.chain.submit(&transaction, wanted.is_some())?;
        self.last_slot = self.last_slot.max(landed.slot);
        let after = self.state()?;

        let expected = spec.expect.describe();
        let observed = match (&landed.error, landed.failed_instruction()) {
            (None, _) => String::from("success"),
            (Some(error), Some(index)) => {
                let name = landed
                    .logged_error()
                    .map_or_else(String::new, |name| format!(" {name}"));
                format!(
                    "failed in instruction {index}:{name} {}",
                    error.pointer("/InstructionError/1").unwrap_or(error)
                )
            }
            (Some(error), None) => format!("failed: {error}"),
        };
        let matched = landed.error == wanted;
        self.evidence.steps.push(Step {
            id: spec.id.to_owned(),
            title: spec.title.to_owned(),
            program: spec.program.to_string(),
            instruction: spec.instruction.to_owned(),
            expected: expected.clone(),
            outcome: observed,
            expected_error: match &spec.expect {
                Expect::Success => None,
                Expect::Fails { index, error, name } => Some(ExpectedError {
                    instruction_index: *index,
                    error: error.clone(),
                    name: name.clone(),
                }),
            },
            observed_error: landed.error.clone(),
            failed_instruction: landed.failed_instruction(),
            error_code: landed.custom_code(),
            simulation,
            signature: landed.signature.clone(),
            slot: landed.slot,
            block_time_utc: landed.block_time.map(utc).unwrap_or_default(),
            confirmation: String::from("confirmed"),
            compute_units: landed.compute_units,
            fee_lamports: landed.fee,
            transaction_version: String::from("legacy"),
            transaction_bytes,
            instructions: landed.programs.clone(),
            explorer_url: report::tx_url(&self.evidence, &landed.signature),
            accounts: spec
                .accounts
                .iter()
                .map(|(role, key)| ((*role).to_owned(), key.to_string()))
                .collect(),
            before: before.json(),
            after: after.json(),
            details: Value::Null,
            checks: Vec::new(),
        });
        self.check(format!("on-chain outcome: {expected}"), matched)?;
        if wanted.is_some() {
            self.check(
                "every tracked account and token balance is byte-for-byte unchanged",
                before.raw == after.raw,
            )?;
        }
        Ok(Outcome {
            landed,
            before,
            after,
        })
    }

    /// A step that relays a risk approval, with its size and cost recorded.
    fn authorized_step(
        &mut self,
        spec: Spec,
        instructions: &[Instruction],
        approval: &str,
    ) -> Res<Outcome> {
        let fails = matches!(spec.expect, Expect::Fails { .. });
        let id = spec.id;
        let out = self.step(spec, instructions, &[])?;
        let receipt = out.after.receipt(approval);
        if fails {
            let before = out
                .before
                .receipt(approval)
                .map(|receipt| receipt.consumed_at);
            self.check(
                format!("the {approval} receipt is exactly as before: no approval was spent"),
                receipt.map(|receipt| receipt.consumed_at) == before,
            )?;
        }
        let step = self
            .evidence
            .steps
            .last()
            .ok_or("measurement outside a step")?;
        let receipt_rent = if fails {
            None
        } else {
            let address = self.ns.receipt(&self.nonce_of(approval)?);
            self.chain
                .account(&address, "confirmed")?
                .map(|account| account.lamports)
        };
        let measurement = Measurement {
            step: id.to_owned(),
            outcome: step.outcome.clone(),
            transaction_version: step.transaction_version.clone(),
            transaction_bytes: step.transaction_bytes,
            packet_limit_bytes: PACKET_LIMIT,
            instructions: instructions.len(),
            account_keys: out.landed.account_keys.len(),
            compute_units: out.landed.compute_units,
            verifier_compute_units: consumed(&out.landed.logs, &es256_authorization_verifier::ID),
            leg_compute_units: consumed(&out.landed.logs, &solevm_remote_leg::ID),
            fee_lamports: out.landed.fee,
            receipt_rent_lamports: receipt_rent,
        };
        self.detail(json!({
            "transaction_bytes": measurement.transaction_bytes,
            "account_keys": measurement.account_keys,
            "verifier_compute_units": measurement.verifier_compute_units,
            "leg_compute_units": measurement.leg_compute_units,
        }))?;
        self.check(
            format!(
                "the legacy transaction is {} bytes, within the {PACKET_LIMIT}-byte packet limit",
                measurement.transaction_bytes
            ),
            measurement.transaction_bytes <= PACKET_LIMIT,
        )?;
        self.evidence.measurements.push(measurement);
        Ok(out)
    }

    fn check(&mut self, what: impl Into<String>, passed: bool) -> Res<()> {
        let what = what.into();
        println!("    [{}] {what}", if passed { " ok " } else { "FAIL" });
        let step = self
            .evidence
            .steps
            .last_mut()
            .ok_or("check outside a step")?;
        step.checks.push(Check {
            what: what.clone(),
            passed,
        });
        if passed {
            Ok(())
        } else {
            Err(format!("{}: check failed: {what}", step.id).into())
        }
    }

    fn detail(&mut self, details: Value) -> Res<()> {
        let step = self
            .evidence
            .steps
            .last_mut()
            .ok_or("detail outside a step")?;
        match (&mut step.details, details) {
            (Value::Object(existing), Value::Object(added)) => existing.extend(added),
            (slot, details) => *slot = details,
        }
        Ok(())
    }

    /// A deployment spent exactly its approval and moved exactly its amount.
    fn check_deployed(
        &mut self,
        out: &Outcome,
        approval: &Approval,
        amount: u64,
        attributed: u64,
        deployed: u64,
    ) -> Res<()> {
        let name = self
            .receipts
            .iter()
            .find(|(_, nonce)| *nonce == approval.nonce())
            .map(|(name, _)| name.clone())
            .ok_or("approval is not tracked")?;
        let receipt = out
            .after
            .receipt(&name)
            .ok_or("the receipt was not created")?
            .clone();
        let step = self
            .evidence
            .steps
            .last()
            .map(|step| step.id.clone())
            .unwrap_or_default();
        self.spent.push((name, step, approval.clone()));
        let message = approval.message;
        self.detail(json!({
            "receipt": self.ns.receipt(&approval.nonce()).to_string(),
            "nonce": hex(&approval.nonce()),
            "amount": receipt.amount,
            "issued_at": receipt.issued_at,
            "expires_at": receipt.expires_at,
            "consumed_at": receipt.consumed_at,
            "deployment_context": hex(&receipt.deployment_context),
            "message_sha256": approval.message_sha256(),
            "signature": hex(&approval.signature),
        }))?;
        self.check(
            "the receipt records this authority, namespace, risk key, nonce, amount, context and message hash",
            receipt.config == self.ns.deployment_authority
                && receipt.remote_config == self.ns.base.config
                && receipt.risk_key == self.ns.risk_key(&self.risk.public_key())
                && receipt.nonce == message.nonce
                && receipt.amount == amount
                && receipt.deployment_context == message.deployment_context
                && receipt.policy_generation == message.policy_generation
                && hex(&receipt.message_hash) == approval.message_sha256(),
        )?;
        self.check(
            "it was consumed by the cluster clock within the approval's lifetime",
            receipt.issued_at == message.issued_at
                && receipt.expires_at == message.expires_at
                && receipt.consumed_at >= message.issued_at
                && receipt.consumed_at < message.expires_at,
        )?;
        let deployed_event: AssetsDeployed =
            event(&out.landed.logs).ok_or("no AssetsDeployed event")?;
        let position = out.after.position()?;
        self.check(
            format!("{amount} moved from custody to the adapter: attributed {attributed}, deployed {deployed}"),
            deployed_event.deployed_now == amount
                && position.attributed_principal == attributed
                && position.deployed_principal == deployed
                && out.before.custody()?.checked_sub(amount) == Some(out.after.custody()?)
                && out.before.vault()?.checked_add(amount) == Some(out.after.vault()?)
                && out.after.adapter()?.principal == deployed,
        )?;
        Ok(())
    }

    /// The transaction names no verifier program or verifier-owned account,
    /// and not the leg's authorization policy.
    fn check_no_authorization(&mut self, landed: &Landed) -> Res<()> {
        let mut forbidden = vec![
            es256_authorization_verifier::ID,
            self.ns.deployment_authority,
            self.ns.authorization_policy,
            self.ns.risk_key(&self.risk.public_key()),
        ];
        forbidden.extend(
            self.receipts
                .iter()
                .map(|(_, nonce)| self.ns.receipt(nonce)),
        );
        let named = forbidden
            .iter()
            .any(|key| landed.account_keys.contains(&key.to_string()));
        self.check(
            "the transaction names neither the verifier, its accounts, nor the authorization policy",
            !named,
        )
    }

    // Approvals

    /// An approval of `amount` for the leg as the chain holds it now,
    /// valid for the longest lifetime the verifier accepts.
    fn approve(&mut self, name: &str, amount: u64) -> Res<Approval> {
        let now = self.cluster_time()?;
        let expires = now
            .checked_add(MAX_LIFETIME_SECONDS)
            .ok_or("time overflow")?;
        self.approve_at(name, amount, now, expires)
    }

    fn approve_at(
        &mut self,
        name: &str,
        amount: u64,
        issued_at: i64,
        expires_at: i64,
    ) -> Res<Approval> {
        let state = self.state()?;
        let nonce = self.ns.nonce(name);
        let message = DeploymentAuthorization {
            authority: self.ns.deployment_authority,
            network_domain: self.network_domain,
            policy_generation: state.authority()?.policy_generation,
            deployment_context: self.context(&state)?.commitment(),
            amount,
            nonce,
            issued_at,
            expires_at,
        };
        if self.receipts.iter().all(|(known, _)| known != name) {
            self.receipts.push((name.to_owned(), nonce));
        }
        self.risk.approve(message)
    }

    fn nonce_of(&self, name: &str) -> Res<[u8; 16]> {
        self.receipts
            .iter()
            .find(|(known, _)| known == name)
            .map(|(_, nonce)| *nonce)
            .ok_or_else(|| format!("approval {name} is not tracked").into())
    }

    /// The deployment a risk authority evaluates, read from the accounts as
    /// they are now.
    fn context(&self, state: &State) -> Res<DeploymentContext> {
        let config = state.config()?;
        let strategy = state.strategy()?;
        Ok(DeploymentContext {
            remote_leg_program: solevm_remote_leg::ID,
            remote_config: self.ns.base.config,
            deployment_id: config.deployment_id,
            vault_id: config.vault_id,
            config_version: config.config_version,
            strategy_config: self.ns.base.strategy_config,
            adapter_program: strategy.adapter_program,
            adapter_state: strategy.adapter_state,
            adapter_authority: strategy.adapter_authority,
            adapter_token_vault: strategy.adapter_token_vault,
            max_remote_principal: strategy.max_remote_principal,
            asset_mint: config.asset_mint,
            token_program: config.token_program,
            custody_authority: config.custody_authority,
            custody_token_account: config.custody_token_account,
            deployed_principal: state.position()?.deployed_principal,
        })
    }

    /// `[secp256r1, consume, deploy_to_strategy]`, relayed by the deployer.
    fn authorized_deploy(
        &self,
        approval: &Approval,
        precompile: Instruction,
        amount: u64,
    ) -> Vec<Instruction> {
        let receipt = self.ns.receipt(&approval.nonce());
        vec![
            precompile,
            self.consume_ix(approval.nonce()),
            self.deploy_ix(amount, receipt),
        ]
    }

    // Instructions

    fn leg(&self, accounts: impl ToAccountMetas, data: impl InstructionData) -> Instruction {
        Instruction {
            program_id: solevm_remote_leg::ID,
            accounts: accounts.to_account_metas(None),
            data: data.data(),
        }
    }

    fn verifier(&self, accounts: impl ToAccountMetas, data: impl InstructionData) -> Instruction {
        Instruction {
            program_id: es256_authorization_verifier::ID,
            accounts: accounts.to_account_metas(None),
            data: data.data(),
        }
    }

    fn control_params(&self) -> ControlStateParams {
        ControlStateParams {
            max_remote_allocation_bps: 6_000,
            max_upward_deviation_bps: 200,
            max_downward_deviation_bps: 1_000,
            max_report_age: 3_600,
            config_commitment: self.ns.base.tag("config/1"),
            mandatory_watermark_lag: 2,
        }
    }

    fn initialize_ix(&self, upgrade_authority: Pubkey) -> Instruction {
        let ns = &self.ns.base;
        self.leg(
            solevm_remote_leg::accounts::InitializeRemoteLeg {
                administrator: self.keys.administrator.pubkey(),
                remote_config: ns.config,
                asset_mint: ns.mint,
                custody_token_account: ns.custody,
                outbound_escrow: ns.escrow,
                token_program: spl_token::ID,
                system_program: system_program::ID,
                upgrade_authority,
                program_data: ns.program_data,
            },
            solevm_remote_leg::instruction::InitializeRemoteLeg {
                params: InitializeParams {
                    deployment_id: ns.deployment_id,
                    vault_id: ns.vault_id,
                    source_chain_id: SOURCE_CHAIN_ID,
                    destination_chain_id: DESTINATION_CHAIN_ID,
                    source_application_id: self.domain.source_application,
                    local_application_id: self.domain.local_application,
                    control_lane_id: CONTROL_LANE_ID,
                    report_lane_id: REPORT_LANE_ID,
                    config_version: CONFIG_VERSION,
                    transport_verifier: self.keys.verifier.pubkey(),
                    emergency_guardian: self.keys.guardian.pubkey(),
                },
            },
        )
    }

    fn initialize_authority_ix(
        &self,
        upgrade_authority: Pubkey,
        params: InitializeDeploymentAuthorityParams,
    ) -> Instruction {
        self.verifier(
            es256_authorization_verifier::accounts::InitializeDeploymentAuthority {
                upgrade_authority,
                program_data: self.ns.verifier_program_data,
                remote_leg_program: solevm_remote_leg::ID,
                remote_config: self.ns.base.config,
                config: self.ns.deployment_authority,
                system_program: system_program::ID,
            },
            es256_authorization_verifier::instruction::InitializeDeploymentAuthority { params },
        )
    }

    fn register_ix(&self, administrator: Pubkey, params: RegisterRiskKeyParams) -> Instruction {
        self.verifier(
            es256_authorization_verifier::accounts::RegisterRiskKey {
                administrator,
                config: self.ns.deployment_authority,
                risk_key: self.ns.risk_key(&params.public_key),
                system_program: system_program::ID,
            },
            es256_authorization_verifier::instruction::RegisterRiskKey { params },
        )
    }

    fn adopt_ix(&self, administrator: Pubkey) -> Instruction {
        self.leg(
            solevm_remote_leg::accounts::InitializeDeployAuthorization {
                administrator,
                remote_config: self.ns.base.config,
                authorization_config: self.ns.deployment_authority,
                authorization_policy: self.ns.authorization_policy,
                system_program: system_program::ID,
            },
            solevm_remote_leg::instruction::InitializeDeployAuthorization {},
        )
    }

    fn consume_ix(&self, nonce: [u8; 16]) -> Instruction {
        self.verifier(
            es256_authorization_verifier::accounts::ConsumeDeploymentAuthorization {
                payer: self.keys.deployer.pubkey(),
                config: self.ns.deployment_authority,
                risk_key: self.ns.risk_key(&self.risk.public_key()),
                receipt: self.ns.receipt(&nonce),
                instructions_sysvar: INSTRUCTIONS_SYSVAR,
                system_program: system_program::ID,
            },
            es256_authorization_verifier::instruction::ConsumeDeploymentAuthorization { nonce },
        )
    }

    fn deploy_ix(&self, amount: u64, receipt: Pubkey) -> Instruction {
        let ns = &self.ns.base;
        self.leg(
            solevm_remote_leg::accounts::DeployToStrategy {
                remote_config: ns.config,
                strategy_config: ns.strategy_config,
                remote_position: ns.position,
                custody_authority: ns.custody_authority,
                custody_token_account: ns.custody,
                adapter_caller: self.ns.adapter_caller,
                adapter_program: solevm_test_strategy::ID,
                adapter_state: ns.adapter_state,
                adapter_authority: ns.adapter_authority,
                adapter_token_vault: ns.adapter_vault,
                asset_mint: ns.mint,
                token_program: spl_token::ID,
                authorization_policy: self.ns.authorization_policy,
                authorization_receipt: receipt,
                instructions_sysvar: INSTRUCTIONS_SYSVAR,
            },
            solevm_remote_leg::instruction::DeployToStrategy { amount },
        )
    }

    fn conditions_ix(&self, deposits_paused: bool) -> Instruction {
        Instruction {
            program_id: solevm_test_strategy::ID,
            accounts: solevm_test_strategy::accounts::ConfigureAdapter {
                test_authority: self.keys.administrator.pubkey(),
                adapter_state: self.ns.base.adapter_state,
            }
            .to_account_metas(None),
            data: solevm_test_strategy::instruction::ConfigureAdapterTestConditions {
                conditions: solevm_test_strategy::TestConditions {
                    max_liquid_principal: u64::MAX,
                    loss_bps: 0,
                    deposits_paused,
                },
            }
            .data(),
        }
    }

    // Composite steps

    fn allocate(
        &mut self,
        id: &'static str,
        title: &'static str,
        name: &str,
        amount: u64,
    ) -> Res<Outcome> {
        let keys = self.keys;
        let ns = self.ns.base.clone();
        let lane = self.state()?.lane()?.clone();
        let sequence = lane
            .highest_consumed_sequence
            .checked_add(1)
            .ok_or("sequence overflow")?;
        let now = u64::try_from(self.cluster_time()?)?;
        let transfer_id = ns.transfer_id(name);
        let message = messages::allocate(
            &self.domain,
            sequence,
            lane.message_commitment,
            now,
            transfer_id,
            amount,
        )?;
        let record = ns.transfer_record(&transfer_id);
        let instruction = self.leg(
            solevm_remote_leg::accounts::ProcessAllocate {
                transport_verifier: keys.verifier.pubkey(),
                remote_config: ns.config,
                risk_config: ns.risk_config,
                strategy_config: ns.strategy_config,
                remote_position: ns.position,
                control_lane: ns.control_lane,
                transfer_record: record,
                consumed_message: ns.consumed(sequence),
                system_program: system_program::ID,
            },
            solevm_remote_leg::instruction::ProcessAllocate {
                message_bytes: message.bytes.clone(),
            },
        );
        let out = self.step(
            Spec {
                id,
                title,
                program: solevm_remote_leg::ID,
                instruction: "process_allocate",
                expect: Expect::Success,
                accounts: vec![
                    ("transport_verifier", keys.verifier.pubkey()),
                    ("transfer_record", record),
                    ("consumed_message", ns.consumed(sequence)),
                ],
            },
            &[instruction],
            &[&keys.verifier],
        )?;
        let accepted: AllocateAccepted =
            event(&out.landed.logs).ok_or("no AllocateAccepted event")?;
        self.detail(json!({
            "sequence": sequence,
            "transfer_id": hex(&transfer_id),
            "message_id": hex(&message.id),
        }))?;
        let record = out
            .after
            .transfer(name)
            .ok_or("transfer record is missing")?;
        self.check(
            format!("transfer {name} is pending with {amount} authorized at sequence {sequence}"),
            accepted.sequence == sequence
                && accepted.message_id == message.id
                && record.status == TransferStatus::Pending
                && record.authorized_amount == amount
                && out.after.lane()?.message_commitment
                    == messages::next_commitment(lane.message_commitment, message.id),
        )?;
        Ok(out)
    }

    fn deposit(&mut self, id: &'static str, title: &'static str, amount: u64) -> Res<Outcome> {
        let keys = self.keys;
        let ns = self.ns.base.clone();
        let transfer = spl_token::instruction::transfer_checked(
            &spl_token::ID,
            &ns.transport_source,
            &ns.mint,
            &ns.custody,
            &keys.transport.pubkey(),
            &[],
            amount,
            DECIMALS,
        )
        .map_err(|error| format!("transfer_checked: {error:?}"))?;
        let out = self.step(
            Spec {
                id,
                title,
                program: spl_token::ID,
                instruction: "spl_token::transfer_checked",
                expect: Expect::Success,
                accounts: vec![
                    ("from", ns.transport_source),
                    ("custody_token_account", ns.custody),
                ],
            },
            &[transfer],
            &[&keys.transport],
        )?;
        self.check(
            format!("custody balance rose by exactly {amount}"),
            out.before.custody()?.checked_add(amount) == Some(out.after.custody()?),
        )?;
        Ok(out)
    }

    fn attribute(&mut self, id: &'static str, title: &'static str, name: &str) -> Res<Outcome> {
        let ns = self.ns.base.clone();
        let record = ns.transfer_record(&ns.transfer_id(name));
        let instruction = self.leg(
            solevm_remote_leg::accounts::AttributeAllocation {
                remote_config: ns.config,
                remote_position: ns.position,
                transfer_record: record,
                custody_token_account: ns.custody,
                emergency_guardian: None,
            },
            solevm_remote_leg::instruction::AttributeAllocation {},
        );
        let out = self.step(
            Spec {
                id,
                title,
                program: solevm_remote_leg::ID,
                instruction: "attribute_allocation",
                expect: Expect::Success,
                accounts: vec![("transfer_record", record)],
            },
            &[instruction],
            &[],
        )?;
        self.check(
            "custody equals attributed + recalled + unattributed",
            out.after.custody_explained()?,
        )?;
        Ok(out)
    }

    // Reads

    /// The cluster clock the programs read, from the Clock sysvar.
    fn cluster_time(&self) -> Res<i64> {
        let clock = self
            .chain
            .account(&CLOCK_SYSVAR, "confirmed")?
            .ok_or("clock sysvar is missing")?;
        let bytes = clock.data.get(32..40).ok_or("clock sysvar is too short")?;
        let mut raw = [0u8; 8];
        raw.copy_from_slice(bytes);
        Ok(i64::from_le_bytes(raw))
    }

    fn token_mint(&self, key: &Pubkey) -> Res<spl_token::state::Mint> {
        let account = self
            .chain
            .account(key, "confirmed")?
            .ok_or("mint is missing")?;
        spl_token::state::Mint::unpack(&account.data)
            .map_err(|error| format!("mint: {error:?}").into())
    }

    fn token_account(&self, key: &Pubkey) -> Res<spl_token::state::Account> {
        let account = self
            .chain
            .account(key, "confirmed")?
            .ok_or("token account is missing")?;
        spl_token::state::Account::unpack(&account.data)
            .map_err(|error| format!("token account: {error:?}").into())
    }

    /// Every tracked account at one slot, no earlier than the last step.
    fn state(&self) -> Res<State> {
        let ns = &self.ns;
        let base = &ns.base;
        let mut keys = vec![
            base.config,
            base.risk_config,
            base.control_lane,
            base.outbound_lane,
            base.strategy_config,
            base.position,
            base.adapter_state,
            ns.deployment_authority,
            ns.risk_key(&self.risk.public_key()),
            ns.authorization_policy,
            base.custody,
            base.transport_source,
            base.escrow,
            base.adapter_vault,
        ];
        keys.extend(
            self.transfers
                .iter()
                .map(|(_, id)| base.transfer_record(id)),
        );
        keys.extend(self.receipts.iter().map(|(_, nonce)| ns.receipt(nonce)));
        keys.push(self.keys.deployer.pubkey());
        let (slot, mut accounts) = self.chain.accounts(&keys, Some(self.last_slot))?;
        let payer_lamports = accounts.pop().flatten().map(|payer| payer.lamports);
        let data: Vec<Option<Vec<u8>>> = accounts
            .into_iter()
            .map(|account| account.map(|found| found.data))
            .collect();
        let token = |index: usize| -> Res<Option<u64>> {
            data.get(index)
                .and_then(Option::as_ref)
                .map(|bytes| spl_token::state::Account::unpack(bytes).map(|account| account.amount))
                .transpose()
                .map_err(|error| format!("token account: {error:?}").into())
        };
        let mut transfers = Vec::with_capacity(self.transfers.len());
        for (offset, (name, _)) in self.transfers.iter().enumerate() {
            let index = TRACKED.checked_add(offset).ok_or("index overflow")?;
            transfers.push((name.clone(), decode_at(&data, index)?));
        }
        let mut receipts = Vec::with_capacity(self.receipts.len());
        for (offset, (name, _)) in self.receipts.iter().enumerate() {
            let index = TRACKED
                .checked_add(self.transfers.len())
                .and_then(|start| start.checked_add(offset))
                .ok_or("index overflow")?;
            receipts.push((name.clone(), decode_at(&data, index)?));
        }
        Ok(State {
            slot,
            payer_lamports,
            config: decode_at(&data, 0)?,
            lane: decode_at(&data, 2)?,
            outbound: decode_at(&data, 3)?,
            strategy: decode_at(&data, 4)?,
            position: decode_at(&data, 5)?,
            adapter: decode_at(&data, 6)?,
            authority: decode_at(&data, 7)?,
            risk_key: decode_at(&data, 8)?,
            policy: decode_at(&data, 9)?,
            custody: token(10)?,
            source: token(11)?,
            escrow: token(12)?,
            vault: token(13)?,
            transfers,
            receipts,
            raw: data,
        })
    }
}

/// Compute units the runtime charged the top-level invocation of `program`.
fn consumed(logs: &[String], program: &Pubkey) -> Option<u64> {
    let prefix = format!("Program {program} consumed ");
    logs.iter()
        .filter_map(|line| line.strip_prefix(&prefix))
        .filter_map(|rest| rest.split_once(' '))
        .filter_map(|(units, _)| units.parse().ok())
        .next_back()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expectations_match_the_rpc_error_shape() {
        let cases = [
            (
                Expect::verifier(1, VerifierError::InvalidDeploymentInstruction),
                json!({ "InstructionError": [1, { "Custom": 6053 }] }),
            ),
            (
                Expect::verifier(1, VerifierError::Expired),
                json!({ "InstructionError": [1, { "Custom": 6033 }] }),
            ),
            (
                Expect::verifier(0, VerifierError::Unauthorized),
                json!({ "InstructionError": [0, { "Custom": 6000 }] }),
            ),
            (
                Expect::precompile(0, PrecompileError::InvalidSignature),
                json!({ "InstructionError": [0, { "Custom": 2 }] }),
            ),
            (
                Expect::adapter(2, AdapterError::DepositsPaused),
                json!({ "InstructionError": [2, { "Custom": 6009 }] }),
            ),
            (
                Expect::anchor(2, AnchorError::AccountNotInitialized),
                json!({ "InstructionError": [2, { "Custom": 3012 }] }),
            ),
            (
                Expect::already_in_use(1),
                json!({ "InstructionError": [1, { "Custom": 0 }] }),
            ),
        ];
        for (expect, wanted) in cases {
            assert_eq!(expect.transaction_error(), Some(wanted));
        }
        assert_eq!(Expect::Success.transaction_error(), None);
    }

    #[test]
    fn consumed_reads_the_outermost_invocation() {
        let program = Pubkey::new_unique();
        let other = Pubkey::new_unique();
        let logs = [
            format!("Program {program} invoke [1]"),
            format!("Program {other} invoke [2]"),
            format!("Program {other} consumed 1200 of 190000 compute units"),
            format!("Program {program} consumed 24000 of 200000 compute units"),
        ];
        assert_eq!(consumed(&logs, &program), Some(24_000));
        assert_eq!(consumed(&logs, &other), Some(1_200));
        assert_eq!(consumed(&logs, &Pubkey::new_unique()), None);
    }
}
