//! The EIP-8037 state gas pricing hook through [`OpHandler`].
//!
//! `OpHandler` owns one state gas call site: the transaction-level refund of the EIP-2780
//! first-frame charge in `last_frame_result`. The transactions run on a context that delegates
//! everything to [`OpContext`] except [`Host::state_gas_price`], which prices chosen sites away
//! from the flat schedule and can be told to fail, as `crates/handler/tests/state_gas_price.rs` of
//! `megaeth-labs/revm` does for the mainnet handler. Each priced test fails if the refund reads the
//! schedule directly; each failure test checks that the cause the hook recorded is what the
//! transaction returns.

use op_revm::{
    DefaultOp, L1BlockInfo, OpContext, OpEvm, OpHaltReason, OpSpecId, OpTransaction,
    OpTransactionError, handler::OpHandler,
};
use revm::{
    Context, ExecuteEvm,
    bytecode::opcode::{PUSH0, REVERT},
    context::{
        BlockEnv, CfgEnv, ContextError, ContextSetters, Journal, LocalContext, TxEnv,
        result::{EVMError, ExecutionResult},
    },
    context_interface::{
        ContextTr, Database, Host,
        cfg::{GasId, GasParams, StateGasSite, gas::GasTracker},
        host::LoadError,
        journaled_state::AccountInfoLoad,
        result::ResultGas,
    },
    database::InMemoryDB,
    handler::{EthFrame, FrameResult, Handler},
    interpreter::{
        CreateOutcome, Gas, InstructionResult, InterpreterResult, SStoreResult, SelfDestructResult,
        StateLoad, interpreter::EthInterpreter,
    },
    primitives::{
        Address, B256, Bytes, HashMap, Log, StorageKey, StorageValue, TxKind, U256, address,
        eip7825::TX_GAS_LIMIT_CAP, hardfork::SpecId,
    },
    state::AccountInfo,
};
use rstest::rstest;

type Db = InMemoryDB;
type DbError = <Db as Database>::Error;
type Outcome = Result<ExecutionResult<OpHaltReason>, EVMError<DbError, OpTransactionError>>;

/// Sender of every transaction.
const CALLER: Address = address!("0x0000000000000000000000000000000000ca11e7");

/// Gas limit below the EIP-7825 cap: the reservoir is empty and the charge spills onto the
/// regular budget.
const GAS_LIMIT: u64 = 10_000_000;
/// Gas limit above the EIP-7825 cap by [`PRICE`]: the reservoir pays the charge.
const RESERVOIR_GAS_LIMIT: u64 = TX_GAS_LIMIT_CAP + PRICE;

/// A price no flat schedule entry has.
const PRICE: u64 = 700_000;

/// What the failing hook records before it returns `None`.
const POISON_CAUSE: &str = "injected state gas price failure";
/// What a second failing lookup records, so the test can tell which failure is returned.
const OTHER_POISON_CAUSE: &str = "second injected state gas price failure";

/// The blake2f precompile, which fails on empty input and has no account.
const BLAKE2F: Address = address!("0x0000000000000000000000000000000000000009");

/// One state gas price lookup: which price, at which site.
type Lookup = (GasId, StateGasSite);
/// A lookup with what the hook answered.
type Answered = (Lookup, Option<u64>);

/// A lookup that fails with `cause` once it has been answered `fail_after` times.
struct Poison {
    lookup: Lookup,
    fail_after: usize,
    cause: &'static str,
}

/// [`OpContext`] with a per-site state gas price table.
struct PricedContext {
    inner: OpContext<Db>,
    /// Unit prices that differ from the flat schedule.
    prices: HashMap<Lookup, u64>,
    poisons: Vec<Poison>,
    /// Every lookup, in order.
    lookups: Vec<Answered>,
}

impl PricedContext {
    fn new() -> Self {
        Self {
            inner: amsterdam(),
            prices: HashMap::default(),
            poisons: Vec::new(),
            lookups: Vec::new(),
        }
    }

    fn with_price(mut self, lookup: Lookup, price: u64) -> Self {
        self.prices.insert(lookup, price);
        self
    }

    fn with_poison(self, lookup: Lookup, fail_after: usize) -> Self {
        self.with_poison_cause(lookup, fail_after, POISON_CAUSE)
    }

    fn with_poison_cause(mut self, lookup: Lookup, fail_after: usize, cause: &'static str) -> Self {
        self.poisons.push(Poison { lookup, fail_after, cause });
        self
    }

