//! `Precompile::required_gas` of the Optimism size-limited precompiles against their own runs.
//!
//! Each of the nine precompiles in `op_revm::precompiles` that puts an input size limit in front
//! of a revm run prices itself from the price function of the run it wraps, and prices the inputs
//! its limit refuses at `0`: they are refused before any gas check. As
//! `crates/precompile/tests/required_gas.rs` of `megaeth-labs/revm` does for the built-ins, the
//! price is checked against runs of the precompile, on edge inputs and on random ones: every gas
//! limit below the price halts out of gas, the price itself and every limit above it give one
//! result, and a success uses exactly the price. The inputs cover what the run accepts, what the
//! run refuses before and after its gas check, and lengths around every size limit of the
//! precompile's family, so each wrapper sees inputs just inside and just outside its own limit and
//! the other hardforks' limits.

use op_revm::{
    OpSpecId,
    precompiles::{OpPrecompiles, bls12_381, bn254_pair},
};
use revm::precompile::{
    Precompile, PrecompileHalt, PrecompileOutput, PrecompileStatus, bls12_381 as eth_bls12_381,
    bls12_381_const::{
        G1_MSM_ADDRESS, G1_MSM_INPUT_LENGTH, G2_MSM_ADDRESS, G2_MSM_INPUT_LENGTH, PADDED_G1_LENGTH,
        PADDED_G2_LENGTH, PAIRING_ADDRESS, PAIRING_INPUT_LENGTH,
    },
    bn254,
    primitives::{Address, hex},
};

/// Random inputs per precompile, on top of its edge inputs.
const RANDOM_INPUTS: usize = 96;

/// Input lengths around every element size the wrapped runs read.
const EDGE_LENGTHS: [usize; 40] = [
    0, 1, 4, 31, 32, 33, 63, 64, 65, 95, 96, 97, 127, 128, 129, 159, 160, 161, 191, 192, 193, 212,
    213, 214, 255, 256, 257, 287, 288, 289, 383, 384, 385, 575, 576, 577, 767, 768, 769, 1_024,
];

/// The BN254 pairing input size limits of every hardfork that set one.
const BN254_PAIR_LIMITS: [usize; 3] = [
    bn254_pair::GRANITE_MAX_INPUT_SIZE,
    bn254_pair::JOVIAN_MAX_INPUT_SIZE,
    bn254_pair::KARST_MAX_INPUT_SIZE,
];
/// The BLS12-381 G1 MSM input size limits of every hardfork that set one.
const G1_MSM_LIMITS: [usize; 2] =
    [bls12_381::ISTHMUS_G1_MSM_MAX_INPUT_SIZE, bls12_381::JOVIAN_G1_MSM_MAX_INPUT_SIZE];
/// The BLS12-381 G2 MSM input size limits of every hardfork that set one.
const G2_MSM_LIMITS: [usize; 2] =
    [bls12_381::ISTHMUS_G2_MSM_MAX_INPUT_SIZE, bls12_381::JOVIAN_G2_MSM_MAX_INPUT_SIZE];
/// The BLS12-381 pairing input size limits of every hardfork that set one.
const PAIRING_LIMITS: [usize; 2] =
    [bls12_381::ISTHMUS_PAIRING_MAX_INPUT_SIZE, bls12_381::JOVIAN_PAIRING_MAX_INPUT_SIZE];

/// The family of runs a wrapper reaches, which decides the inputs it is given.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Family {
    Bn254Pair,
    G1Msm,
    G2Msm,
    Pairing,
}

impl Family {
    /// The element the run's input is a whole number of.
    const fn element(self) -> usize {
        match self {
            Self::Bn254Pair => bn254::PAIR_ELEMENT_LEN,
            Self::G1Msm => G1_MSM_INPUT_LENGTH,
            Self::G2Msm => G2_MSM_INPUT_LENGTH,
            Self::Pairing => PAIRING_INPUT_LENGTH,
        }
    }

