//! One fresh namespace: deployment checks, setup, and every accounting path
//! the brief asks for, each step checked against the state it leaves behind.

use std::collections::BTreeMap;
use std::fs::{self, DirBuilder};
use std::os::unix::fs::DirBuilderExt as _;
use std::path::PathBuf;
use std::thread;
use std::time::{Duration, Instant};

use anchor_lang::prelude::Pubkey;
use anchor_lang::solana_program::bpf_loader_upgradeable;
use anchor_lang::solana_program::program_pack::Pack;
use anchor_lang::{
    AccountDeserialize, AnchorDeserialize, Discriminator, InstructionData, ToAccountMetas,
    system_program,
};
use anchor_spl::token::spl_token;
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use protocol_types::{Body, ResolveAction, SettlementKind};
use serde_json::{Value, json};
use solana_instruction::Instruction;
use solana_keypair::Keypair;
use solana_signer::Signer;
use solevm_remote_leg::{
    AllocateAccepted, ControlStateParams, CustodyReconciled, InitializeParams, OutboundLane,
    RemoteConfig, RemoteLegError, RemoteLegFrozen, RemotePosition, RemoteReportPublished,
    ReplayLane, ReportRecord, ReportType, SettlementProcessed, StrategyConfig, TransferKind,
    TransferRecord, TransferStatus, TransportLedger, TransportSettlementPublished,
};
use spl_associated_token_account_interface::instruction::create_associated_token_account_idempotent;

use crate::Res;
use crate::chain::{Chain, DEVNET_GENESIS, Landed, MAINNET_GENESIS, TESTNET_GENESIS};
use crate::evidence::{self, Build, Check, Evidence, Mint, Program, SCHEMA, Source, Step};
use crate::keys::Keys;
use crate::messages::{
    self, CONFIG_VERSION, CONTROL_LANE_ID, DESTINATION_CHAIN_ID, Domain, Encoded, REPORT_LANE_ID,
    SOURCE_CHAIN_ID,
};
use crate::namespace::Namespace;
use crate::report;
use crate::settings::Settings;
use crate::time::{now_utc, utc};

const MEMO_PROGRAM: Pubkey = Pubkey::from_str_const("MemoSq4gqABAXKb96qnH8TysNcWxMyWCqXgDLGmfcHr");
const CLOCK_SYSVAR: Pubkey = Pubkey::from_str_const("SysvarC1ock11111111111111111111111111111111");
const MINT_LABEL: &str = "SolEVM devnet test token: 6 decimals, no value, not USDC";
const DECIMALS: u8 = 6;
/// Base units minted to the wallet that plays the bridge.
const MINTED: u64 = 5_000_000;
const MAX_REMOTE_PRINCIPAL: u64 = 10_000_000;
/// Lamports for the two roles that pay rent for the records they create.
const ROLE_FUNDING: u64 = 50_000_000;
/// Role funding, the mint, four token accounts, two report records and the
/// fees of every step come to about 0.112 SOL at devnet rent.
const MIN_RUN_BALANCE: u64 = 150_000_000;
const FINALIZE_TIMEOUT: Duration = Duration::from_secs(300);

// Scenario amounts in base units, with every derived value spelled out.
/// Allocation A arrives in full.
const FIRST: u64 = 1_000_000;
/// Allocation B is closed after only part of it arrived.
const SECOND: u64 = 1_000_000;
const SECOND_ARRIVED: u64 = 600_000;
const SECOND_SHORTFALL: u64 = 400_000;
/// Arrives after B closed and is recovered against the impairment pool.
const LATE: u64 = 300_000;
const IMPAIRED_AFTER_RECOVERY: u64 = 100_000;
/// Allocation C is closed by the guardian on a frozen leg with nothing arrived.
const THIRD: u64 = 500_000;
const IMPAIRED_AT_END: u64 = 600_000;
const PRINCIPAL_AFTER_CLOSE: u64 = 1_600_000;
const PRINCIPAL_AFTER_RECOVERY: u64 = 1_900_000;
const CLOSED_CLAIMS_AT_END: u128 = 1_500_000;
const CLOSED_CREDIT_AT_END: u128 = 600_000;

/// The leg id of the recorded V1 devnet instance.
const V1_LEG_ID: Pubkey = Pubkey::from_str_const("27zReYgZvFTaUSWwob4qirN2Yxekcm32mnQDpMzeTyAE");

/// The V1 commands derive every address from the compiled leg id. A build
/// whose sources declare another id would address a different program while
/// reading the V1 keys and writing over V1 records, so it stops before
/// reading settings, keys or the network.
fn require_v1_build() -> Res<()> {
    if solevm_remote_leg::ID != V1_LEG_ID {
        return Err(format!(
            "these sources declare the V2 devnet leg {}, not the V1 leg {V1_LEG_ID}; \
             the V1 instance is read-only (verify and report still work), use run-v2",
            solevm_remote_leg::ID
        )
        .into());
    }
    Ok(())
}

pub(crate) fn print_addresses() -> Res<()> {
    require_v1_build()?;
    let settings = Settings::from_env()?;
    let keys = Keys::load(&settings.key_dir(), &settings.label)?;
    let namespace = Namespace::derive(&settings.label, &keys);
    for (role, key) in namespace.roles(&keys) {
        println!("{role:<36} {key}");
    }
    Ok(())
}

