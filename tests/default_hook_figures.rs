//! With the revm fork's default lifecycle hooks, `OpHandler`'s settlement of the transaction's
//! frame computes the figures it computed before the lifecycle existed.
//!
//! `OpHandler::last_frame_result` closes the transaction's start after the revm fork's settlement
//! of the first frame, so a figure the close moved would reach every OP transaction. Each case
//! answers or rewrites the transaction's frame as an inspector can; the expected figures were
//! recorded on this crate's revision before the lifecycle (`589c5078`, on the revm fork's
//! `b370b538`).
//!
//! The context has the Amsterdam state gas EIPs on and a gas limit below the transaction gas cap,
//! so the transaction's frame starts with no reservoir and the recipient's leaf spills into
//! regular gas: a result that hands a reservoir back unwinds that spill into regular gas, and a
//! halt keeps it.

use op_revm::{DefaultOp, L1BlockInfo, OpBuilder, OpContext, OpSpecId, OpTransaction};
use revm::{
    Context, InspectEvm, Inspector,
    context::{CfgEnv, TxEnv, result::ExecutionResult},
    context_interface::cfg::GasParams,
    database::InMemoryDB,
    interpreter::{
        CallInputs, CallOutcome, Gas, InstructionResult, InterpreterResult,
        interpreter::EthInterpreter,
    },
    primitives::{Address, Bytes, TxKind, U256, address, hardfork::SpecId},
    state::AccountInfo,
};

type Ctx = OpContext<InMemoryDB>;

const CALLER: Address = address!("0x0000000000000000000000000000000000ca11e7");
/// No code, no balance, no nonce.
const EMPTY: Address = address!("0x00000000000000000000000000000000000000ab");
const GAS_LIMIT: u64 = 1_000_000;

/// What the inspector does to the transaction's frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Shape {
    /// `call` answers with a constructor-built outcome: the status, and a tracker with the whole
    /// gas limit and a reservoir of the given amount.
    Answer(InstructionResult, u64),
    /// `call_end` puts a constructor-built tracker with the whole gas limit and a reservoir of the
    /// given amount in the result.
    ReplaceGas(u64),
    /// `call_end` rewrites the status and puts such a tracker in the result.
    RewriteAndReplace(InstructionResult, u64),
}

const fn built_gas(gas_limit: u64, reservoir: u64) -> Gas {
    let mut gas = Gas::new(gas_limit);
    gas.tracker_mut().set_reservoir(reservoir);
    gas
}

struct Rewrites(Shape);

impl Inspector<Ctx, EthInterpreter> for Rewrites {
    fn call(&mut self, _: &mut Ctx, inputs: &mut CallInputs) -> Option<CallOutcome> {
        let Shape::Answer(status, reservoir) = self.0 else {
            return None;
        };
        (inputs.caller == CALLER).then(|| {
            CallOutcome::new(
                InterpreterResult::new(
                    status,
                    Bytes::new(),
                    built_gas(inputs.gas_limit, reservoir),
                ),
                inputs.return_memory_offset.clone(),
            )
        })
    }

    fn call_end(&mut self, _: &mut Ctx, inputs: &CallInputs, outcome: &mut CallOutcome) {
        if inputs.caller != CALLER {
            return;
        }
        match self.0 {
            Shape::Answer(..) => {}
            Shape::ReplaceGas(reservoir) => {
                outcome.result.gas = built_gas(inputs.gas_limit, reservoir);
            }
            Shape::RewriteAndReplace(status, reservoir) => {
                outcome.result.result = status;
                outcome.result.gas = built_gas(inputs.gas_limit, reservoir);
            }
        }
    }
}