    /// Every size limit a hardfork set on this family.
    const fn limits(self) -> &'static [usize] {
        match self {
            Self::Bn254Pair => &BN254_PAIR_LIMITS,
            Self::G1Msm => &G1_MSM_LIMITS,
            Self::G2Msm => &G2_MSM_LIMITS,
            Self::Pairing => &PAIRING_LIMITS,
        }
    }

    /// The address the wrappers of this family, and the built-in they wrap, are at.
    const fn address(self) -> Address {
        match self {
            Self::Bn254Pair => bn254::pair::ADDRESS,
            Self::G1Msm => G1_MSM_ADDRESS,
            Self::G2Msm => G2_MSM_ADDRESS,
            Self::Pairing => PAIRING_ADDRESS,
        }
    }

    /// The built-in without a size limit whose run the wrappers of this family reach.
    const fn wrapped(self) -> Precompile {
        match self {
            Self::Bn254Pair => bn254::pair::ISTANBUL,
            Self::G1Msm => eth_bls12_381::g1_msm::PRECOMPILE,
            Self::G2Msm => eth_bls12_381::g2_msm::PRECOMPILE,
            Self::Pairing => eth_bls12_381::pairing::PRECOMPILE,
        }
    }

    /// Whether `halt` is the refusal the wrappers of this family give an input above their limit.
    fn is_size_limit_refusal(self, halt: &PrecompileHalt) -> bool {
        match self {
            Self::Bn254Pair => matches!(halt, PrecompileHalt::Bn254PairLength),
            Self::G1Msm | Self::G2Msm | Self::Pairing => {
                matches!(halt, PrecompileHalt::Other(msg) if msg.contains("input length too long"))
            }
        }
    }
}

/// A precompile that puts an input size limit in front of a revm run.
#[derive(Debug)]
struct Wrapper {
    name: &'static str,
    precompile: Precompile,
    max_input_size: usize,
    family: Family,
}

/// The nine size-limited precompiles `op_revm::precompiles` defines.
const WRAPPERS: [Wrapper; 9] = [
    Wrapper {
        name: "bn254_pair::GRANITE",
        precompile: bn254_pair::GRANITE,
        max_input_size: bn254_pair::GRANITE_MAX_INPUT_SIZE,
        family: Family::Bn254Pair,
    },
    Wrapper {
        name: "bn254_pair::JOVIAN",
        precompile: bn254_pair::JOVIAN,
        max_input_size: bn254_pair::JOVIAN_MAX_INPUT_SIZE,
        family: Family::Bn254Pair,
    },
    Wrapper {
        name: "bn254_pair::KARST",
        precompile: bn254_pair::KARST,
        max_input_size: bn254_pair::KARST_MAX_INPUT_SIZE,
        family: Family::Bn254Pair,
    },
    Wrapper {
        name: "bls12_381::ISTHMUS_G1_MSM",
        precompile: bls12_381::ISTHMUS_G1_MSM,
        max_input_size: bls12_381::ISTHMUS_G1_MSM_MAX_INPUT_SIZE,
        family: Family::G1Msm,
    },
    Wrapper {
        name: "bls12_381::ISTHMUS_G2_MSM",
        precompile: bls12_381::ISTHMUS_G2_MSM,
        max_input_size: bls12_381::ISTHMUS_G2_MSM_MAX_INPUT_SIZE,
        family: Family::G2Msm,
    },
    Wrapper {
        name: "bls12_381::ISTHMUS_PAIRING",
        precompile: bls12_381::ISTHMUS_PAIRING,
        max_input_size: bls12_381::ISTHMUS_PAIRING_MAX_INPUT_SIZE,
        family: Family::Pairing,
    },
    Wrapper {
        name: "bls12_381::JOVIAN_G1_MSM",
        precompile: bls12_381::JOVIAN_G1_MSM,
        max_input_size: bls12_381::JOVIAN_G1_MSM_MAX_INPUT_SIZE,
        family: Family::G1Msm,
    },
    Wrapper {
        name: "bls12_381::JOVIAN_G2_MSM",
        precompile: bls12_381::JOVIAN_G2_MSM,
        max_input_size: bls12_381::JOVIAN_G2_MSM_MAX_INPUT_SIZE,
        family: Family::G2Msm,
    },
    Wrapper {
        name: "bls12_381::JOVIAN_PAIRING",
        precompile: bls12_381::JOVIAN_PAIRING,
        max_input_size: bls12_381::JOVIAN_PAIRING_MAX_INPUT_SIZE,
        family: Family::Pairing,
    },
];

/// What the runs of one wrapper showed.
#[derive(Debug, Default)]
struct Seen {
    /// Runs at the price that succeeded, whose gas used was compared.
    used_the_price: usize,
    /// Runs at the price that a check after the gas check failed.
    failed_after_the_gas_check: usize,
    /// Inputs within the limit that the wrapped run refused before its gas check: priced at 0.
    refused_by_the_run_before_pricing: usize,
    /// Inputs above the limit, refused by the wrapper before any gas check: priced at 0.
    refused_by_the_size_limit: usize,
    /// The longest input a run at the price succeeded on.
    longest_success: usize,
}

