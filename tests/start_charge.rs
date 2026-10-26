//! The start charge of the transaction's frame through [`OpHandler`].
//!
//! A consumer of the revm fork that charges the transaction's gas for the transaction's frame
//! before execution, in history gas and an amount of one of its pools, declares the refundable part
//! on the first frame's input (`charged_start`). The settlement of the first frame refunds it whole
//! when the frame reverts or halts, and after a halt consumes the regular gas the refund credited;
//! a frame that succeeds keeps it. `OpHandler` overrides that settlement (`last_frame_result`) for
//! its pre-Regolith deposit rule, so it must make the refund too.
//!
//! Each test runs `OpHandler`'s phases as `Handler::run` does, with the consumer's part where a
//! consumer's handler makes it: a pool on the transaction's gas and the charge on it after
//! pre-execution, the charge declared on the first frame's input. It reads the transaction's gas
//! once the first frame settled into it.

use op_revm::{
    DefaultOp, L1BlockInfo, OpContext, OpEvm, OpSpecId, OpTransaction, OpTransactionError,
    handler::OpHandler,
};
use revm::{
    Context,
    bytecode::opcode::{INVALID, PUSH1, REVERT, STOP},
    context::{CfgEnv, TxEnv, result::EVMError},
    context_interface::{ContextTr, Database, JournalTr, cfg::gas::GasTracker},
    database::InMemoryDB,
    handler::{EthFrame, EvmTr, Handler, lifecycle},
    interpreter::{
        FrameInput, InstructionResult, PoolAmounts, PoolId, StartCharge,
        interpreter::EthInterpreter,
    },
    primitives::{Address, Bytes, TxKind, U256, address},
    state::{AccountInfo, Bytecode},
};

type Db = InMemoryDB;
type Error = EVMError<<Db as Database>::Error, OpTransactionError>;

/// Sender of every transaction.
const CALLER: Address = address!("0x0000000000000000000000000000000000ca11e7");
/// The account a call transaction calls.
const TARGET: Address = address!("0x00000000000000000000000000000000000c0de0");

/// Below the EIP-7825 cap, so the transaction's gas has no reservoir and the history charge is
/// paid from regular gas.
const GAS_LIMIT: u64 = 1_000_000;
/// The pool the consumer sets on the transaction's gas.
const POOL: u64 = 1_000;
/// What the consumer charges the transaction's gas for its frame and declares on the frame's
/// input.
/// The pool the consumer draws: any of the tracker's pools behaves the same.
const FIRST: PoolId = PoolId::new(0);
const CHARGE: StartCharge = StartCharge::new(3_520, PoolAmounts::ZERO.with(FIRST, 160));
/// What `PUSH1 0 PUSH1 0 REVERT` spends.
const REVERT_GAS: u64 = 6;

/// How the transaction's frame ends.
#[derive(Clone, Copy, Debug)]
enum End {
    Stops,
    Reverts,
    Halts,
}

impl End {
    /// The frame's code: a call's target code, or a creation's init code.
    const fn code(self) -> &'static [u8] {
        match self {
            Self::Stops => &[STOP],
            Self::Reverts => &[PUSH1, 0, PUSH1, 0, REVERT],
            Self::Halts => &[INVALID],
        }
    }

    fn assert_ended(self, result: InstructionResult) {
        let ended = match self {
            Self::Stops => result.is_ok(),
            Self::Reverts => result.is_revert(),
            Self::Halts => result.is_halt(),
        };
        assert!(ended, "{self:?}: {result:?}");
    }
}

/// The transaction's gas after the first frame settled into it: its pool, its net draw, its
/// history gas and its regular gas.
type Settled = (u64, i64, i64, u64);

const fn settled(gas: &GasTracker) -> Settled {
    (gas.pool_remaining(FIRST), gas.pool_spent(FIRST), gas.history_gas_spent(), gas.remaining())
}

/// What the transaction's gas settles to when its frame ends as `end`, from the regular gas
/// `before` it had before the consumer's charge: a frame that succeeds keeps the charge, and one
/// that reverts or halts gives it back whole, the regular gas it spilled included, which a halt
/// then consumes.
const fn expected(end: End, before: u64) -> Settled {
    match end {
        End::Stops => (
            POOL - CHARGE.pools.get(FIRST),
            CHARGE.pools.get(FIRST) as i64,
            CHARGE.history as i64,
            before - CHARGE.history,
        ),
        End::Reverts => (POOL, 0, 0, before - REVERT_GAS),
        End::Halts => (POOL, 0, 0, 0),
    }
}

/// An OP context on `spec` whose [`TARGET`] runs `end`'s code and whose [`CALLER`] is funded;
/// the L1 block info is current and prices nothing, so no L1 or operator fee applies.
fn context(spec: OpSpecId, end: End, tx: OpTransaction<TxEnv>) -> OpContext<Db> {
    let mut db = Db::default();
    db.insert_account_info(CALLER, AccountInfo::from_balance(U256::from(10u128.pow(21))));
    db.insert_account_info(
        TARGET,
        AccountInfo::default().with_code(Bytecode::new_raw(Bytes::from_static(end.code()))),
    );
    Context::op()
        .with_db(db)
        .with_cfg(CfgEnv::new_with_spec(spec))
        .with_chain(L1BlockInfo {
            l2_block: Some(U256::ZERO),
            operator_fee_scalar: Some(U256::ZERO),
            operator_fee_constant: Some(U256::ZERO),
            ..Default::default()
        })
        .with_tx(tx)
}

