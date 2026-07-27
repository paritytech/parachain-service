//! Service backend: run a JAM service blob (`*.jam`) on a bare PolkaVM engine.
//!
//! Unlike the runtime backend (Substrate `WasmExecutor`, which resolves `sp_io`
//! imports by name), a JAM service uses numeric `ecalli` host calls and the entry
//! ABI `refine_ext/accumulate_ext(ptr, size) -> (u64, u64)` = `(out_ptr, out_len)`.
//! So we drive a raw PolkaVM instance ourselves: write the encoded params into the
//! guest heap, invoke the export, and service `ecalli` interrupts via [`HostCalls`].

use anyhow::{anyhow, bail, Result};
use polkavm::{
    program::InstructionSetKind, BackendKind, Config, Engine, GasMeteringKind, InterruptKind,
    Module, ModuleConfig, ProgramBlob, ProgramCounter, ProgramParts, RawInstance, Reg,
};

use crate::host::HostCalls;

/// Which JAM entry point to invoke.
#[derive(Clone, Copy)]
pub enum Entry {
    Refine,
    Accumulate,
}

impl Entry {
    /// JAM dispatch index. `.jam` blobs strip PolkaVM's named-export section, so
    /// entry points are addressed by position in `jam-pvm-builder`'s dispatch table
    /// (`Service => [refine_ext, accumulate_ext]`); see [`entry_pc`].
    fn index(self) -> u32 {
        match self {
            Entry::Refine => 0,
            Entry::Accumulate => 1,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Entry::Refine => "refine",
            Entry::Accumulate => "accumulate",
        }
    }
}

/// Result of a service invocation.
pub struct Outcome {
    pub output: Vec<u8>,
    pub gas_used: i64,
}

/// The PolkaVM page size the `.jam` format aligns RW data to.
const PAGE_SIZE: usize = 4096;

/// Parse a `.jam` service blob into a PolkaVM `ProgramBlob` (JamV1 ISA).
fn parse_blob(jam_bytes: &[u8]) -> Result<ProgramBlob> {
    // 1. Split the `.jam` *container* into sections. This is the outer, JAM-specific
    //    format — not a PolkaVM `ProgramBlob` — so PolkaVM can't read it directly.
    let jam = jam_program_blob_common::ProgramBlob::from_bytes(jam_bytes)
        .ok_or_else(|| anyhow!("failed to parse `.jam` container"))?;

    // 2. Build a PolkaVM 0.36 `ProgramParts` for the JamV1 ISA from those sections.
    //    (Mirrors jam-program-blob-common's own `From` impl, which targets polkavm 0.30.)
    //    Note: `.jam` carries no export/import section, so entry points are addressed
    //    by dispatch index -> code offset (see `entry_pc`), not by name.
    let mut parts = ProgramParts::empty(InstructionSetKind::JamV1);
    parts.ro_data_size = jam.ro_data.len() as u32;
    parts.rw_data_size = jam.rw_data.len().next_multiple_of(PAGE_SIZE) as u32
        + jam.rw_data_padding_pages as u32 * PAGE_SIZE as u32;
    parts.stack_size = jam.stack_size;
    parts.ro_data = jam.ro_data.into_owned().into();
    parts.rw_data = jam.rw_data.into_owned().into();
    parts.code_and_jump_table = jam.code_blob.into_owned().into();

    // 3. Hand the parts to PolkaVM's own parser/validator for the inner code.
    ProgramBlob::from_parts(parts).map_err(|e| anyhow!("PolkaVM blob parse failed: {e}"))
}

/// Byte stride of a dispatch-table entry.
///
/// `polkavm-linker` lays the N entry points named in `jam-pvm-builder`'s dispatch
/// table (`[refine_ext, accumulate_ext]`) as the *first N instructions* of the
/// code, padding each to a fixed 5-byte `jump` (`writer.rs`: `minimum_size = 5`).
/// So entry `i` starts at code offset `5 * i` — refine at 0, accumulate at 5. This
/// is the JAM service invocation convention (entry points are the leading basic
/// blocks), not a jump-table lookup.
const DISPATCH_ENTRY_STRIDE: u32 = 5;