/// A deterministic `SplitMix64` generator, so the test needs no dependency and every run sees the
/// same inputs.
struct Rng(u64);

impl Rng {
    const fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }

    /// A number in `0..n`.
    const fn below(&mut self, n: usize) -> usize {
        (self.next_u64() % n as u64) as usize
    }

    fn bytes(&mut self, len: usize) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(len + 8);
        while bytes.len() < len {
            bytes.extend_from_slice(&self.next_u64().to_le_bytes());
        }
        bytes.truncate(len);
        bytes
    }
}

const fn is_out_of_gas(output: &PrecompileOutput) -> bool {
    matches!(output.status, PrecompileStatus::Halt(PrecompileHalt::OutOfGas))
}

fn run(precompile: &Precompile, input: &[u8], gas_limit: u64) -> PrecompileOutput {
    precompile.execute(input, gas_limit, 0).expect("no size-limited precompile fails fatally")
}

/// Checks the price `wrapper` answers for `input` against its runs.
fn check(wrapper: &Wrapper, input: &[u8], seen: &mut Seen) {
    let Wrapper { name, precompile, max_input_size, family } = wrapper;
    let len = input.len();
    let price =
        precompile.required_gas(input).unwrap_or_else(|| panic!("{name} has no price function"));
    let shown = || hex::encode(&input[..len.min(64)]);

    // The price is the wrapped run's for an input the limit accepts, and 0 for one it refuses.
    let wrapped_price = family.wrapped().required_gas(input).expect("the built-ins are priced");
    if len > *max_input_size {
        assert_eq!(price, 0, "{name}: an input of {len} bytes is above the limit {max_input_size}");
    } else {
        assert_eq!(price, wrapped_price, "{name}: an input of {len} bytes is within the limit");
    }

    // Below the price: out of gas, at every limit.
    for limit in [0, price / 2, price.saturating_sub(1)] {
        if limit < price {
            assert!(
                is_out_of_gas(&run(precompile, input, limit)),
                "{name}: a limit of {limit} below the price {price} must run out of gas \
                 (input {len} bytes, {}..)",
                shown(),
            );
        }
    }

    // At the price and above it: never out of gas, one result, and a success uses the price.
    let at_price = run(precompile, input, price);
    assert!(
        !is_out_of_gas(&at_price),
        "{name}: the price {price} must be enough (input {len} bytes, {}..)",
        shown(),
    );
    for limit in [price + 1, price.saturating_mul(2).saturating_add(7), u64::MAX] {
        assert_eq!(
            run(precompile, input, limit),
            at_price,
            "{name}: a limit of {limit} must give the result of the price {price} \
             (input {len} bytes, {}..)",
            shown(),
        );
    }

    if len > *max_input_size {
        let PrecompileStatus::Halt(halt) = &at_price.status else {
            panic!("{name}: an input of {len} bytes above the limit must be refused");
        };
        assert!(family.is_size_limit_refusal(halt), "{name}: refused with {halt:?}");
        seen.refused_by_the_size_limit += 1;
        return;
    }

    // Within the limit the wrapper's run is the wrapped run.
    assert_eq!(
        run(&family.wrapped(), input, price),
        at_price,
        "{name}: an input of {len} bytes within the limit must run as the wrapped precompile",
    );
    if at_price.is_success() || at_price.is_revert() {
        assert_eq!(
            at_price.gas_used,
            price,
            "{name}: a success must use the price (input {len} bytes, {}..)",
            shown(),
        );
        seen.used_the_price += 1;
        seen.longest_success = seen.longest_success.max(len);
    } else if price == 0 {
        seen.refused_by_the_run_before_pricing += 1;
    } else {
        seen.failed_after_the_gas_check += 1;
    }
}

/// An input the wrapped run accepts: `pairs` elements whose points are at infinity, which every
/// backend accepts, with random scalars for the MSMs.
fn valid_input(family: Family, pairs: usize, rng: &mut Rng) -> Vec<u8> {
    let mut input = Vec::with_capacity(pairs * family.element());
    for _ in 0..pairs {
        match family {
            Family::Bn254Pair | Family::Pairing => input.resize(input.len() + family.element(), 0),
            Family::G1Msm | Family::G2Msm => {
                let point =
                    if family == Family::G1Msm { PADDED_G1_LENGTH } else { PADDED_G2_LENGTH };
                input.resize(input.len() + point, 0);
                input.extend(rng.bytes(family.element() - point));
            }
        }
    }
    input
}