/// A non-deposit transaction from [`CALLER`] of `kind` with `data`. The enveloped transaction is
/// empty, which carries no L1 data fee.
fn tx(kind: TxKind, data: &[u8]) -> OpTransaction<TxEnv> {
    OpTransaction::builder()
        .base(
            TxEnv::builder()
                .caller(CALLER)
                .kind(kind)
                .data(Bytes::copy_from_slice(data))
                .gas_price(0)
                .gas_limit(GAS_LIMIT),
        )
        .enveloped_tx(Some(Bytes::new()))
        .build_fill()
}

/// A deposit transaction from [`CALLER`] to [`TARGET`], not a system transaction.
fn deposit_tx() -> OpTransaction<TxEnv> {
    OpTransaction::builder()
        .base(TxEnv::builder().caller(CALLER).kind(TxKind::Call(TARGET)).gas_limit(GAS_LIMIT))
        .is_deposit_tx()
        .build_fill()
}

/// Runs the transaction on `ctx` through `OpHandler`'s phases up to the first frame's settlement,
/// with the consumer's part for the transaction's frame. Returns how the frame ended, the
/// transaction's gas as the frame settled into it, and the regular gas it had before the
/// consumer's charge.
fn settle(ctx: OpContext<Db>) -> (InstructionResult, GasTracker, u64) {
    let mut evm = OpEvm::new(ctx, ());
    let mut handler = OpHandler::<_, Error, EthFrame<EthInterpreter>>::new();
    let init_and_floor_gas = handler.validate(&mut evm).expect("the transaction is valid");
    let mut gas = handler.tx_gas(&mut evm, &init_and_floor_gas);
    handler
        .pre_execution(&mut evm, &mut gas)
        .expect("pre-execution runs")
        .expect("the runtime gas phase pays");

    // The consumer's part, as it charges the transaction's body: the transaction's pool, and its
    // frame's charge on the transaction's gas.
    let before = gas.remaining();
    gas.set_pool_remaining(FIRST, POOL);
    assert!(gas.record_history_cost(CHARGE.history));
    assert!(gas.record_pool(FIRST, CHARGE.pools.get(FIRST)));

    // The consumer's part, as its `first_frame_input` makes it: the charge declared on the input.
    let mut init = handler
        .first_frame_input(&mut evm, &mut gas)
        .expect("the first frame's input is built")
        .expect("the runtime gas phase pays");
    match &mut init.frame_input {
        FrameInput::Call(inputs) => inputs.charged_start = CHARGE,
        FrameInput::Create(inputs) => inputs.set_charged_start(CHARGE),
        FrameInput::Empty => unreachable!("the first frame is a call or a creation"),
    }
    evm.ctx().journal_mut().checkpoint_commit();
    // The transaction's start opens with its frame's init, as `Handler::execution` opens it.
    lifecycle::open_frame_start(&mut evm, &init).expect("the start opens");

    let mut result = handler.run_exec_loop(&mut evm, init).expect("the frame runs");
    handler.last_frame_result(&mut evm, &mut result, &mut gas).expect("the frame settles");
    assert_eq!(*result.gas().tracker(), gas, "the result carries the transaction's gas");
    (result.instruction_result(), gas, before)
}

/// A call transaction's frame: the charge comes back whole when the frame reverts or halts, a
/// halt then consumes the regular gas, and a frame that succeeds keeps the charge.
#[test]
fn test_a_call_transaction_refunds_the_start_charge_when_its_frame_fails() {
    for end in [End::Reverts, End::Halts, End::Stops] {
        let ctx = context(OpSpecId::KARST, end, tx(TxKind::Call(TARGET), &[]));

        let (result, gas, before) = settle(ctx);

        end.assert_ended(result);
        assert_eq!(settled(&gas), expected(end, before), "{end:?}");
        assert_eq!(gas.reservoir(), 0, "{end:?}");
    }
}

/// A creation transaction's frame settles as a call's does.
#[test]
fn test_a_creation_transaction_refunds_the_start_charge_when_its_frame_fails() {
    for end in [End::Reverts, End::Halts, End::Stops] {
        let ctx = context(OpSpecId::KARST, end, tx(TxKind::Create, end.code()));

        let (result, gas, before) = settle(ctx);

        end.assert_ended(result);
        assert_eq!(settled(&gas), expected(end, before), "{end:?}");
        assert_eq!(gas.reservoir(), 0, "{end:?}");
    }
}

/// Before Regolith a deposit reports its gas limit as used: after a revert its rule consumes the
/// regular gas, including what the refund of the start charge credited, so the refund comes
/// before the rule. The pool, the net and the history still come back. From Regolith on, the rule
/// is off and a deposit refunds as any transaction does.
#[test]
fn test_a_deposit_refunds_the_start_charge_before_its_pre_regolith_rule() {
    for (spec, deposit_rule) in [(OpSpecId::BEDROCK, true), (OpSpecId::REGOLITH, false)] {
        let ctx = context(spec, End::Reverts, deposit_tx());

        let (result, gas, before) = settle(ctx);

        End::Reverts.assert_ended(result);
        let regular = if deposit_rule { 0 } else { before - REVERT_GAS };
        assert_eq!(settled(&gas), (POOL, 0, 0, regular), "{spec:?}");
    }
}
