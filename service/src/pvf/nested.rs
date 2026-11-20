//! Nested PVM host calls, with handles private to one PVF invocation.

use alloc::{vec, vec::Vec};
use jam_pvm_common::{imports, refine::InstanceHandle};
use jam_types::{gas_costs, InvokeOutcomeCode, SignedGas, SimpleResultCode, LOWEST_ERROR};
use parachain_service_core::host_call::HostCall;
use polkavm::Reg;

// The enclosing PVF already occupies one outer module and one outer machine slot.
const MODULES: usize = 127;
const INSTANCES: usize = 62;
const A0: usize = Reg::A0 as usize;
const A1: usize = Reg::A1 as usize;
const A2: usize = Reg::A2 as usize;
const A3: usize = Reg::A3 as usize;
const A4: usize = Reg::A4 as usize;
const OK: u64 = SimpleResultCode::Ok as u64;
const WHO: u64 = SimpleResultCode::IndexUnknown as u64;
const FULL: u64 = SimpleResultCode::StorageFull as u64;
const HUH: u64 = SimpleResultCode::ActionInvalid as u64;

#[derive(Clone, Copy)]
struct Module {
	outer: u64,
	held: bool,
	instances: usize,
}

#[derive(Clone, Copy)]
struct Instance {
	outer: u64,
	module: usize,
}

/// Owns the outer host resources created on behalf of one PVF.
pub(super) struct NestedVms {
	modules: [Option<Module>; MODULES],
	instances: [Option<Instance>; INSTANCES],
}

// Match InvokeArgs alignment for the raw gas counter and 13 registers.
#[repr(align(8))]
struct InvokeBytes([u8; 112]);

impl NestedVms {
	pub(super) fn new() -> Self {
		Self { modules: [None; MODULES], instances: [None; INSTANCES] }
	}

	fn instance(&self, handle: u64) -> Option<Instance> {
		usize::try_from(handle).ok().and_then(|i| self.instances.get(i)).copied().flatten()
	}

	fn module(&self, handle: u64) -> Option<(usize, Module)> {
		let index = usize::try_from(handle).ok()?;
		let module = self.modules.get(index).copied().flatten()?;
		module.held.then_some((index, module))
	}

	fn release_module_if_unused(&mut self, index: usize) {
		let module = self.modules[index].expect("nested module exists");
		if !module.held && module.instances == 0 {
			self.modules[index] = None;
		}
	}