    /// What the hook answers for `lookup`: a failure once a poison on it runs out, otherwise the
    /// table's price or the flat one.
    fn answer(&mut self, lookup: Lookup) -> Option<u64> {
        for poison in &mut self.poisons {
            if poison.lookup == lookup {
                if poison.fail_after == 0 {
                    *self.inner.error() = Err(ContextError::Custom(poison.cause.into()));
                    return None;
                }
                poison.fail_after -= 1;
            }
        }
        let (id, site) = lookup;
        self.prices.get(&lookup).copied().or_else(|| self.inner.state_gas_price(id, site))
    }
}

impl ContextTr for PricedContext {
    type Block = BlockEnv;
    type Tx = OpTransaction<TxEnv>;
    type Cfg = CfgEnv<OpSpecId>;
    type Db = Db;
    type Journal = Journal<Db>;
    type Chain = L1BlockInfo;
    type Local = LocalContext;

    fn all(
        &self,
    ) -> (&Self::Block, &Self::Tx, &Self::Cfg, &Self::Db, &Self::Journal, &Self::Chain, &Self::Local)
    {
        self.inner.all()
    }

    fn all_mut(
        &mut self,
    ) -> (&Self::Block, &Self::Tx, &Self::Cfg, &mut Self::Journal, &mut Self::Chain, &mut Self::Local)
    {
        self.inner.all_mut()
    }

    fn error(&mut self) -> &mut Result<(), ContextError<DbError>> {
        self.inner.error()
    }
}

impl ContextSetters for PricedContext {
    fn set_tx(&mut self, tx: Self::Tx) {
        self.inner.set_tx(tx);
    }

    fn set_block(&mut self, block: Self::Block) {
        self.inner.set_block(block);
    }
}

impl Host for PricedContext {
    fn state_gas_price(&mut self, id: GasId, site: StateGasSite) -> Option<u64> {
        let lookup = (id, site);
        let answer = self.answer(lookup);
        self.lookups.push((lookup, answer));
        answer
    }

    fn basefee(&self) -> U256 {
        self.inner.basefee()
    }

    fn blob_gasprice(&self) -> U256 {
        self.inner.blob_gasprice()
    }

    fn gas_limit(&self) -> U256 {
        self.inner.gas_limit()
    }

    fn difficulty(&self) -> U256 {
        self.inner.difficulty()
    }

    fn prevrandao(&self) -> Option<U256> {
        self.inner.prevrandao()
    }

    fn block_number(&self) -> U256 {
        self.inner.block_number()
    }

    fn timestamp(&self) -> U256 {
        self.inner.timestamp()
    }

    fn beneficiary(&self) -> Address {
        self.inner.beneficiary()
    }

    fn slot_num(&self) -> U256 {
        self.inner.slot_num()
    }

    fn chain_id(&self) -> U256 {
        self.inner.chain_id()
    }

    fn effective_gas_price(&self) -> U256 {
        self.inner.effective_gas_price()
    }

    fn caller(&self) -> Address {
        self.inner.caller()
    }

    fn blob_hash(&self, number: usize) -> Option<U256> {
        self.inner.blob_hash(number)
    }

    fn max_initcode_size(&self) -> usize {
        self.inner.max_initcode_size()
    }

    fn gas_params(&self) -> &GasParams {
        self.inner.gas_params()
    }

    fn is_amsterdam_eip8037_enabled(&self) -> bool {
        self.inner.is_amsterdam_eip8037_enabled()
    }

    fn block_hash(&mut self, number: u64) -> Option<B256> {
        self.inner.block_hash(number)
    }

    fn selfdestruct(
        &mut self,
        address: Address,
        target: Address,
        skip_cold_load: bool,
    ) -> Result<StateLoad<SelfDestructResult>, LoadError> {
        self.inner.selfdestruct(address, target, skip_cold_load)
    }

    fn log(&mut self, log: Log) {
        self.inner.log(log)
    }

    fn sstore_skip_cold_load(
        &mut self,
        address: Address,
        key: StorageKey,
        value: StorageValue,
        skip_cold_load: bool,
    ) -> Result<StateLoad<SStoreResult>, LoadError> {
        self.inner.sstore_skip_cold_load(address, key, value, skip_cold_load)
    }

    fn sload_skip_cold_load(
        &mut self,
        address: Address,
        key: StorageKey,
        skip_cold_load: bool,
    ) -> Result<StateLoad<StorageValue>, LoadError> {
        self.inner.sload_skip_cold_load(address, key, skip_cold_load)
    }