/// Lengths around every size limit of `family`: a byte and an element on each side of the limit
/// and of the longest whole number of elements it allows.
fn limit_lengths(family: Family) -> Vec<usize> {
    let element = family.element();
    let mut lengths = Vec::new();
    for &limit in family.limits() {
        let whole = limit / element * element;
        lengths.extend([limit - 1, limit, limit + 1, limit + element]);
        lengths.extend([whole - element, whole - 1, whole, whole + 1, whole + element]);
    }
    lengths.sort_unstable();
    lengths.dedup();
    lengths
}

/// The inputs `wrapper` is checked on: valid ones, edge lengths zeroed and random, lengths around
/// every limit of its family zeroed and random, whole valid inputs at those limits, and random
/// lengths near the limits and below 1,200 bytes.
fn inputs(wrapper: &Wrapper, rng: &mut Rng) -> Vec<Vec<u8>> {
    let family = wrapper.family;
    let element = family.element();
    let mut inputs = Vec::new();
    for pairs in 1..=3 {
        inputs.push(valid_input(family, pairs, rng));
    }
    for len in EDGE_LENGTHS {
        inputs.push(vec![0; len]);
        inputs.push(rng.bytes(len));
    }
    for len in limit_lengths(family) {
        inputs.push(vec![0; len]);
        inputs.push(rng.bytes(len));
    }
    for &limit in family.limits() {
        // The longest valid input the limit allows, and one element more.
        let whole = limit / element;
        inputs.push(valid_input(family, whole, rng));
        inputs.push(valid_input(family, whole + 1, rng));
        for _ in 0..8 {
            let len = limit - 3 * element + rng.below(6 * element);
            inputs.push(rng.bytes(len));
        }
    }
    for _ in 0..RANDOM_INPUTS {
        let len = rng.below(1_201);
        inputs.push(rng.bytes(len));
    }
    inputs
}

/// The differential test: every size-limited precompile, on its inputs.
#[test]
fn required_gas_matches_every_size_limited_run() {
    for (index, wrapper) in WRAPPERS.iter().enumerate() {
        let name = wrapper.name;
        let mut rng = Rng(0x5eed ^ index as u64);
        let mut seen = Seen::default();
        let inputs = inputs(wrapper, &mut rng);
        for input in &inputs {
            check(wrapper, input, &mut seen);
        }
        assert!(seen.used_the_price > 0, "{name}: no input succeeded: {seen:?}");
        assert!(seen.refused_by_the_size_limit > 0, "{name}: no input was above the limit");
        let longest = wrapper.max_input_size / wrapper.family.element() * wrapper.family.element();
        assert_eq!(
            seen.longest_success, longest,
            "{name}: the longest input the limit allows must have run at its price"
        );
        println!("{name}: {} inputs, {seen:?}", inputs.len());
    }
}

/// Each wrapper prices at its limit as the run it wraps, and above it at `0`, and refuses there at
/// every gas limit without running out of gas.
#[test]
fn the_size_limit_refuses_before_any_gas_check() {
    // BN254 pairing: 34,000 per pair and 45,000. Granite's limit is not a whole number of pairs:
    // the 175 bytes past the last pair are refused by the run, after its gas check for 586 pairs.
    let pair = |pairs: u64| Some(pairs * 34_000 + 45_000);
    assert_eq!(bn254_pair::GRANITE.required_gas(&[0; 112_687]), pair(586));
    assert_eq!(bn254_pair::GRANITE.required_gas(&[0; 112_688]), Some(0));
    assert_eq!(bn254_pair::JOVIAN.required_gas(&[0; 81_984]), pair(427));
    assert_eq!(bn254_pair::JOVIAN.required_gas(&[0; 81_985]), Some(0));
    assert_eq!(bn254_pair::KARST.required_gas(&[0; 57_600]), pair(300));
    assert_eq!(bn254_pair::KARST.required_gas(&[0; 57_601]), Some(0));
    let run_pair = |precompile: &Precompile, len: usize, gas_limit| {
        run(precompile, &vec![0; len], gas_limit).status
    };
    assert_eq!(
        run_pair(&bn254_pair::GRANITE, 112_687, 586 * 34_000 + 45_000 - 1),
        PrecompileStatus::Halt(PrecompileHalt::OutOfGas)
    );
    assert_eq!(
        run_pair(&bn254_pair::GRANITE, 112_687, 586 * 34_000 + 45_000),
        PrecompileStatus::Halt(PrecompileHalt::Bn254PairLength)
    );

    for wrapper in &WRAPPERS {
        let Wrapper { name, precompile, max_input_size, family } = wrapper;
        let element = family.element();
        let whole = max_input_size / element * element;
        let at_limit = vec![0; whole];
        assert_eq!(
            precompile.required_gas(&at_limit),
            family.wrapped().required_gas(&at_limit),
            "{name}: {whole} bytes"
        );
        for len in [max_input_size + 1, whole + element] {
            let above = vec![0; len];
            assert_eq!(precompile.required_gas(&above), Some(0), "{name}: {len} bytes");
            for gas_limit in [0, u64::MAX] {
                let PrecompileStatus::Halt(halt) = run(precompile, &above, gas_limit).status else {
                    panic!("{name}: {len} bytes must be refused");
                };
                assert!(family.is_size_limit_refusal(&halt), "{name}: {len} bytes: {halt:?}");
            }
        }
    }
}

