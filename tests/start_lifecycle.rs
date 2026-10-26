//! The revm fork's start lifecycle through [`OpHandler`], which overrides three of the boundaries
//! that close it: `last_frame_result` closes the transaction's start, `execution_result` ends the
//! lifecycle, and `catch_error` discards it. Each override calls the revm fork's helper where the
//! mainnet method calls it.
//!
//! The tests of the record and of the fatal errors run in every build. The tests that read the
//! events the hooks received need the lifecycle mirror the context keeps in debug builds only.
//!
//! [`PARENT`] calls [`CHILD`] and then writes its slot 0; [`CHILD`] writes its slot 0.

use op_revm::{
    DefaultOp, L1BlockInfo, OpBuilder, OpContext, OpHaltReason, OpSpecId, OpTransaction,
    OpTransactionError, api::builder::DefaultOpEvm, handler::OpHandler,
};
use revm::{
    Context, ExecuteEvm, InspectEvm, Inspector, SystemCallEvm,
    bytecode::opcode::{CALL, INVALID, PUSH0, PUSH1, PUSH3, PUSH20, SSTORE, STOP},
    context::{
        CfgEnv, TxEnv,
        result::{EVMError, ExecutionResult, ResultGas},
    },
    context_interface::{
        ContextSetters, ContextTr, Database, cfg::gas::GasTracker, context::ContextError,
    },
    database::InMemoryDB,
    handler::{EthFrame, EvmTr, FrameResult, Handler, execution::settle_first_frame},
    interpreter::{CallInputs, CallOutcome, interpreter::EthInterpreter},
    primitives::{Address, Bytes, TxKind, U256, address},
    state::{AccountInfo, Bytecode},
};

type Db = InMemoryDB;
type Ctx = OpContext<Db>;
type Error = EVMError<<Db as Database>::Error, OpTransactionError>;

const CALLER: Address = address!("0x1000000000000000000000000000000000000001");
const PARENT: Address = address!("0x00000000000000000000000000000000000c0de0");
const CHILD: Address = address!("0x00000000000000000000000000000000000c0de1");
const GAS_LIMIT: u64 = 1_000_000;

/// Records a fatal error on the context at the call to [`CHILD`] once, as a failed external
/// lookup would.
#[derive(Default)]
struct FailChildOnce(bool);

impl Inspector<Ctx, EthInterpreter> for FailChildOnce {
    fn call(&mut self, ctx: &mut Ctx, inputs: &mut CallInputs) -> Option<CallOutcome> {
        if inputs.target_address == CHILD && !self.0 {
            *ctx.error() = Err(ContextError::Custom("injected".into()));
            self.0 = true;
        }
        None
    }
}

fn account(code: Vec<u8>) -> AccountInfo {
    let code = Bytecode::new_raw(code.into());
    AccountInfo::new(U256::ZERO, 1, code.hash_slow(), code)
}

/// [`PARENT`] runs `parent`, [`CHILD`] writes its slot.
fn context_running(parent: Vec<u8>) -> Ctx {
    let mut db = Db::default();
    db.insert_account_info(CALLER, AccountInfo::from_balance(U256::from(10u128.pow(21))));
    db.insert_account_info(PARENT, account(parent));
    db.insert_account_info(CHILD, account(vec![PUSH1, 2, PUSH0, SSTORE, STOP]));
    Context::op().with_db(db).with_cfg(CfgEnv::new_with_spec(OpSpecId::JOVIAN)).with_chain(
        L1BlockInfo {
            l2_block: Some(U256::ZERO),
            operator_fee_scalar: Some(U256::ZERO),
            operator_fee_constant: Some(U256::ZERO),
            ..Default::default()
        },
    )
}

/// [`PARENT`] calls [`CHILD`] and then writes its slot.
fn context() -> Ctx {
    let mut parent = vec![PUSH0, PUSH0, PUSH0, PUSH0, PUSH0, PUSH20];
    parent.extend_from_slice(CHILD.as_slice());
    parent.extend_from_slice(&[PUSH3, 0x01, 0x00, 0x00, CALL, PUSH1, 1, PUSH0, SSTORE, STOP]);
    context_running(parent)
}

fn tx() -> OpTransaction<TxEnv> {
    OpTransaction::builder()
        .base(
            TxEnv::builder()
                .caller(CALLER)
                .kind(TxKind::Call(PARENT))
                .gas_price(0)
                .gas_limit(GAS_LIMIT),
        )
        .enveloped_tx(Some(Bytes::new()))
        .build_fill()
}