pub(crate) fn run() -> Res<()> {
    require_v1_build()?;
    let settings = Settings::from_env()?;
    let chain = Chain::new(&settings.rpc_url);
    let (cluster, genesis) = identify_cluster(&chain, settings.allow_localnet)?;
    let keys = Keys::load(&settings.key_dir(), &settings.label)?;
    let namespace = Namespace::derive(&settings.label, &keys);
    if chain.account(&namespace.config, "confirmed")?.is_some() {
        return Err(format!(
            "namespace '{}' already exists at {}; set a new SOLEVM_RUN_LABEL",
            settings.label, namespace.config
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

    let mut driver = Driver::new(&chain, &keys, &settings, namespace, cluster, genesis);
    let outcome = driver.execute();
    driver.summarize(outcome.is_ok());
    let path = driver.write()?;
    println!("evidence written to {}", path.display());
    outcome
}

fn identify_cluster(chain: &Chain, allow_localnet: bool) -> Res<(String, String)> {
    let genesis = chain.genesis_hash()?;
    if genesis == MAINNET_GENESIS || genesis == TESTNET_GENESIS {
        return Err(format!(
            "refusing to run against {} (genesis {genesis})",
            chain.url()
        )
        .into());
    }
    if genesis == DEVNET_GENESIS {
        return Ok((String::from("devnet"), genesis));
    }
    if allow_localnet {
        return Ok((String::from("localnet"), genesis));
    }
    Err(format!(
        "{} is not devnet (genesis {genesis}); set SOLEVM_ALLOW_LOCALNET=1 for a local validator",
        chain.url()
    )
    .into())
}

enum Expect {
    Success,
    Rejected(RemoteLegError),
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

/// Decoded leg state at one slot, plus the raw bytes behind it.
#[derive(Clone)]
struct State {
    slot: u64,
    raw: Vec<Option<Vec<u8>>>,
    config: Option<RemoteConfig>,
    lane: Option<ReplayLane>,
    outbound: Option<OutboundLane>,
    position: Option<RemotePosition>,
    strategy: Option<StrategyConfig>,
    ledger: Option<TransportLedger>,
    custody: Option<u64>,
    source: Option<u64>,
    transfers: Vec<(String, Option<TransferRecord>)>,
}

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
    fn position(&self) -> Res<&RemotePosition> {
        self.position
            .as_ref()
            .ok_or_else(|| "remote position is missing".into())
    }
    fn ledger(&self) -> Res<&TransportLedger> {
        self.ledger
            .as_ref()
            .ok_or_else(|| "transport ledger is missing".into())
    }
    fn custody(&self) -> Res<u64> {
        self.custody
            .ok_or_else(|| "custody token account is missing".into())
    }
    fn transfer(&self, name: &str) -> Option<&TransferRecord> {
        self.transfers
            .iter()
            .find(|(known, _)| known == name)
            .and_then(|(_, record)| record.as_ref())
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

    /// Every closed claim is credited, impaired or recovered, exactly once.
    fn ledger_balanced(&self) -> Res<bool> {
        let ledger = self.ledger()?;
        let explained = ledger
            .cumulative_closed_credit
            .checked_add(ledger.impaired_outstanding)
            .and_then(|total| total.checked_add(ledger.cumulative_recovered));
        Ok(explained == Some(ledger.cumulative_closed_claim))
    }

    fn json(&self) -> Value {
        let hex = |bytes: &[u8]| evidence::hex(bytes);
        json!({
            "context_slot": self.slot,
            "frozen": self.config.as_ref().map(|config| config.frozen),
            "control_lane": self.lane.as_ref().map(|lane| json!({
                "highest_consumed_sequence": lane.highest_consumed_sequence,
                "message_commitment": hex(&lane.message_commitment),
            })),
            "outbound_lane": self.outbound.as_ref().map(|lane| json!({
                "next_sequence": lane.next_sequence,
                "current_commitment": hex(&lane.current_commitment),
                "total_reports": lane.total_reports,
            })),
            "position": self.position.as_ref().map(|position| json!({
                "attributed_principal": position.attributed_principal,
                "deployed_principal": position.deployed_principal,
                "recalled_custody": position.recalled_custody,
                "unattributed_custody": position.unattributed_custody,
                "active_transfer_kind": format!("{:?}", position.active_transfer_kind),
                "active_transfer_status": format!("{:?}", position.active_transfer_status),
                "active_transfer_sequence": position.active_transfer_sequence,
            })),
            "transport_ledger": self.ledger.as_ref().map(|ledger| json!({
                "impaired_outstanding": ledger.impaired_outstanding.to_string(),
                "cumulative_closed_claim": ledger.cumulative_closed_claim.to_string(),
                "cumulative_closed_credit": ledger.cumulative_closed_credit.to_string(),
                "cumulative_recovered": ledger.cumulative_recovered.to_string(),
                "last_impaired_sequence": ledger.last_impaired_sequence,
                "total_closed": ledger.total_closed,
                "total_recoveries": ledger.total_recoveries,
            })),
            "custody_token_balance": self.custody,
            "transport_source_token_balance": self.source,
            "transfers": self.transfers.iter().map(|(name, record)| (name.clone(), record.as_ref().map(|record| json!({
                "status": format!("{:?}", record.status),
                "kind": format!("{:?}", record.transfer_kind),
                "sequence": record.message_sequence,
                "authorized_amount": record.authorized_amount,
                "attributed_amount": record.attributed_amount,
            })))).collect::<BTreeMap<_, _>>(),
        })
    }
}

struct Driver<'a> {
    chain: &'a Chain,
    keys: &'a Keys,
    settings: &'a Settings,
    ns: Namespace,
    domain: Domain,
    evidence: Evidence,
    /// Named transfers whose records every snapshot includes.
    transfers: Vec<(String, [u8; 32])>,
    last_slot: u64,
}

impl<'a> Driver<'a> {
    fn new(
        chain: &'a Chain,
        keys: &'a Keys,
        settings: &'a Settings,
        ns: Namespace,
        cluster: String,
        genesis: String,
    ) -> Self {
        let domain = Domain {
            source_application: protocol_types::keccak256(b"solevm/devnet/simulated-evm-hub"),
            local_application: solevm_remote_leg::ID.to_bytes(),
            deployment_id: ns.deployment_id,
            vault_id: ns.vault_id,
        };
        let transfers = ["A", "X", "B", "R", "C"]
            .iter()
            .map(|name| ((*name).to_owned(), ns.transfer_id(name)))
            .collect();
        let accounts = ns
            .roles(keys)
            .into_iter()
            .map(|(role, key)| (role.to_owned(), key.to_string()))
            .collect();
        let evidence = Evidence {
            schema: SCHEMA,
            cluster,
            rpc_url: chain.url().to_owned(),
            genesis_hash: genesis,
            run_label: ns.label.clone(),
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
                address: ns.mint.to_string(),
                decimals: DECIMALS,
                mint_authority: keys.transport.pubkey().to_string(),
                label: MINT_LABEL.to_owned(),
            },
            accounts,
            ..Evidence::default()
        };
        Self {
            chain,
            keys,
            settings,
            ns,
            domain,
            evidence,
            transfers,
            last_slot: 0,
        }
    }

    fn execute(&mut self) -> Res<()> {
        self.verify_programs()?;
        self.setup()?;
        self.allocation_and_reporting()?;
        self.close_and_recover()?;
        self.frozen_guardian_path()?;
        self.finalize()
    }

    // Deployment

    fn verify_programs(&mut self) -> Res<()> {
        let binaries = [
            (
                "solevm_remote_leg",
                solevm_remote_leg::ID,
                self.settings.leg_binary.clone(),
            ),
            (
                "solevm_test_strategy",
                solevm_test_strategy::ID,
                self.settings.adapter_binary.clone(),
            ),
        ];
        for (name, id, path) in binaries {
            let local = fs::read(&path).map_err(|error| format!("{}: {error}", path.display()))?;
            if !local.windows(32).any(|window| window == id.as_ref()) {
                return Err(format!(
                    "{} does not declare {id}; build it with scripts/devnet/build.sh",
                    path.display()
                )
                .into());
            }
            let deploy = self.deploy_record(name, &id)?;
            if deploy.is_none() {
                self.await_finalized_account(&id)?;
            }
            let found = evidence::read_program(self.chain, &id, local.len())?;
            let local_sha256 = evidence::sha256_hex(&local);
            let onchain_sha256 = evidence::sha256_hex(&found.code);
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
            if found.upgrade_authority != Some(self.keys.deployer.pubkey()) {
                return Err(format!("{name} upgrade authority is not the deployer").into());
            }
            if local_sha256 != onchain_sha256 {
                return Err(format!("{name} on chain differs from {}", path.display()).into());
            }
        }
        Ok(())
    }

    /// Waits until a program deployed by other means is visible at finalized
    /// commitment, which is where the program checks read.
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

    /// The deploy transaction the deploy script recorded, once finalized.
    fn deploy_record(&self, name: &str, id: &Pubkey) -> Res<Option<Landed>> {
        let path = self
            .settings
            .state_dir(&self.evidence.cluster)
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

    // Setup

    fn setup(&mut self) -> Res<()> {
        let keys = self.keys;
        let ns = self.ns.clone();
        let deployer = keys.deployer.pubkey();

        let fund = [
            solana_system_interface::instruction::transfer(
                &deployer,
                &keys.administrator.pubkey(),
                ROLE_FUNDING,
            ),
            solana_system_interface::instruction::transfer(
                &deployer,
                &keys.verifier.pubkey(),
                ROLE_FUNDING,
            ),
        ];
        self.step(
            Spec {
                id: "S01",
                title: "Fund the administrator and transport verifier, who pay rent for the records they create",
                program: system_program::ID,
                instruction: "system::transfer",
                expect: Expect::Success,
                accounts: vec![("administrator", keys.administrator.pubkey()), ("transport_verifier", keys.verifier.pubkey())],
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
                title: "Create the labelled 6-decimal devnet test mint (not Circle USDC)",
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
                    ("custody_authority", ns.custody_authority),
                    ("outbound_escrow", ns.escrow),
                    ("adapter_token_vault", ns.adapter_vault),
                    ("transport_source_token_account", ns.transport_source),
                ],
            },
            &accounts,
            &[&keys.transport],
        )?;
        let custody = self.token_account(&ns.custody)?;
        self.check("custody is owned by the custody-authority PDA, empty, with no delegate or close authority",
            custody.owner == ns.custody_authority
                && custody.mint == ns.mint
                && custody.amount == 0
                && custody.delegate.is_none()
                && custody.close_authority.is_none())?;
        let source = self.token_account(&ns.transport_source)?;
        self.check(
            format!("transport source holds the {MINTED} minted base units"),
            source.amount == MINTED,
        )?;

        let forged = self.initialize_ix(keys.attacker.pubkey());
        self.step(
            Spec {
                id: "N01",
                title: "Namespace claim co-signed by a key that is not the program's upgrade authority",
                program: solevm_remote_leg::ID,
                instruction: "initialize_remote_leg",
                expect: Expect::Rejected(RemoteLegError::Unauthorized),
                accounts: vec![("claimed_upgrade_authority", keys.attacker.pubkey()), ("remote_config", ns.config)],
            },
            &[forged],
            &[&keys.administrator, &keys.attacker],
        )?;
        let exists = self.chain.account(&ns.config, "confirmed")?.is_some();
        self.check("remote config was not created", !exists)?;

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
                    ("program_data", ns.program_data),
                    ("remote_config", ns.config),
                ],
            },
            &[initialize],
            &[&keys.administrator],
        )?;
        let config = out.after.config()?;
        self.check(
            "config binds administrator, guardian, verifier, mint, custody and escrow",
            config.administrator == keys.administrator.pubkey()
                && config.emergency_guardian == keys.guardian.pubkey()
                && config.transport_verifier == keys.verifier.pubkey()
                && config.asset_mint == ns.mint
                && config.custody_authority == ns.custody_authority
                && config.custody_token_account == ns.custody
                && config.outbound_escrow == ns.escrow,
        )?;
        self.check(
            "config binds the domain and starts unfrozen at version 1",
            config.deployment_id == ns.deployment_id
                && config.vault_id == ns.vault_id
                && config.source_chain_id == SOURCE_CHAIN_ID
                && config.destination_chain_id == DESTINATION_CHAIN_ID
                && config.control_lane_id == CONTROL_LANE_ID
                && config.report_lane_id == REPORT_LANE_ID
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
        let out = self.step(
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
        let lane = out.after.lane()?;
        self.check(
            "control lane starts empty with a zero commitment",
            lane.lane_id == CONTROL_LANE_ID
                && lane.highest_consumed_sequence == 0
                && lane.message_commitment == [0u8; 32],
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
        self.step(
            Spec {
                id: "S06",
                title: "Initialize the deterministic test strategy adapter bound to this namespace",
                program: solevm_test_strategy::ID,
                instruction: "initialize_adapter",
                expect: Expect::Success,
                accounts: vec![
                    ("adapter_state", ns.adapter_state),
                    ("adapter_token_vault", ns.adapter_vault),
                ],
            },
            &[adapter],
            &[&keys.administrator],
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
                title: "Create the strategy config and the remote position",
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
        let strategy = out
            .after
            .strategy
            .as_ref()
            .ok_or("strategy config is missing")?;
        let position = out.after.position()?;
        self.check(
            "strategy binds the adapter program, state and vault with the principal ceiling",
            strategy.adapter_program == solevm_test_strategy::ID
                && strategy.adapter_state == ns.adapter_state
                && strategy.adapter_token_vault == ns.adapter_vault
                && strategy.max_remote_principal == MAX_REMOTE_PRINCIPAL,
        )?;
        self.check(
            "position starts with every bucket at zero and no open transfer",
            position.attributed_principal == 0
                && position.deployed_principal == 0
                && position.recalled_custody == 0
                && position.unattributed_custody == 0
                && position.active_transfer_kind == TransferKind::None,
        )?;

        let outbound = self.leg(
            solevm_remote_leg::accounts::InitializeOutboundState {
                administrator: keys.administrator.pubkey(),
                remote_config: ns.config,
                outbound_lane: ns.outbound_lane,
                system_program: system_program::ID,
            },
            solevm_remote_leg::instruction::InitializeOutboundState {},
        );
        let out = self.step(
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
        let lane = out.after.outbound()?;
        self.check(
            "outbound lane starts at sequence 1 with no reports",
            lane.lane_id == REPORT_LANE_ID && lane.next_sequence == 1 && lane.total_reports == 0,
        )?;
        Ok(())
    }

    // Allocation, custody and reporting

    fn allocation_and_reporting(&mut self) -> Res<()> {
        let keys = self.keys;
        let ns = self.ns.clone();

        let (out, first) = self.allocate(
            "T01",
            "Authenticated Allocate A (sequence 1) delivered by the transport verifier",
            "A",
            FIRST,
        )?;
        let position = out.after.position()?;
        self.check(
            "position holds A as the open allocation",
            position.active_transfer_kind == TransferKind::Allocate
                && position.active_transfer_sequence == 1,
        )?;

        let replay = self.allocate_ix(keys.verifier.pubkey(), "A", 1, first.bytes.clone())?;
        self.step(
            Spec {
                id: "N02",
                title: "Replay of Allocate A's exact canonical bytes",
                program: solevm_remote_leg::ID,
                instruction: "process_allocate",
                expect: Expect::Rejected(RemoteLegError::InvalidSequence),
                accounts: vec![
                    ("transport_verifier", keys.verifier.pubkey()),
                    ("consumed_message", ns.consumed(1)),
                ],
            },
            &[replay],
            &[&keys.verifier],
        )?;

        let lane = self.state()?.lane()?.clone();
        let now = self.cluster_time()?;
        let next = lane
            .highest_consumed_sequence
            .checked_add(1)
            .ok_or("sequence overflow")?;
        let forged = messages::allocate(
            &self.domain,
            next,
            lane.message_commitment,
            now,
            ns.transfer_id("X"),
            FIRST,
        )?;
        let forged = self.allocate_ix(keys.attacker.pubkey(), "X", next, forged.bytes)?;
        let out = self.step(
            Spec {
                id: "N03",
                title: "Valid next Allocate signed by an attacker posing as the transport verifier",
                program: solevm_remote_leg::ID,
                instruction: "process_allocate",
                expect: Expect::Rejected(RemoteLegError::Unauthorized),
                accounts: vec![("forged_transport_verifier", keys.attacker.pubkey())],
            },
            &[forged],
            &[&keys.attacker],
        )?;
        self.check(
            "no transfer record exists for the forged allocation",
            out.after.transfer("X").is_none(),
        )?;

        self.deposit(
            "T02",
            "Bridge stand-in delivers allocation A's test tokens to custody",
            FIRST,
        )?;

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
                title: "Permissionless reconcile classifies the arrival as unattributed custody",
                program: solevm_remote_leg::ID,
                instruction: "reconcile_custody",
                expect: Expect::Success,
                accounts: vec![
                    ("custody_token_account", ns.custody),
                    ("remote_position", ns.position),
                ],
            },
            &[reconcile],
            &[],
        )?;
        let reconciled: CustodyReconciled =
            event(&out.landed.logs).ok_or("no CustodyReconciled event")?;
        self.detail(json!({ "observed_surplus": reconciled.observed_surplus }))?;
        self.check(
            format!("observed surplus and unattributed custody are both {FIRST}"),
            reconciled.observed_surplus == FIRST
                && out.after.position()?.unattributed_custody == FIRST,
        )?;

        let out = self.attribute(
            "T04",
            "Attribute the arrival to allocation A, completing it",
            "A",
        )?;
        let position = out.after.position()?;
        let record = out.after.transfer("A").ok_or("transfer A is missing")?;
        self.check(
            format!("A is complete with {FIRST} attributed and no unattributed custody left"),
            record.status == TransferStatus::Complete
                && record.attributed_amount == FIRST
                && position.attributed_principal == FIRST
                && position.unattributed_custody == 0
                && position.active_transfer_kind == TransferKind::None
                && position.latest_completed_transfer_id == ns.transfer_id("A"),
        )?;

        self.report(
            "T05",
            "Publish RemoteReport epoch 1 on the outbound lane",
            1,
            FIRST,
        )?;
        Ok(())
    }

    // Close with partial arrival, then recovery

    fn close_and_recover(&mut self) -> Res<()> {
        let keys = self.keys;
        let ns = self.ns.clone();

        self.allocate("T06", "Authenticated Allocate B (sequence 2)", "B", SECOND)?;
        self.deposit(
            "T07",
            "Only part of allocation B arrives in custody",
            SECOND_ARRIVED,
        )?;
        let out = self.attribute("T08", "Attribute the partial arrival to allocation B", "B")?;
        let record = out.after.transfer("B").ok_or("transfer B is missing")?;
        self.check(
            format!("B stays pending with {SECOND_ARRIVED} of {SECOND} attributed"),
            record.status == TransferStatus::Pending
                && record.attributed_amount == SECOND_ARRIVED
                && out.after.position()?.attributed_principal == PRINCIPAL_AFTER_CLOSE,
        )?;

        let close_sequence = self.next_inbound_sequence()?;
        let close = self.resolve_message("B", close_sequence, 2, ResolveAction::Close)?;
        let out = self.settle(
            Spec {
                id: "T09",
                title: "Resolve(Close) of allocation B after a partial arrival",
                program: solevm_remote_leg::ID,
                instruction: "process_settlement",
                expect: Expect::Success,
                accounts: vec![
                    ("transport_verifier", keys.verifier.pubkey()),
                    ("transport_ledger", ns.ledger),
                ],
            },
            "B",
            close_sequence,
            &close,
            None,
            &[&keys.verifier],
        )?;
        let (processed, body) = settlement(&out.landed)?;
        self.check(format!("settlement is Closed: claim {SECOND}, credited {SECOND_ARRIVED}, impaired {SECOND_SHORTFALL}"),
            body.kind == SettlementKind::Closed
                && body.transfer_id.to_bytes() == ns.transfer_id("B")
                && body.claim_amount.get() == u128::from(SECOND)
                && body.credited_amount.get() == u128::from(SECOND_ARRIVED)
                && body.impaired_outstanding.get() == u128::from(SECOND_SHORTFALL)
                && processed.target_sequence == 2
                && !processed.guardian_cosigned)?;
        let ledger = out.after.ledger()?;
        self.check(format!("ledger: claim {SECOND} = credit {SECOND_ARRIVED} + impaired {SECOND_SHORTFALL} + recovered 0"),
            ledger.cumulative_closed_claim == u128::from(SECOND)
                && ledger.cumulative_closed_credit == u128::from(SECOND_ARRIVED)
                && ledger.impaired_outstanding == u128::from(SECOND_SHORTFALL)
                && ledger.cumulative_recovered == 0
                && ledger.last_impaired_sequence == 2
                && ledger.total_closed == 1
                && out.after.ledger_balanced()?)?;
        let record = out.after.transfer("B").ok_or("transfer B is missing")?;
        self.check(
            "B is closed with only its arrival attributed, and no cycle is open",
            record.status == TransferStatus::Closed
                && record.attributed_amount == SECOND_ARRIVED
                && out.after.position()?.active_transfer_kind == TransferKind::None
                && out.after.custody_explained()?,
        )?;

        self.deposit("T10", "The rest of B's assets arrive after the close", LATE)?;

        let recover_sequence = self.next_inbound_sequence()?;
        let recover = self.resolve_message("R", recover_sequence, 2, ResolveAction::Recover)?;
        let out = self.settle(
            Spec {
                id: "T11",
                title: "Resolve(Recover) credits the late arrival against the impairment pool",
                program: solevm_remote_leg::ID,
                instruction: "process_settlement",
                expect: Expect::Success,
                accounts: vec![
                    ("transport_verifier", keys.verifier.pubkey()),
                    ("recovery_record", ns.transfer_record(&ns.transfer_id("R"))),
                ],
            },
            "R",
            recover_sequence,
            &recover,
            None,
            &[&keys.verifier],
        )?;
        let (_, body) = settlement(&out.landed)?;
        self.check(format!("settlement is Recovered: claim {SECOND_SHORTFALL}, credited {LATE}, impaired {IMPAIRED_AFTER_RECOVERY}"),
            body.kind == SettlementKind::Recovered
                && body.claim_amount.get() == u128::from(SECOND_SHORTFALL)
                && body.credited_amount.get() == u128::from(LATE)
                && body.impaired_outstanding.get() == u128::from(IMPAIRED_AFTER_RECOVERY))?;
        let ledger = out.after.ledger()?;
        self.check(format!("impairment fell {SECOND_SHORTFALL} -> {IMPAIRED_AFTER_RECOVERY}; claim {SECOND} = 600000 + 100000 + 300000"),
            ledger.impaired_outstanding == u128::from(IMPAIRED_AFTER_RECOVERY)
                && ledger.cumulative_recovered == u128::from(LATE)
                && ledger.total_recoveries == 1
                && out.after.ledger_balanced()?)?;
        let closed = out.after.transfer("B").ok_or("transfer B is missing")?;
        let recovery = out
            .after
            .transfer("R")
            .ok_or("recovery record is missing")?;
        self.check(
            "no double count: B keeps its 600000, the recovery record holds the 300000",
            closed.status == TransferStatus::Closed
                && closed.attributed_amount == SECOND_ARRIVED
                && recovery.transfer_kind == TransferKind::Recover
                && recovery.status == TransferStatus::Complete
                && recovery.attributed_amount == LATE
                && out.after.position()?.attributed_principal == PRINCIPAL_AFTER_RECOVERY
                && out.after.position()?.unattributed_custody == 0
                && out.after.custody_explained()?,
        )?;

        let replay = self.settlement_ix(
            keys.verifier.pubkey(),
            "R",
            recover_sequence,
            recover.bytes.clone(),
            None,
        )?;
        self.step(
            Spec {
                id: "N04",
                title: "Replay of the Recover message cannot credit the pool twice",
                program: solevm_remote_leg::ID,
                instruction: "process_settlement",
                expect: Expect::Rejected(RemoteLegError::InvalidSequence),
                accounts: vec![("transport_ledger", ns.ledger)],
            },
            &[replay],
            &[&keys.verifier],
        )?;

        self.report(
            "T12",
            "Publish RemoteReport epoch 2 after the close and recovery",
            2,
            PRINCIPAL_AFTER_RECOVERY,
        )?;
        Ok(())
    }

    // Frozen leg and the guardian

    fn frozen_guardian_path(&mut self) -> Res<()> {
        let keys = self.keys;
        let ns = self.ns.clone();

        let (out, _) = self.allocate("T13", "Authenticated Allocate C (sequence 5)", "C", THIRD)?;
        let sequence_c = out.after.position()?.active_transfer_sequence;

        let freeze = self.leg(
            solevm_remote_leg::accounts::FreezeRemoteLeg {
                authority: keys.guardian.pubkey(),
                remote_config: ns.config,
            },
            solevm_remote_leg::instruction::FreezeRemoteLeg {},
        );
        let out = self.step(
            Spec {
                id: "T14",
                title: "Emergency guardian freezes the leg",
                program: solevm_remote_leg::ID,
                instruction: "freeze_remote_leg",
                expect: Expect::Success,
                accounts: vec![
                    ("emergency_guardian", keys.guardian.pubkey()),
                    ("remote_config", ns.config),
                ],
            },
            &[freeze],
            &[&keys.guardian],
        )?;
        let frozen: RemoteLegFrozen = event(&out.landed.logs).ok_or("no RemoteLegFrozen event")?;
        self.check(
            "leg is frozen and the event names the guardian",
            out.after.config()?.frozen && frozen.authority == keys.guardian.pubkey(),
        )?;

        let close_sequence = self.next_inbound_sequence()?;
        let close = self.resolve_message("C", close_sequence, sequence_c, ResolveAction::Close)?;
        let unsigned = self.settlement_ix(
            keys.verifier.pubkey(),
            "C",
            close_sequence,
            close.bytes.clone(),
            None,
        )?;
        self.step(
            Spec {
                id: "N05",
                title: "Close of C on the frozen leg without the guardian",
                program: solevm_remote_leg::ID,
                instruction: "process_settlement",
                expect: Expect::Rejected(RemoteLegError::Frozen),
                accounts: vec![("transport_verifier", keys.verifier.pubkey())],
            },
            &[unsigned],
            &[&keys.verifier],
        )?;

        let impostor = self.settlement_ix(
            keys.verifier.pubkey(),
            "C",
            close_sequence,
            close.bytes.clone(),
            Some(keys.attacker.pubkey()),
        )?;
        self.step(
            Spec {
                id: "N06",
                title: "Close of C on the frozen leg co-signed by an attacker posing as the guardian",
                program: solevm_remote_leg::ID,
                instruction: "process_settlement",
                expect: Expect::Rejected(RemoteLegError::Unauthorized),
                accounts: vec![("forged_emergency_guardian", keys.attacker.pubkey())],
            },
            &[impostor],
            &[&keys.verifier, &keys.attacker],
        )?;

        let out = self.settle(
            Spec {
                id: "T15",
                title: "Guardian co-signed Close of C on the frozen leg (nothing arrived)",
                program: solevm_remote_leg::ID,
                instruction: "process_settlement",
                expect: Expect::Success,
                accounts: vec![
                    ("transport_verifier", keys.verifier.pubkey()),
                    ("emergency_guardian", keys.guardian.pubkey()),
                ],
            },
            "C",
            close_sequence,
            &close,
            Some(&keys.guardian),
            &[&keys.verifier, &keys.guardian],
        )?;
        let (processed, body) = settlement(&out.landed)?;
        self.check(format!("guardian-cosigned Closed settlement: claim {THIRD}, credited 0, impaired {IMPAIRED_AT_END}"),
            processed.guardian_cosigned
                && body.kind == SettlementKind::Closed
                && body.claim_amount.get() == u128::from(THIRD)
                && body.credited_amount.get() == 0
                && body.impaired_outstanding.get() == u128::from(IMPAIRED_AT_END))?;
        let ledger = out.after.ledger()?;
        self.check(
            "final ledger: claims 1500000 = credit 600000 + impaired 600000 + recovered 300000",
            ledger.cumulative_closed_claim == CLOSED_CLAIMS_AT_END
                && ledger.cumulative_closed_credit == CLOSED_CREDIT_AT_END
                && ledger.impaired_outstanding == u128::from(IMPAIRED_AT_END)
                && ledger.cumulative_recovered == u128::from(LATE)
                && ledger.total_closed == 2
                && out.after.ledger_balanced()?,
        )?;
        self.check(
            "C is closed and custody still equals attributed principal",
            out.after
                .transfer("C")
                .is_some_and(|record| record.status == TransferStatus::Closed)
                && out.after.position()?.attributed_principal == PRINCIPAL_AFTER_RECOVERY
                && out.after.custody_explained()?,
        )?;
        Ok(())
    }

    fn finalize(&mut self) -> Res<()> {
        let chain = self.chain;
        let signatures: Vec<String> = self
            .evidence
            .steps
            .iter()
            .map(|step| step.signature.clone())
            .collect();
        println!("waiting for {} transactions to finalize", signatures.len());
        chain.wait_all(&signatures, "finalized", FINALIZE_TIMEOUT)?;
        for step in &mut self.evidence.steps {
            let landed = chain
                .transaction(&step.signature, "finalized")?
                .ok_or_else(|| format!("{} is finalized but not returned", step.signature))?;
            if landed.slot != step.slot {
                return Err(format!(
                    "{} moved from slot {} to {}",
                    step.signature, step.slot, landed.slot
                )
                .into());
            }
            step.confirmation = String::from("finalized");
            step.block_time_utc = landed.block_time.map(utc).unwrap_or_default();
        }
        self.evidence.summary.all_finalized = true;
        self.evidence.final_state = self.state()?.json();
        Ok(())
    }

    fn summarize(&mut self, completed: bool) {
        let steps = &self.evidence.steps;
        let checks = steps.iter().flat_map(|step| step.checks.iter());
        self.evidence.summary.transactions = steps.len();
        self.evidence.summary.expected_successes = steps
            .iter()
            .filter(|step| step.expected == "success")
            .count();
        self.evidence.summary.expected_failures = steps
            .iter()
            .filter(|step| step.expected != "success")
            .count();
        self.evidence.summary.checks = checks.clone().count();
        self.evidence.summary.checks_passed = checks.filter(|check| check.passed).count();
        self.evidence.summary.completed = completed;
        self.evidence.completed_at_utc = now_utc();
    }

    fn write(&self) -> Res<PathBuf> {
        let dir = self.settings.state_dir(&self.evidence.cluster);
        DirBuilder::new().recursive(true).mode(0o700).create(&dir)?;
        let path = dir.join(format!("evidence-{}.json", self.ns.label));
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
        let rejected = matches!(spec.expect, Expect::Rejected(_));
        let landed = self
            .chain
            .execute(instructions, &self.keys.deployer, signers, rejected)?;
        self.last_slot = self.last_slot.max(landed.slot);
        let after = self.state()?;

        let expected = match &spec.expect {
            Expect::Success => String::from("success"),
            Expect::Rejected(error) => format!("rejected: {error:?} ({})", code_of(*error)),
        };
        let observed = match (&landed.error, landed.custom_code()) {
            (None, _) => String::from("success"),
            (Some(_), Some(code)) => format!(
                "rejected: {} ({code})",
                landed
                    .logged_error()
                    .unwrap_or_else(|| String::from("custom error"))
            ),
            (Some(error), None) => format!("rejected: {error}"),
        };
        let matched = match &spec.expect {
            Expect::Success => landed.error.is_none(),
            Expect::Rejected(error) => landed.custom_code() == Some(code_of(*error)),
        };
        self.evidence.steps.push(Step {
            id: spec.id.to_owned(),
            title: spec.title.to_owned(),
            program: spec.program.to_string(),
            instruction: spec.instruction.to_owned(),
            expected: expected.clone(),
            outcome: observed,
            error_code: landed.custom_code(),
            signature: landed.signature.clone(),
            slot: landed.slot,
            block_time_utc: landed.block_time.map(utc).unwrap_or_default(),
            confirmation: String::from("confirmed"),
            compute_units: landed.compute_units,
            fee_lamports: landed.fee,
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
        self.check(format!("on-chain outcome is {expected}"), matched)?;
        if rejected {
            self.check(
                "every tracked leg account and token balance is byte-for-byte unchanged",
                before.raw == after.raw,
            )?;
        }
        Ok(Outcome {
            landed,
            before,
            after,
        })
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
        step.details = details;
        Ok(())
    }

    // Composite steps

    fn allocate(
        &mut self,
        id: &'static str,
        title: &'static str,
        name: &str,
        amount: u64,
    ) -> Res<(Outcome, Encoded)> {
        let keys = self.keys;
        let lane = self.state()?.lane()?.clone();
        let sequence = lane
            .highest_consumed_sequence
            .checked_add(1)
            .ok_or("sequence overflow")?;
        let now = self.cluster_time()?;
        let transfer_id = self.ns.transfer_id(name);
        let message = messages::allocate(
            &self.domain,
            sequence,
            lane.message_commitment,
            now,
            transfer_id,
            amount,
        )?;
        let instruction = self.allocate_ix(
            keys.verifier.pubkey(),
            name,
            sequence,
            message.bytes.clone(),
        )?;
        let record = self.ns.transfer_record(&transfer_id);
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
                    ("consumed_message", self.ns.consumed(sequence)),
                ],
            },
            &[instruction],
            &[&keys.verifier],
        )?;
        let accepted: AllocateAccepted =
            event(&out.landed.logs).ok_or("no AllocateAccepted event")?;
        self.detail(json!({
            "sequence": sequence,
            "transfer_id": evidence::hex(&transfer_id),
            "message_id": evidence::hex(&message.id),
            "message_len": message.bytes.len(),
            "lane_commitment": evidence::hex(&accepted.lane_commitment),
        }))?;
        self.check(format!("AllocateAccepted carries sequence {sequence}, {amount} authorized and the canonical message id"),
            accepted.sequence == sequence && accepted.message_id == message.id && accepted.authorized_amount == amount)?;
        let lane_after = out.after.lane()?;
        self.check(format!("control lane at sequence {sequence} with commitment = keccak(domain, previous, message id)"),
            lane_after.highest_consumed_sequence == sequence
                && lane_after.message_commitment == messages::next_commitment(lane.message_commitment, message.id))?;
        let record = out
            .after
            .transfer(name)
            .ok_or("transfer record is missing")?;
        self.check(
            format!("transfer {name} is pending with {amount} authorized at sequence {sequence}"),
            record.status == TransferStatus::Pending
                && record.authorized_amount == amount
                && record.message_sequence == sequence,
        )?;
        let consumed = self
            .chain
            .account(&self.ns.consumed(sequence), "confirmed")?;
        self.check(
            format!("consumed-message record for sequence {sequence} exists"),
            consumed.is_some_and(|account| account.owner == solevm_remote_leg::ID),
        )?;
        Ok((out, message))
    }

    fn deposit(&mut self, id: &'static str, title: &'static str, amount: u64) -> Res<Outcome> {
        let keys = self.keys;
        let ns = self.ns.clone();
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
        let before = out.before.custody()?;
        self.check(
            format!("custody balance rose by exactly {amount}"),
            before.checked_add(amount) == Some(out.after.custody()?),
        )?;
        self.check(
            "leg accounting is untouched until the leg itself observes the arrival",
            out.before.position()?.attributed_principal
                == out.after.position()?.attributed_principal
                && out.before.position()?.unattributed_custody
                    == out.after.position()?.unattributed_custody,
        )?;
        Ok(out)
    }

    fn attribute(&mut self, id: &'static str, title: &'static str, name: &str) -> Res<Outcome> {
        let ns = self.ns.clone();
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
                accounts: vec![
                    ("transfer_record", record),
                    ("custody_token_account", ns.custody),
                ],
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

    fn report(
        &mut self,
        id: &'static str,
        title: &'static str,
        epoch: u64,
        principal: u64,
    ) -> Res<()> {
        let ns = self.ns.clone();
        let lane = self.state()?.outbound()?.clone();
        let sequence = lane.next_sequence;
        let record_key = ns.report_record(sequence);
        let instruction = self.leg(
            solevm_remote_leg::accounts::PublishRemoteReport {
                payer: self.keys.deployer.pubkey(),
                remote_config: ns.config,
                risk_config: ns.risk_config,
                strategy_config: ns.strategy_config,
                remote_position: ns.position,
                outbound_lane: ns.outbound_lane,
                custody_token_account: ns.custody,
                adapter_state: ns.adapter_state,
                report_record: record_key,
                system_program: system_program::ID,
                active_transfer_record: None,
            },
            solevm_remote_leg::instruction::PublishRemoteReport { epoch_id: epoch },
        );
        let out = self.step(
            Spec {
                id,
                title,
                program: solevm_remote_leg::ID,
                instruction: "publish_remote_report",
                expect: Expect::Success,
                accounts: vec![
                    ("outbound_lane", ns.outbound_lane),
                    ("report_record", record_key),
                ],
            },
            &[instruction],
            &[],
        )?;
        let published: RemoteReportPublished =
            event(&out.landed.logs).ok_or("no RemoteReportPublished event")?;
        let message = protocol_types::decode_message(&published.message_bytes)
            .map_err(|error| format!("report decode: {error:?}"))?;
        let Body::RemoteReport(body) = &message.body else {
            return Err("published report is not a RemoteReport".into());
        };
        let reencoded = message
            .message_id()
            .map_err(|error| format!("{error:?}"))?
            .to_bytes();
        self.detail(json!({
            "sequence": sequence,
            "epoch_id": body.epoch_id.get(),
            "remote_principal": body.remote_principal.get().to_string(),
            "reported_value": body.reported_value.get().to_string(),
            "message_id": evidence::hex(&published.message_id),
            "previous_commitment": evidence::hex(&published.previous_commitment),
            "next_commitment": evidence::hex(&published.next_commitment),
            "message_hex": evidence::hex(&published.message_bytes),
        }))?;
        self.check(
            "the published bytes decode canonically and re-encode to the same message id",
            reencoded == published.message_id,
        )?;
        self.check(
            format!("report is sequence {sequence} and chains from the lane's previous commitment"),
            message.header.sequence.get() == sequence
                && published.sequence == sequence
                && *message.header.previous_commitment.as_bytes() == lane.current_commitment
                && published.previous_commitment == lane.current_commitment,
        )?;
        let lane_after = out.after.outbound()?;
        self.check("lane commitment advanced to keccak(domain, previous, message id) and the next sequence by one",
            messages::next_commitment(lane.current_commitment, published.message_id) == published.next_commitment
                && lane_after.current_commitment == published.next_commitment
                && Some(lane_after.next_sequence) == sequence.checked_add(1))?;
        self.check(
            format!("report states epoch {epoch} and remote principal {principal}"),
            body.epoch_id.get() == epoch && body.remote_principal.get() == u128::from(principal),
        )?;
        let stored: Option<ReportRecord> = self.load(&record_key)?;
        self.check(
            "report record stores the type, message id and commitments",
            stored.is_some_and(|record| {
                record.report_type == ReportType::RemoteReport
                    && record.message_id == published.message_id
                    && record.previous_commitment == lane.current_commitment
                    && record.next_commitment == published.next_commitment
            }),
        )?;
        Ok(())
    }

    /// Sends one settlement and checks its publication on the outbound lane.
    fn settle(
        &mut self,
        spec: Spec,
        name: &str,
        sequence: u64,
        message: &Encoded,
        guardian: Option<&Keypair>,
        signers: &[&Keypair],
    ) -> Res<Outcome> {
        let lane = self.state()?.outbound()?.clone();
        let instruction = self.settlement_ix(
            self.keys.verifier.pubkey(),
            name,
            sequence,
            message.bytes.clone(),
            guardian.map(Signer::pubkey),
        )?;
        let out = self.step(spec, &[instruction], signers)?;
        let published: TransportSettlementPublished =
            event(&out.landed.logs).ok_or("no TransportSettlementPublished event")?;
        let processed: SettlementProcessed =
            event(&out.landed.logs).ok_or("no SettlementProcessed event")?;
        let decoded = protocol_types::decode_message(&published.message_bytes)
            .map_err(|error| format!("settlement decode: {error:?}"))?;
        let reencoded = decoded
            .message_id()
            .map_err(|error| format!("{error:?}"))?
            .to_bytes();
        self.detail(json!({
            "inbound_sequence": sequence,
            "inbound_message_id": evidence::hex(&message.id),
            "kind": processed.kind,
            "claim_amount": processed.claim_amount.to_string(),
            "credited_amount": processed.credited_amount,
            "impaired_outstanding": processed.impaired_outstanding.to_string(),
            "guardian_cosigned": processed.guardian_cosigned,
            "outbound_sequence": published.sequence,
            "outbound_message_id": evidence::hex(&published.message_id),
            "outbound_message_hex": evidence::hex(&published.message_bytes),
        }))?;
        let inbound = out.after.lane()?;
        self.check(
            format!("Resolve consumed inbound sequence {sequence} on the shared control lane"),
            inbound.highest_consumed_sequence == sequence
                && inbound.message_commitment
                    == messages::next_commitment(out.before.lane()?.message_commitment, message.id),
        )?;
        let outbound = out.after.outbound()?;
        self.check("TransportSettlement is published in the same instruction, chained on the outbound lane",
            reencoded == published.message_id
                && published.sequence == lane.next_sequence
                && published.previous_commitment == lane.current_commitment
                && messages::next_commitment(lane.current_commitment, published.message_id) == published.next_commitment
                && outbound.current_commitment == published.next_commitment
                && Some(outbound.next_sequence) == lane.next_sequence.checked_add(1))?;
        Ok(out)
    }

    // Instructions

    fn leg(&self, accounts: impl ToAccountMetas, data: impl InstructionData) -> Instruction {
        Instruction {
            program_id: solevm_remote_leg::ID,
            accounts: accounts.to_account_metas(None),
            data: data.data(),
        }
    }

    fn init_params(&self) -> InitializeParams {
        InitializeParams {
            deployment_id: self.ns.deployment_id,
            vault_id: self.ns.vault_id,
            source_chain_id: SOURCE_CHAIN_ID,
            destination_chain_id: DESTINATION_CHAIN_ID,
            source_application_id: self.domain.source_application,
            local_application_id: self.domain.local_application,
            control_lane_id: CONTROL_LANE_ID,
            report_lane_id: REPORT_LANE_ID,
            config_version: CONFIG_VERSION,
            transport_verifier: self.keys.verifier.pubkey(),
            emergency_guardian: self.keys.guardian.pubkey(),
        }
    }

    fn control_params(&self) -> ControlStateParams {
        ControlStateParams {
            max_remote_allocation_bps: 6_000,
            max_upward_deviation_bps: 200,
            max_downward_deviation_bps: 1_000,
            max_report_age: 3_600,
            config_commitment: protocol_types::keccak256(
                format!("solevm/devnet/{}/config/1", self.ns.label).as_bytes(),
            ),
            mandatory_watermark_lag: 2,
        }
    }

    fn initialize_ix(&self, upgrade_authority: Pubkey) -> Instruction {
        self.leg(
            solevm_remote_leg::accounts::InitializeRemoteLeg {
                administrator: self.keys.administrator.pubkey(),
                remote_config: self.ns.config,
                asset_mint: self.ns.mint,
                custody_token_account: self.ns.custody,
                outbound_escrow: self.ns.escrow,
                token_program: spl_token::ID,
                system_program: system_program::ID,
                upgrade_authority,
                program_data: self.ns.program_data,
            },
            solevm_remote_leg::instruction::InitializeRemoteLeg {
                params: self.init_params(),
            },
        )
    }

    fn allocate_ix(
        &self,
        verifier: Pubkey,
        name: &str,
        sequence: u64,
        bytes: Vec<u8>,
    ) -> Res<Instruction> {
        let transfer_id = self.ns.transfer_id(name);
        Ok(self.leg(
            solevm_remote_leg::accounts::ProcessAllocate {
                transport_verifier: verifier,
                remote_config: self.ns.config,
                risk_config: self.ns.risk_config,
                strategy_config: self.ns.strategy_config,
                remote_position: self.ns.position,
                control_lane: self.ns.control_lane,
                transfer_record: self.ns.transfer_record(&transfer_id),
                consumed_message: self.ns.consumed(sequence),
                system_program: system_program::ID,
            },
            solevm_remote_leg::instruction::ProcessAllocate {
                message_bytes: bytes,
            },
        ))
    }

    fn settlement_ix(
        &self,
        verifier: Pubkey,
        name: &str,
        sequence: u64,
        bytes: Vec<u8>,
        guardian: Option<Pubkey>,
    ) -> Res<Instruction> {
        let report_sequence = self.state()?.outbound()?.next_sequence;
        let transfer_id = self.ns.transfer_id(name);
        Ok(self.leg(
            solevm_remote_leg::accounts::ProcessSettlement {
                transport_verifier: verifier,
                remote_config: self.ns.config,
                risk_config: self.ns.risk_config,
                strategy_config: self.ns.strategy_config,
                remote_position: self.ns.position,
                control_lane: self.ns.control_lane,
                outbound_lane: self.ns.outbound_lane,
                transfer_record: self.ns.transfer_record(&transfer_id),
                transport_ledger: self.ns.ledger,
                custody_token_account: self.ns.custody,
                consumed_message: self.ns.consumed(sequence),
                report_record: self.ns.report_record(report_sequence),
                system_program: system_program::ID,
                emergency_guardian: guardian,
            },
            solevm_remote_leg::instruction::ProcessSettlement {
                message_bytes: bytes,
            },
        ))
    }

    fn resolve_message(
        &self,
        name: &str,
        sequence: u64,
        target: u64,
        action: ResolveAction,
    ) -> Res<Encoded> {
        let lane = self.state()?.lane()?.clone();
        let now = self.cluster_time()?;
        messages::resolve(
            &self.domain,
            sequence,
            lane.message_commitment,
            now,
            self.ns.transfer_id(name),
            target,
            action,
        )
    }

    fn next_inbound_sequence(&self) -> Res<u64> {
        self.state()?
            .lane()?
            .highest_consumed_sequence
            .checked_add(1)
            .ok_or_else(|| "sequence overflow".into())
    }

    // Reads

    /// The cluster clock the program reads, from the Clock sysvar.
    fn cluster_time(&self) -> Res<u64> {
        let clock = self
            .chain
            .account(&CLOCK_SYSVAR, "confirmed")?
            .ok_or("clock sysvar is missing")?;
        let bytes = clock.data.get(32..40).ok_or("clock sysvar is too short")?;
        let mut raw = [0u8; 8];
        raw.copy_from_slice(bytes);
        u64::try_from(i64::from_le_bytes(raw)).map_err(|_| "cluster time is negative".into())
    }

    fn load<T: AccountDeserialize>(&self, key: &Pubkey) -> Res<Option<T>> {
        let (_, mut accounts) = self.chain.accounts(&[*key], Some(self.last_slot))?;
        accounts
            .pop()
            .flatten()
            .map(|account| decode(&account.data))
            .transpose()
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
        let mut keys = vec![
            ns.config,
            ns.risk_config,
            ns.control_lane,
            ns.outbound_lane,
            ns.strategy_config,
            ns.position,
            ns.ledger,
            ns.custody,
            ns.transport_source,
        ];
        keys.extend(self.transfers.iter().map(|(_, id)| ns.transfer_record(id)));
        let (slot, accounts) = self.chain.accounts(&keys, Some(self.last_slot))?;
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
            let index = offset.checked_add(9).ok_or("index overflow")?;
            transfers.push((name.clone(), decode_at(&data, index)?));
        }
        Ok(State {
            slot,
            config: decode_at(&data, 0)?,
            lane: decode_at(&data, 2)?,
            outbound: decode_at(&data, 3)?,
            strategy: decode_at(&data, 4)?,
            position: decode_at(&data, 5)?,
            ledger: decode_at(&data, 6)?,
            custody: token(7)?,
            source: token(8)?,
            transfers,
            raw: data.clone(),
        })
    }
}

pub(crate) fn decode_at<T: AccountDeserialize>(
    data: &[Option<Vec<u8>>],
    index: usize,
) -> Res<Option<T>> {
    data.get(index)
        .and_then(Option::as_ref)
        .map(|bytes| decode(bytes))
        .transpose()
}

pub(crate) fn decode<T: AccountDeserialize>(data: &[u8]) -> Res<T> {
    let mut slice = data;
    T::try_deserialize(&mut slice).map_err(|error| format!("account decode: {error:?}").into())
}

/// The last event of type `T` in a transaction's logs.
pub(crate) fn event<T: Discriminator + AnchorDeserialize>(logs: &[String]) -> Option<T> {
    logs.iter()
        .filter_map(|line| line.strip_prefix("Program data: "))
        .filter_map(|payload| BASE64.decode(payload).ok())
        .filter_map(|decoded| {
            let (tag, body) = decoded.split_at_checked(8)?;
            if tag != T::DISCRIMINATOR {
                return None;
            }
            T::try_from_slice(body).ok()
        })
        .next_back()
}

fn settlement(
    landed: &Landed,
) -> Res<(SettlementProcessed, protocol_types::TransportSettlementBody)> {
    let processed: SettlementProcessed =
        event(&landed.logs).ok_or("no SettlementProcessed event")?;
    let published: TransportSettlementPublished =
        event(&landed.logs).ok_or("no TransportSettlementPublished event")?;
    let message = protocol_types::decode_message(&published.message_bytes)
        .map_err(|error| format!("settlement decode: {error:?}"))?;
    let Body::TransportSettlement(body) = message.body else {
        return Err("published settlement is not a TransportSettlement".into());
    };
    Ok((processed, body))
}

/// The number Anchor reports for one program error.
pub(crate) fn code_of(error: RemoteLegError) -> u32 {
    (error as u32).saturating_add(anchor_lang::error::ERROR_CODE_OFFSET)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn v1_commands_refuse_the_v2_build() {
        let error = require_v1_build()
            .err()
            .map(|error| error.to_string())
            .unwrap_or_default();
        assert!(error.contains("V2 devnet leg"), "{error}");
        assert!(error.contains(&V1_LEG_ID.to_string()), "{error}");
    }
}
