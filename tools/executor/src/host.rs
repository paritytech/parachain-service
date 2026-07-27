//! JAM host-call dispatch for the service backend.
//!
//! JAM host calls arrive as numeric `ecalli` interrupts (the index table lives in
//! `jam-pvm-common`'s `imports.rs`: `gas`=0, `fetch`=1, `write`=4, `export`=7,
//! `log`=100, …). They're abstracted behind the [`HostCalls`] trait so the host
//! environment can be swapped or extended — we expect to add / change these a lot.
//!
//! For now [`DebugHost`] implements *no* semantics: it logs each call (index, name,
//! argument registers) and aborts the run. Real host calls come later; this makes
//! the first host call the guest reaches obvious.
//!
//! NB: `sbrk` is a PVM *instruction* (opcode 101 in the JamV1 ISA), not a host
//! call, so it never appears here — PolkaVM handles it internally.

use anyhow::{bail, Result};
use polkavm::{RawInstance, Reg};

/// Handles JAM host calls (`ecalli` interrupts) for a running service.
///
/// Arguments are in the argument registers (`a0..`) of `inst`; an implementation
/// may read/write guest memory and leaves return value(s) in `a0`(`/a1`) before
/// returning `Ok`. Returning `Err` aborts the invocation.
pub trait HostCalls {
    fn ecall(&mut self, index: u32, inst: &mut RawInstance) -> Result<()>;
}

/// Human-readable name for a JAM host-call index (tracing only).
pub fn name(index: u32) -> &'static str {
    match index {
        0 => "gas",
        1 => "fetch",
        2 => "lookup",
        3 => "read",
        4 => "write",
        5 => "info",
        6 => "historical_lookup",
        7 => "export",
        8 => "machine",
        9 => "peek",
        10 => "poke",
        11 => "pages",
        12 => "invoke",
        13 => "expunge",
        14 => "bless",
        15 => "assign",
        16 => "designate",
        17 => "checkpoint",
        18 => "new",
        19 => "upgrade",
        20 => "transfer",
        21 => "eject",
        22 => "query",
        23 => "solicit",
        24 => "forget",
        25 => "yield",
        26 => "provide",
        100 => "log",
        _ => "unknown",
    }
}

/// Logs every host call and aborts. No host-call semantics are implemented yet.
#[derive(Default)]
pub struct DebugHost;

impl HostCalls for DebugHost {
    fn ecall(&mut self, index: u32, inst: &mut RawInstance) -> Result<()> {
        let a: [u64; 6] = [
            inst.reg(Reg::ARG_REGS[0]),
            inst.reg(Reg::ARG_REGS[1]),
            inst.reg(Reg::ARG_REGS[2]),
            inst.reg(Reg::ARG_REGS[3]),
            inst.reg(Reg::ARG_REGS[4]),
            inst.reg(Reg::ARG_REGS[5]),
        ];
        eprintln!("  · ecall {index:>3} {:<12} a0..={a:x?}", name(index));
        bail!(
            "host call {index} ({}) not implemented — aborting",
            name(index)
        );
    }
}