fn context() -> Ctx {
    let mut db = InMemoryDB::default();
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

/// A value transfer of 1 to [`EMPTY`], or the same as a deposit.
fn tx(deposit: bool) -> OpTransaction<TxEnv> {
    let base = TxEnv::builder()
        .caller(CALLER)
        .kind(TxKind::Call(EMPTY))
        .value(U256::from(1))
        .gas_price(0)
        .gas_limit(GAS_LIMIT);
    let builder = OpTransaction::builder().base(base);
    if deposit {
        builder.is_deposit_tx().build_fill()
    } else {
        builder.enveloped_tx(Some(Bytes::new())).build_fill()
    }
}

/// The figures a transaction's result carries: its kind, its gas and its output.
fn figures<H: core::fmt::Debug>(result: &ExecutionResult<H>) -> String {
    let kind = match result {
        ExecutionResult::Success { reason, .. } => format!("Success({reason:?})"),
        ExecutionResult::Revert { .. } => "Revert".to_string(),
        ExecutionResult::Halt { reason, .. } => format!("Halt({reason:?})"),
    };
    let gas = result.gas();
    format!(
        "{kind} spent {} state {} refunded {} floor {} output {}",
        gas.total_gas_spent(),
        gas.state_gas_spent_final(),
        gas.inner_refunded(),
        gas.floor_gas(),
        result.output().cloned().unwrap_or_default(),
    )
}

fn run(deposit: bool, shape: Shape) -> String {
    let mut evm = context().build_op_with_inspector(Rewrites(shape));
    figures(&evm.inspect_tx(tx(deposit)).expect("runs").result)
}

fn cases() -> Vec<(String, String)> {
    use InstructionResult::{OutOfGas, Revert, Stop};
    let mut shapes = Vec::new();
    for status in [Stop, Revert, OutOfGas] {
        for reservoir in [0, 1] {
            shapes.push(Shape::Answer(status, reservoir));
        }
    }
    shapes.push(Shape::ReplaceGas(1));
    shapes.push(Shape::RewriteAndReplace(OutOfGas, 1));
    shapes.push(Shape::RewriteAndReplace(Revert, 1));
    let mut cases = Vec::new();
    for deposit in [false, true] {
        let what = if deposit { "deposit" } else { "transaction" };
        for shape in &shapes {
            cases.push((format!("{what} {shape:?}"), run(deposit, *shape)));
        }
    }
    cases
}

/// The figures recorded on `589c5078`.
const EXPECTED: &[(&str, &str)] = &[
    (
        "transaction Answer(Stop, 0)",
        "Success(Stop) spent 204600 state 183600 refunded 0 floor 21000 output 0x",
    ),
    (
        "transaction Answer(Stop, 1)",
        "Success(Stop) spent 204599 state 183600 refunded 0 floor 21000 output 0x",
    ),
    (
        "transaction Answer(Revert, 0)",
        "Revert spent 204600 state 183600 refunded 0 floor 21000 output 0x",
    ),
    (
        "transaction Answer(Revert, 1)",
        "Revert spent 204599 state 183600 refunded 0 floor 21000 output 0x",
    ),
    (
        "transaction Answer(OutOfGas, 0)",
        "Halt(Base(OutOfGas(Basic))) spent 1000000 state 183600 refunded 0 floor 21000 output 0x",
    ),
    (
        "transaction Answer(OutOfGas, 1)",
        "Halt(Base(OutOfGas(Basic))) spent 999999 state 183600 refunded 0 floor 21000 output 0x",
    ),
    (
        "transaction ReplaceGas(1)",
        "Success(Stop) spent 204599 state 183600 refunded 0 floor 21000 output 0x",
    ),
    (
        "transaction RewriteAndReplace(OutOfGas, 1)",
        "Halt(Base(OutOfGas(Basic))) spent 999999 state 0 refunded 0 floor 21000 output 0x",
    ),
    (
        "transaction RewriteAndReplace(Revert, 1)",
        "Revert spent 20999 state 0 refunded 0 floor 21000 output 0x",
    ),
    (
        "deposit Answer(Stop, 0)",
        "Success(Stop) spent 204600 state 183600 refunded 0 floor 21000 output 0x",
    ),
    (
        "deposit Answer(Stop, 1)",
        "Success(Stop) spent 204599 state 183600 refunded 0 floor 21000 output 0x",
    ),
    (
        "deposit Answer(Revert, 0)",
        "Revert spent 204600 state 183600 refunded 0 floor 21000 output 0x",
    ),
    (
        "deposit Answer(Revert, 1)",
        "Revert spent 204599 state 183600 refunded 0 floor 21000 output 0x",
    ),
    (
        "deposit Answer(OutOfGas, 0)",
        "Halt(FailedDeposit) spent 1000000 state 0 refunded 0 floor 0 output 0x",
    ),
    (
        "deposit Answer(OutOfGas, 1)",
        "Halt(FailedDeposit) spent 1000000 state 0 refunded 0 floor 0 output 0x",
    ),
    (
        "deposit ReplaceGas(1)",
        "Success(Stop) spent 204599 state 183600 refunded 0 floor 21000 output 0x",
    ),
    (
        "deposit RewriteAndReplace(OutOfGas, 1)",
        "Halt(FailedDeposit) spent 1000000 state 0 refunded 0 floor 0 output 0x",
    ),
    (
        "deposit RewriteAndReplace(Revert, 1)",
        "Revert spent 20999 state 0 refunded 0 floor 21000 output 0x",
    ),
];

#[test]
fn test_default_hooks_change_no_figure_op_handler_settles() {
    let actual = cases();
    let table: String =
        actual.iter().map(|(name, figures)| format!("    ({name:?}, {figures:?}),\n")).collect();
    let expected: Vec<(String, String)> =
        EXPECTED.iter().map(|(name, figures)| (name.to_string(), figures.to_string())).collect();
    assert!(actual == expected, "the figures moved; this revision's table:\n{table}");
}

/// The transaction's frame answered with an out-of-gas outcome whose tracker hands back a
/// reservoir of 1, which unwinds 1 of the transaction's spill into regular gas: the halt keeps it,
/// so 999,999 of the 1,000,000 gas limit is spent.
#[test]
fn test_a_halted_answer_to_the_transactions_frame_keeps_the_gas_its_settlement_returned() {
    let figures = run(false, Shape::Answer(InstructionResult::OutOfGas, 1));
    assert!(figures.starts_with("Halt(Base(OutOfGas(Basic))) spent 999999 "), "{figures}");
}