fn deposit() -> OpTransaction<TxEnv> {
    OpTransaction::builder()
        .base(TxEnv::builder().caller(CALLER).kind(TxKind::Call(PARENT)).gas_limit(GAS_LIMIT))
        .is_deposit_tx()
        .build_fill()
}

/// The record is as a transaction's boundary leaves it: no lifecycle under way, nothing open.
fn assert_clear<E: EvmTr>(evm: &mut E) {
    assert!(!evm.frame_stack().lifecycle_active(), "a lifecycle is still under way");
    assert!(evm.frame_stack().lifecycle_settled(), "a start is still open");
}

fn assert_fatal<T: core::fmt::Debug>(result: Result<T, Error>, message: &str) {
    match result {
        Err(EVMError::Custom(error)) => assert!(error.contains(message), "{error}"),
        other => panic!("expected the fatal error {message:?}, got {other:?}"),
    }
}

/// An OP transaction, a deposit and a system call each leave the record clear, so the reused EVM
/// runs the next. (`transact` commits nothing to the database, so each runs at nonce 0.)
#[test]
fn test_each_op_run_leaves_the_record_clear_for_the_next() {
    let mut evm = context().build_op();
    for _ in 0..2 {
        assert!(evm.transact(tx()).expect("runs").result.is_success());
        assert_clear(&mut evm);
        assert!(evm.transact(deposit()).expect("runs").result.is_success());
        assert_clear(&mut evm);
        assert!(evm.system_call_one(PARENT, Bytes::new()).expect("runs").is_success());
        assert_clear(&mut evm);
    }
}

/// A fatal error in a child discards the lifecycle at `catch_error`, and the EVM runs the next
/// transaction.
#[test]
fn test_a_fatal_child_error_discards_the_lifecycle_and_the_evm_is_reused() {
    let mut evm = context().build_op_with_inspector(FailChildOnce::default());
    assert_fatal(evm.inspect_tx(tx()), "injected");
    assert_clear(&mut evm);
    assert!(evm.inspect_tx(tx()).expect("reused").result.is_success());
    assert_clear(&mut evm);
}

/// A post-Regolith deposit that halts settles its start, ends its lifecycle settled, and then
/// becomes a failed deposit at `catch_error`, which owes no second end; the EVM is reused.
#[test]
fn test_a_halted_deposit_becomes_a_failed_deposit_and_the_evm_is_reused() {
    let mut evm = context_running(vec![INVALID]).build_op();
    for _ in 0..2 {
        let result = evm.transact(deposit()).expect("a failed deposit is a result");
        assert!(
            matches!(
                result.result,
                ExecutionResult::Halt { reason: OpHaltReason::FailedDeposit, .. }
            ),
            "{:?}",
            result.result
        );
        assert_clear(&mut evm);
    }
}

/// `OpHandler` with a `last_frame_result` that settles the transaction's frame but does not close
/// the transaction's start, as an override that misses the call would.
struct MissesTheClose(OpHandler<DefaultOpEvm<Ctx>, Error, EthFrame>);

impl Handler for MissesTheClose {
    type Evm = DefaultOpEvm<Ctx>;
    type Error = Error;
    type HaltReason = OpHaltReason;

    fn validate_env(&self, evm: &mut Self::Evm) -> Result<(), Self::Error> {
        self.0.validate_env(evm)
    }

    fn validate_against_state_and_deduct_caller(
        &self,
        evm: &mut Self::Evm,
        init_and_floor_gas: &mut revm::interpreter::InitialAndFloorGas,
    ) -> Result<(), Self::Error> {
        self.0.validate_against_state_and_deduct_caller(evm, init_and_floor_gas)
    }

    fn last_frame_result(
        &mut self,
        evm: &mut Self::Evm,
        frame_result: &mut FrameResult,
        parent_gas: &mut GasTracker,
    ) -> Result<(), Self::Error> {
        settle_first_frame::<_, Self::Error>(evm.ctx(), frame_result, parent_gas)?;
        *frame_result.gas_mut().tracker_mut() = *parent_gas;
        Ok(())
    }

    fn reimburse_caller(
        &self,
        evm: &mut Self::Evm,
        frame_result: &mut FrameResult,
    ) -> Result<(), Self::Error> {
        self.0.reimburse_caller(evm, frame_result)
    }