    fn tstore(&mut self, address: Address, key: StorageKey, value: StorageValue) {
        self.inner.tstore(address, key, value)
    }

    fn tload(&mut self, address: Address, key: StorageKey) -> StorageValue {
        self.inner.tload(address, key)
    }

    fn load_account_info_skip_cold_load(
        &mut self,
        address: Address,
        load_code: bool,
        skip_cold_load: bool,
    ) -> Result<AccountInfoLoad<'_>, LoadError> {
        self.inner.load_account_info_skip_cold_load(address, load_code, skip_cold_load)
    }
}

/// An OP context on the newest fork with the Amsterdam state gas EIPs (EIP-8037, and EIP-2780,
/// which charges the first frame's new account upfront) and their gas params. [`CALLER`] is
/// funded; the L1 block info is current and prices nothing, so no L1 or operator fee applies.
fn amsterdam() -> OpContext<Db> {
    let mut db = Db::default();
    db.insert_account_info(CALLER, AccountInfo::from_balance(U256::from(10u128.pow(21))));
    let mut cfg = CfgEnv::new_with_spec(OpSpecId::KARST)
        .with_enable_amsterdam_eip8037(true)
        .with_enable_amsterdam_eip2780(true);
    cfg.set_gas_params(GasParams::new_spec(SpecId::AMSTERDAM));
    Context::op().with_db(db).with_cfg(cfg).with_chain(L1BlockInfo {
        l2_block: Some(U256::ZERO),
        operator_fee_scalar: Some(U256::ZERO),
        operator_fee_constant: Some(U256::ZERO),
        ..Default::default()
    })
}

/// A non-deposit transaction from [`CALLER`]. The enveloped transaction is empty, which carries
/// no L1 data fee.
fn tx(kind: TxKind, value: u64, data: &[u8], gas_limit: u64) -> OpTransaction<TxEnv> {
    OpTransaction::builder()
        .base(
            TxEnv::builder()
                .caller(CALLER)
                .kind(kind)
                .value(U256::from(value))
                .data(Bytes::copy_from_slice(data))
                .gas_price(0)
                .gas_limit(gas_limit),
        )
        .enveloped_tx(Some(Bytes::new()))
        .build_fill()
}

/// Initcode that deploys nothing: `REVERT(0, 0)`.
const REVERTING_INITCODE: [u8; 3] = [PUSH0, PUSH0, REVERT];

/// A creation transaction whose initcode reverts.
fn reverting_create_tx(gas_limit: u64) -> OpTransaction<TxEnv> {
    tx(TxKind::Create, 0, &REVERTING_INITCODE, gas_limit)
}

/// A value transfer to [`BLAKE2F`]. The precompile rejects the empty input, so the first frame
/// halts and the recipient leaf is never created.
fn failing_value_transfer_tx(gas_limit: u64) -> OpTransaction<TxEnv> {
    tx(TxKind::Call(BLAKE2F), 1, &[], gas_limit)
}

/// Runs `tx` on `ctx` and returns the outcome with the lookups the hook saw.
fn transact(ctx: PricedContext, tx: OpTransaction<TxEnv>) -> (Outcome, Vec<Answered>) {
    let mut evm = OpEvm::new(ctx, ());
    let outcome = evm.transact(tx).map(|out| out.result);
    (outcome, evm.into_context().lookups)
}

/// Runs `tx` with the flat schedule and returns its gas.
fn flat_gas(tx: OpTransaction<TxEnv>) -> ResultGas {
    let (outcome, _) = transact(PricedContext::new(), tx);
    *outcome.expect("flat-priced transaction runs").gas()
}

const fn new_account(address: Address) -> Lookup {
    (GasId::new_account_state_gas(), StateGasSite::account(address))
}

const fn create_charge(address: Address) -> Lookup {
    (GasId::create_state_gas(), StateGasSite::account(address))
}

fn priced(lookup: Lookup, price: u64) -> PricedContext {
    PricedContext::new().with_price(lookup, price)
}

fn assert_fails_with_recorded_cause<T: core::fmt::Debug>(
    outcome: Result<T, EVMError<DbError, OpTransactionError>>,
) {
    match outcome {
        Err(EVMError::Custom(cause)) => assert_eq!(cause, POISON_CAUSE),
        other => panic!("expected the recorded cause, got {other:?}"),
    }
}