/// Resolve a JAM entry point to its initial instruction counter.
fn entry_pc(entry: Entry) -> ProgramCounter {
    ProgramCounter(entry.index() * DISPATCH_ENTRY_STRIDE)
}

/// Run `entry` with the SCALE-encoded `params`, dispatching host calls to `host`.
pub fn run(
    jam_bytes: &[u8],
    entry: Entry,
    params: &[u8],
    gas: i64,
    host: &mut dyn HostCalls,
) -> Result<Outcome> {
    let mut config = Config::new();
    config.set_backend(Some(BackendKind::Interpreter)); // portable & good for debugging
    let engine = Engine::new(&config).map_err(|e| anyhow!("engine init failed: {e}"))?;

    let blob = parse_blob(jam_bytes)?;
    let pc = entry_pc(entry);

    if std::env::var_os("EXECUTOR_TRACE").is_some() {
        eprintln!(
            "[trace] ro_data_size={} rw_data_size={} stack_size={} entering `{}` at pc {pc}",
            blob.ro_data_size(),
            blob.rw_data_size(),
            blob.stack_size(),
            entry.as_str(),
        );
    }

    let mut mcfg = ModuleConfig::new();
    mcfg.set_gas_metering(Some(GasMeteringKind::Sync));
    let module = Module::from_blob(&engine, &mcfg, blob)
        .map_err(|e| anyhow!("module compile failed: {e}"))?;

    let mut inst = module
        .instantiate()
        .map_err(|e| anyhow!("instantiate failed: {e}"))?;
    inst.set_gas(gas);

    // Place the encoded params into the guest heap and pass (ptr, len) as (a0, a1).
    let ptr = write_scratch(&mut inst, params)?;
    inst.prepare_call_untyped(pc, &[u64::from(ptr), params.len() as u64]);

    loop {
        match inst.run().map_err(|e| anyhow!("run failed: {e}"))? {
            InterruptKind::Finished => break,
            InterruptKind::Ecalli(index) => host.ecall(index, &mut inst)?,
            InterruptKind::Trap => {
                bail!(
                    "guest trapped (panic / invalid instruction / bad access) at pc {:?}",
                    inst.program_counter()
                )
            }
            InterruptKind::NotEnoughGas => bail!("guest ran out of gas (budget {gas})"),
            InterruptKind::Segfault(s) => bail!("guest segfault: {s:?}"),
            InterruptKind::Step => {}
        }
    }

    // JAM entry points return `(out_ptr, out_len)` in (a0, a1).
    let out_ptr = inst.reg(Reg::A0);
    let out_len = inst.reg(Reg::A1);
    let output = if out_len == 0 {
        Vec::new()
    } else {
        inst.read_memory(out_ptr as u32, out_len as u32)?
    };

    Ok(Outcome {
        output,
        gas_used: gas - inst.gas(),
    })
}

/// Reserve `data.len()` bytes at the top of the guest heap (via the host-side
/// `sbrk` helper — the same heap the guest's `sbrk` *instruction* bumps) and write
/// `data` there, returning the guest address.
fn write_scratch(inst: &mut RawInstance, data: &[u8]) -> Result<u32> {
    // `sbrk(0)` returns the current break = start of the free region.
    let ptr = inst
        .sbrk(0)
        .map_err(|e| anyhow!("sbrk(0) failed: {e}"))?
        .ok_or_else(|| anyhow!("sbrk(0) returned no address"))?;
    if !data.is_empty() {
        inst.sbrk(data.len() as u32)
            .map_err(|e| anyhow!("sbrk grow failed: {e}"))?
            .ok_or_else(|| anyhow!("heap exhausted reserving {} bytes for params", data.len()))?;
        inst.write_memory(ptr, data)?;
    }
    Ok(ptr)
}