/// The size limit each hardfork's set puts on a family, `None` where it dispatches the built-in.
const fn limit_in(spec: OpSpecId, family: Family) -> Option<usize> {
    match spec {
        OpSpecId::BEDROCK |
        OpSpecId::REGOLITH |
        OpSpecId::CANYON |
        OpSpecId::ECOTONE |
        OpSpecId::FJORD => None,
        OpSpecId::GRANITE | OpSpecId::HOLOCENE => match family {
            Family::Bn254Pair => Some(bn254_pair::GRANITE_MAX_INPUT_SIZE),
            Family::G1Msm | Family::G2Msm | Family::Pairing => None,
        },
        OpSpecId::ISTHMUS => Some(match family {
            Family::Bn254Pair => bn254_pair::GRANITE_MAX_INPUT_SIZE,
            Family::G1Msm => bls12_381::ISTHMUS_G1_MSM_MAX_INPUT_SIZE,
            Family::G2Msm => bls12_381::ISTHMUS_G2_MSM_MAX_INPUT_SIZE,
            Family::Pairing => bls12_381::ISTHMUS_PAIRING_MAX_INPUT_SIZE,
        }),
        OpSpecId::JOVIAN | OpSpecId::KARST | OpSpecId::INTEROP => Some(match family {
            Family::Bn254Pair if matches!(spec, OpSpecId::JOVIAN) => {
                bn254_pair::JOVIAN_MAX_INPUT_SIZE
            }
            Family::Bn254Pair => bn254_pair::KARST_MAX_INPUT_SIZE,
            Family::G1Msm => bls12_381::JOVIAN_G1_MSM_MAX_INPUT_SIZE,
            Family::G2Msm => bls12_381::JOVIAN_G2_MSM_MAX_INPUT_SIZE,
            Family::Pairing => bls12_381::JOVIAN_PAIRING_MAX_INPUT_SIZE,
        }),
    }
}

/// Every precompile of every hardfork's set answers, and the value each set dispatches for a
/// size-limited address prices with that hardfork's limit: the consumer takes the price from the
/// set it runs.
#[test]
fn every_set_prices_with_its_own_limits() {
    let specs = [
        OpSpecId::BEDROCK,
        OpSpecId::REGOLITH,
        OpSpecId::CANYON,
        OpSpecId::ECOTONE,
        OpSpecId::FJORD,
        OpSpecId::GRANITE,
        OpSpecId::HOLOCENE,
        OpSpecId::ISTHMUS,
        OpSpecId::JOVIAN,
        OpSpecId::KARST,
        OpSpecId::INTEROP,
    ];
    for spec in specs {
        let set = OpPrecompiles::new_with_spec(spec).precompiles();
        for precompile in set.inner().values() {
            assert!(
                precompile.required_gas(&[]).is_some(),
                "{} in {spec:?} has no price function",
                precompile.id().name()
            );
        }
        for family in [Family::Bn254Pair, Family::G1Msm, Family::G2Msm, Family::Pairing] {
            let Some(precompile) = set.get(&family.address()) else {
                assert!(limit_in(spec, family).is_none(), "{spec:?} lacks {family:?}");
                continue;
            };
            let element = family.element();
            let price = |len| precompile.required_gas(&vec![0; len]);
            let wrapped_price = |len| family.wrapped().required_gas(&vec![0; len]);
            if let Some(limit) = limit_in(spec, family) {
                let whole = limit / element * element;
                assert_eq!(price(whole), wrapped_price(whole), "{spec:?} {family:?}");
                assert_eq!(price(whole + element), Some(0), "{spec:?} {family:?}");
            } else {
                // No limit: priced as the built-in beyond every limit of the family.
                let beyond = family.limits().iter().max().unwrap() + element;
                assert_eq!(price(beyond), wrapped_price(beyond), "{spec:?} {family:?}");
            }
        }
    }
}