	pub(super) fn dispatch(
		&mut self,
		caller: &InstanceHandle,
		call: HostCall,
		gas_left: &mut SignedGas,
		regs: &mut [u64; 13],
	) {
		match call {
			HostCall::Compile => {
				let Some(slot) = self.modules.iter().position(Option::is_none) else {
					regs[A0] = FULL;
					return;
				};
				let mode = regs[A0];
				let bytes = match mode {
					// Code bytes from PVF memory.
					0 => read_parent(caller, regs[A1], regs[A2]),
					// Historical preimage by hash.
					1 => read_parent(caller, regs[A1], 32),
					_ => {
						regs[A0] = HUH;
						return;
					},
				};
				let result = unsafe {
					imports::compile(mode, bytes.as_ptr(), regs[A2], regs[A3], regs[A4])
				};
				if result >= LOWEST_ERROR {
					regs[A0] = result;
				} else {
					self.modules[slot] = Some(Module { outer: result, held: true, instances: 0 });
					regs[A0] = slot as u64;
				}
			},
			HostCall::Machine => {
				let Some(slot) = self.instances.iter().position(Option::is_none) else {
					regs[A0] = FULL;
					return;
				};
				let Some((module_index, module)) = self.module(regs[A0]) else {
					regs[A0] = WHO;
					return;
				};
				let result = unsafe { imports::machine(module.outer, regs[A1]) };
				if result >= LOWEST_ERROR {
					regs[A0] = result;
				} else {
					self.instances[slot] = Some(Instance { outer: result, module: module_index });
					self.modules[module_index].as_mut().unwrap().instances += 1;
					regs[A0] = slot as u64;
				}
			},
			HostCall::Peek => {
				let mut bytes = read_parent(caller, regs[A1], regs[A3]);
				// The host API has no access probe: rewrite unchanged bytes to check writability.
				write_parent(caller, regs[A1], &bytes);
				let Some(instance) = self.instance(regs[A0]) else {
					regs[A0] = WHO;
					return;
				};
				let result = unsafe {
					imports::peek(instance.outer, bytes.as_mut_ptr(), regs[A2], regs[A3])
				};
				if result == OK {
					write_parent(caller, regs[A1], &bytes);
				}
				regs[A0] = result;
			},
			HostCall::Poke => {
				let bytes = read_parent(caller, regs[A1], regs[A3]);
				let Some(instance) = self.instance(regs[A0]) else {
					regs[A0] = WHO;
					return;
				};
				regs[A0] = unsafe {
					imports::poke(instance.outer, bytes.as_ptr(), regs[A2], regs[A3])
				};
			},
			HostCall::Pages => {
				let Some(instance) = self.instance(regs[A0]) else {
					regs[A0] = WHO;
					return;
				};
				regs[A0] = unsafe { imports::pages(instance.outer, regs[A1], regs[A2], regs[A3]) };
			},
			HostCall::Invoke => {
				let mut args = InvokeBytes([0; 112]);
				if !read_parent_into(caller, regs[A1], &mut args.0)
					|| !write_parent_checked(caller, regs[A1], &args.0)
				{
					charge(gas_left, gas_costs::INVOKE as SignedGas);
					panic!("PVF invoke argument block is not writable");
				}
				let requested = SignedGas::from_le_bytes(args.0[..8].try_into().unwrap());
				let cost = requested
					.checked_add(gas_costs::INVOKE as SignedGas)
					.filter(|_| requested >= 0)
					.unwrap_or_else(|| {
						*gas_left = 0;
						panic!("PVF invoke gas overflow");
					});
				charge(gas_left, cost);
				let Some(instance) = self.instance(regs[A0]) else {
					regs[A0] = WHO;
					return;
				};
				let (a0, a1) = unsafe {
					imports::invoke(instance.outer, args.0.as_mut_ptr().cast())
				};
				if a0 < LOWEST_ERROR {
					let remaining = SignedGas::from_le_bytes(args.0[..8].try_into().unwrap());
					*gas_left = gas_left.saturating_add(remaining);
					write_parent(caller, regs[A1], &args.0);
				}
				regs[A0] = a0;
				// The ABI only changes A1 for a page fault or a host-call fault.
				if a0 == InvokeOutcomeCode::PageFault as u64
					|| a0 == InvokeOutcomeCode::HostCallFault as u64
				{
					regs[A1] = a1;
				}
			},
			HostCall::Expunge => {
				let handle = regs[A1];
				regs[A0] = match regs[A0] {
					// Remove an instance.
					0 => {
						if let Some(instance) = self.instance(handle) {
							let result = unsafe { imports::expunge(0, instance.outer) };
							if result < LOWEST_ERROR {
								self.instances[handle as usize] = None;
								self.modules[instance.module].as_mut().unwrap().instances -= 1;
								self.release_module_if_unused(instance.module);
							}
							result
						} else {
							WHO
						}
					},
					// Release a module handle.
					1 => {
						if let Some((index, module)) = self.module(handle) {
							let result = unsafe { imports::expunge(1, module.outer) };
							if result == OK {
								self.modules[index].as_mut().unwrap().held = false;
								self.release_module_if_unused(index);
							}
							result
						} else {
							WHO
						}
					},
					// Invalid expunge mode.
					_ => HUH,
				};
			},
			_ => unreachable!("non-nested host call"),
		}
	}
}

impl Drop for NestedVms {
	fn drop(&mut self) {
		for instance in self.instances.iter_mut().filter_map(Option::take) {
			let _ = unsafe { imports::expunge(0, instance.outer) };
		}
		for module in self.modules.iter_mut().filter_map(Option::take) {
			if module.held {
				let _ = unsafe { imports::expunge(1, module.outer) };
			}
		}
	}
}

fn read_parent_into(caller: &InstanceHandle, ptr: u64, bytes: &mut [u8]) -> bool {
	unsafe { imports::peek(caller.id(), bytes.as_mut_ptr(), ptr, bytes.len() as u64) == OK }
}

fn write_parent_checked(caller: &InstanceHandle, ptr: u64, bytes: &[u8]) -> bool {
	unsafe { imports::poke(caller.id(), bytes.as_ptr(), ptr, bytes.len() as u64) == OK }
}

fn read_parent(caller: &InstanceHandle, ptr: u64, len: u64) -> Vec<u8> {
	let len = u32::try_from(len).expect("PVF memory length exceeds u32");
	let mut bytes = vec![0; len as usize];
	assert!(read_parent_into(caller, ptr, &mut bytes), "PVF guest memory read failed");
	bytes
}

fn write_parent(caller: &InstanceHandle, ptr: u64, bytes: &[u8]) {
	assert!(write_parent_checked(caller, ptr, bytes), "PVF guest memory write failed");
}

fn charge(gas_left: &mut SignedGas, amount: SignedGas) {
	match gas_left.checked_sub(amount) {
		Some(left) if left >= 0 => *gas_left = left,
		_ => {
			*gas_left = 0;
			panic!("PVF nested invoke out of gas");
		},
	}
}