    fn refund(
        &self,
        evm: &mut Self::Evm,
        frame_result: &mut FrameResult,
        eip7702_refund: i64,
    ) -> Result<(), Self::Error> {
        self.0.refund(evm, frame_result, eip7702_refund)
    }

    fn reward_beneficiary(
        &self,
        evm: &mut Self::Evm,
        frame_result: &mut FrameResult,
    ) -> Result<(), Self::Error> {
        self.0.reward_beneficiary(evm, frame_result)
    }

    fn execution_result(
        &mut self,
        evm: &mut Self::Evm,
        frame_result: FrameResult,
        result_gas: ResultGas,
    ) -> Result<ExecutionResult<Self::HaltReason>, Self::Error> {
        self.0.execution_result(evm, frame_result, result_gas)
    }

    fn catch_error(
        &self,
        evm: &mut Self::Evm,
        error: Self::Error,
    ) -> Result<ExecutionResult<Self::HaltReason>, Self::Error> {
        self.0.catch_error(evm, error)
    }
}

/// A run whose transaction's start is never closed fails at `OpHandler`'s successful boundary
/// with a fatal error, in every build, and the boundary discards the lifecycle, so the EVM runs
/// the next transaction.
#[test]
fn test_a_missed_settlement_fails_at_the_successful_boundary() {
    let mut evm = context().build_op();
    evm.ctx().set_tx(tx());
    assert_fatal(
        MissesTheClose(OpHandler::new()).run(&mut evm),
        "the transaction ended with a start open",
    );
    assert_clear(&mut evm);
    // As `transact` does after a run, error or not.
    let _ = evm.finalize();
    assert!(evm.transact(tx()).expect("reused").result.is_success());
}

/// The events the hooks received, which the mirror the context keeps in debug builds records.
#[cfg(debug_assertions)]
mod events {
    use super::*;
    use revm::context::{LocalContextTr, context_interface::LifecycleEvent};

    fn events<E: EvmTr<Context = Ctx>>(evm: &mut E) -> Vec<String> {
        let mirror = evm.ctx().local.lifecycle_mirror().expect("a debug build keeps one");
        mirror
            .events()
            .iter()
            .map(|event| match event {
                LifecycleEvent::Split(split) => {
                    format!("split {} parent {:?} depth {}", split.id, split.parent, split.depth)
                }
                LifecycleEvent::Settle(settle) => {
                    format!("settle {} depth {} {:?}", settle.id, settle.depth, settle.status)
                }
                LifecycleEvent::End(end) => format!("end {end:?}"),
            })
            .collect()
    }

    const PAIRED: [&str; 5] = [
        "split 1 parent None depth 0",
        "split 2 parent Some(1) depth 1",
        "settle 2 depth 1 Ok",
        "settle 1 depth 0 Ok",
        "end Settled",
    ];

    #[test]
    fn test_an_op_transaction_settles_every_start() {
        let mut evm = context().build_op();
        assert!(evm.transact(tx()).expect("runs").result.is_success());
        assert_eq!(events(&mut evm), PAIRED);
    }

    #[test]
    fn test_a_deposit_settles_every_start() {
        let mut evm = context().build_op();
        assert!(evm.transact(deposit()).expect("runs").result.is_success());
        assert_eq!(events(&mut evm), PAIRED);
    }

    #[test]
    fn test_an_op_system_call_settles_every_start() {
        let mut evm = context().build_op();
        assert!(evm.system_call_one(PARENT, Bytes::new()).expect("runs").is_success());
        assert_eq!(events(&mut evm), PAIRED);
    }

    #[test]
    fn test_a_fatal_child_error_ends_the_lifecycle_discarded() {
        let mut evm = context().build_op_with_inspector(FailChildOnce::default());
        assert!(evm.inspect_tx(tx()).is_err());
        assert_eq!(
            events(&mut evm),
            ["split 1 parent None depth 0", "split 2 parent Some(1) depth 1", "end Discarded"]
        );
        assert!(evm.inspect_tx(tx()).expect("reused").result.is_success());
        assert_eq!(events(&mut evm), PAIRED);
    }

    #[test]
    fn test_a_halted_deposit_settles_before_it_becomes_a_failed_deposit() {
        let mut evm = context_running(vec![INVALID]).build_op();
        evm.transact(deposit()).expect("a failed deposit is a result");
        assert_eq!(
            events(&mut evm),
            ["split 1 parent None depth 0", "settle 1 depth 0 Halt", "end Settled"]
        );
    }
}