#[rstest]
#[case::charge_spilled_into_regular_gas(GAS_LIMIT)]
#[case::charged_from_reservoir(RESERVOIR_GAS_LIMIT)]
fn test_reverted_creation_tx_refunds_the_created_address_price(#[case] gas_limit: u64) {
    let created = CALLER.create(0);
    let tx = reverting_create_tx(gas_limit);
    let (outcome, lookups) = transact(priced(create_charge(created), PRICE), tx.clone());

    let result = outcome.unwrap();
    assert!(matches!(result, ExecutionResult::Revert { .. }), "{result:?}");
    // The refund cancels the priced charge: no state gas is left, and the transaction costs what
    // it costs on the flat schedule.
    assert_eq!(result.gas().state_gas_spent_final(), 0);
    assert_eq!(*result.gas(), flat_gas(tx));
    // The charge and its refund are the same lookup, answered with the same price.
    assert_eq!(lookups, [(create_charge(created), Some(PRICE)); 2]);
}

#[rstest]
#[case::charge_spilled_into_regular_gas(GAS_LIMIT)]
#[case::charged_from_reservoir(RESERVOIR_GAS_LIMIT)]
fn test_failed_value_transfer_tx_refunds_the_recipient_price(#[case] gas_limit: u64) {
    let tx = failing_value_transfer_tx(gas_limit);
    let (outcome, lookups) = transact(priced(new_account(BLAKE2F), PRICE), tx.clone());

    let result = outcome.unwrap();
    assert!(result.is_halt(), "{result:?}");
    // The halt consumes the regular gas; the refund still cancels the priced charge, and the
    // reservoir ends where it does on the flat schedule.
    assert_eq!(result.gas().state_gas_spent_final(), 0);
    assert_eq!(*result.gas(), flat_gas(tx));
    assert_eq!(lookups, [(new_account(BLAKE2F), Some(PRICE)); 2]);
}

#[rstest]
#[case::reverted_creation(create_charge(CALLER.create(0)), reverting_create_tx(GAS_LIMIT))]
#[case::failed_value_transfer(new_account(BLAKE2F), failing_value_transfer_tx(GAS_LIMIT))]
fn test_refund_lookup_failure_fails_tx_with_the_recorded_cause(
    #[case] lookup: Lookup,
    #[case] tx: OpTransaction<TxEnv>,
) {
    // The charge is priced; the refund, the second lookup of the same site, fails.
    let ctx = priced(lookup, PRICE).with_poison(lookup, 1);
    let (outcome, lookups) = transact(ctx, tx);

    assert_fails_with_recorded_cause(outcome);
    assert_eq!(lookups, [(lookup, Some(PRICE)), (lookup, None)]);
}

#[test]
fn test_recorded_frame_failure_is_returned_before_the_refund_is_priced() {
    // A failed code deposit lookup ends the first frame with `FatalExternalError` and leaves its
    // cause recorded, while the create charge stays refundable; pricing that refund would fail
    // with another cause. No transaction can tell this apart from a handler that never prices the
    // refund (post-execution returns the recorded cause either way), so the settlement is driven
    // directly: it returns the frame's cause, and the refund is never priced.
    let created = CALLER.create(0);
    let mut ctx =
        PricedContext::new().with_poison_cause(create_charge(created), 0, OTHER_POISON_CAUSE);
    ctx.set_tx(reverting_create_tx(GAS_LIMIT));
    *ctx.error() = Err(ContextError::Custom(POISON_CAUSE.into()));

    // The EIP-2780 runtime phase charged the create on the transaction-level gas and forwarded
    // the rest to the frame.
    let mut parent_gas = GasTracker::new(GAS_LIMIT, GAS_LIMIT, 0);
    assert!(parent_gas.record_state_cost(PRICE));
    let gas =
        Gas::new_with_regular_gas_and_reservoir(parent_gas.remaining(), parent_gas.reservoir());
    let mut outcome = CreateOutcome::new(
        InterpreterResult {
            result: InstructionResult::FatalExternalError,
            output: Bytes::new(),
            gas,
        },
        Some(created),
    );
    outcome.charged_create_state_gas = true;
    outcome.charged_state_gas_address = created;
    let mut frame_result = FrameResult::Create(outcome);

    let mut evm = OpEvm::new(ctx, ());
    let mut handler =
        OpHandler::<_, EVMError<DbError, OpTransactionError>, EthFrame<EthInterpreter>>::new();
    let settled = handler.last_frame_result(&mut evm, &mut frame_result, &mut parent_gas);

    assert_fails_with_recorded_cause(settled);
    assert_eq!(evm.into_context().lookups, []);
}
