//! Differential host handling costs through the real JAM engine and service guest.
//! The explicit buffer sizes below are samples, not bounds on every host input.
use codec::Encode;
use executor::pj;
use jam_std_common::hash_raw;
use parachain_service::{
	pvf::{pvm::parse_pvf, PVF_ENTRY_POINT},
	refine::ParachainCandidate,
	work_digest::{validation_code_hash, ParachainWorkDigest, RefineLog},
};
use parachain_service_bin::{
	authorizer_blob, blob,
	mock::{good_config_for, good_token, good_trace, refine_args, refine_work_item},
};
use parachain_service_core::{
	types::{MessageId, ParaId},
	upward_message::{UpwardMessage, MAX_UPWARD_MESSAGE_BYTES},
};
use polkavm_common::{
	program::{asm, Instruction, InstructionSetKind, Reg},
	writer::ProgramBlobBuilder,
};

const RW_SIZE: u32 = 128 * 1024;
const SAMPLE: usize = 40 * 1024;
const BUF: u64 = 65536;
const NESTED: u64 = 8192;
const ARGS: u64 = 4096;
const HASH: u64 = 6000;
const MESSAGE: u64 = 16384;

fn call(code: &mut Vec<Instruction>, index: i32, args: &[u64], expected: Option<i32>) {
	let regs = [Reg::A0, Reg::A1, Reg::A2, Reg::A3, Reg::A4, Reg::A5];
	for (&reg, &value) in regs.iter().zip(args) {
		code.push(asm::load_imm64(reg, value));
	}
	code.push(asm::ecalli(index));
	if let Some(value) = expected {
		// A wrong host result must trap, rather than silently benchmark a failed call.
		let next = code.iter().filter(|op| op.opcode().starts_new_basic_block()).count() as u32 + 2;
		code.push(asm::branch_eq_imm(Reg::A0, value, next));
		code.push(Instruction::trap);
	}
}

fn program(case: &str, enabled: bool, preimage: &[u8], message: &[u8]) -> Vec<u8> {
	let memory = polkavm::MemoryMapBuilder::new(jam_types::PAGE_SIZE)
		.rw_data_size(RW_SIZE)
		.stack_size(4096)
		.build()
		.unwrap();
	let rw = u64::from(memory.rw_data_address());
	let mut inner = ProgramBlobBuilder::new(InstructionSetKind::JamV1);
	let inner_instructions =
		if case == "compile" { vec![Instruction::trap; 16 * 1024] } else { vec![asm::ret()] };
	inner.set_code(&inner_instructions, &[]);
	let inner_blob = inner.into_vec().unwrap();
	let inner_code = polkavm::ProgramParts::from_bytes(inner_blob.into())
		.unwrap()
		.code_and_jump_table;
	let mut data = vec![b'a'; RW_SIZE as usize];
	data[HASH as usize..HASH as usize + 32].copy_from_slice(&hash_raw(preimage));
	data[NESTED as usize..NESTED as usize + inner_code.len()].copy_from_slice(&inner_code);
	if case == "send" {
		data[MESSAGE as usize..MESSAGE as usize + message.len()].copy_from_slice(message);
	}
	data[ARGS as usize..ARGS as usize + 112].fill(0);
	data[ARGS as usize..ARGS as usize + 8].copy_from_slice(&100_000i64.to_le_bytes());
	data[ARGS as usize + 8..ARGS as usize + 16]
		.copy_from_slice(&polkavm::RETURN_TO_HOST.to_le_bytes());
	let mut code = Vec::new();
	let nested_module =
		matches!(case, "machine" | "peek" | "poke" | "pages" | "invoke" | "expunge");
	let nested_machine = matches!(case, "peek" | "poke" | "pages" | "invoke" | "expunge");
	if nested_module {
		call(&mut code, 9, &[0, rw + NESTED, inner_code.len() as u64, 0, 0], Some(0));
	}
	if nested_machine {
		call(&mut code, 10, &[0, 0], Some(0));
	}
	if matches!(case, "peek" | "poke") {
		call(&mut code, 13, &[0, 512, 16, 2], Some(0));
	}
	if case != "parent" || enabled {
		call(&mut code, 200, &[rw], None);
	}
	if case != "head" || enabled {
		call(&mut code, 201, &[rw, 4096], None);
	}
	if enabled {
		match case {
			"gas" => {
				for _ in 0..4 {
					call(&mut code, 0, &[], None);
				}
			},
			"grow_heap" => {
				call(&mut code, 1, &[u64::from(memory.heap_base()).div_ceil(4096) + 16], None)
			},
			"fetch" => {
				call(&mut code, 2, &[rw + BUF, 0, SAMPLE as u64, 4, 0, 0], Some(SAMPLE as i32))
			},
			"historical_lookup" => call(
				&mut code,
				7,
				&[u64::MAX, rw + HASH, rw + BUF, 0, SAMPLE as u64],
				Some(SAMPLE as i32),
			),
			"compile" => {
				call(&mut code, 9, &[0, rw + NESTED, inner_code.len() as u64, 0, 0], Some(0))
			},
			"machine" => call(&mut code, 10, &[0, 0], Some(0)),
			"peek" => call(&mut code, 11, &[0, rw + BUF, 512 * 4096, 65536], Some(0)),
			"poke" => call(&mut code, 12, &[0, rw + BUF, 512 * 4096, 65536], Some(0)),
			"pages" => call(&mut code, 13, &[0, 512, 16, 2], Some(0)),
			"invoke" => call(&mut code, 14, &[0, rw + ARGS], Some(0)),
			"expunge" => call(&mut code, 15, &[0, 0], Some(0)),
			"export" => call(&mut code, 8, &[rw + BUF, jam_types::SEGMENT_LEN as u64], Some(0)),
			"send" => call(&mut code, 202, &[rw + MESSAGE, message.len() as u64], None),
			"jam_log" => call(&mut code, 100, &[2, rw, 128, rw + BUF, 4096], None),
			"log" => call(&mut code, 204, &[2, rw, 128, rw + BUF, 4096], None),
			"report_error" => call(&mut code, 203, &[rw + BUF, 1024], None),
			"parent" | "head" => {},
			_ => unreachable!(),
		}
	}
	code.push(asm::ret());
	let mut builder = ProgramBlobBuilder::new(InstructionSetKind::JamV1);
	builder.set_rw_data_size(RW_SIZE);
	builder.set_stack_size(4096);
	builder.set_rw_data(data);
	builder.set_code(&code, &[]);
	builder.add_export_by_instruction(0, PVF_ENTRY_POINT.as_bytes());
	let blob = builder.into_vec().unwrap();
	assert!(parse_pvf(&blob).is_ok());
	blob
}

fn measure(case: &str, enabled: bool, preimage: &[u8], message: &[u8]) -> u64 {
	let service = blob();
	let pvf = program(case, enabled, preimage, message);
	let para = ParaId::new(0);
	let payload =
		ParachainCandidate { para_id: para, validation_code: validation_code_hash(&pvf) }.encode();
	let mut item = refine_work_item(&service, payload, vec![preimage.to_vec()]);
	if case == "export" && enabled {
		item.0.export_count = 1;
	}
	let (engine, hash, mut ctx) = refine_args(
		&service,
		&authorizer_blob(),
		good_config_for(vec![para]),
		good_token(),
		good_trace(),
		vec![item],
		&[&pvf, preimage],
		0,
	);
	let outcome =
		pj::refine(&engine, hash, &mut ctx).expect("real-engine refine must return a digest");
	let expected_error = if !enabled && matches!(case, "parent" | "head") {
		Some(RefineLog::InvalidHeadDeclaration)
	} else if enabled && case == "report_error" {
		Some(RefineLog::Opaque(vec![b'a'; 1024].try_into().unwrap()))
	} else {
		None
	};
	match (outcome.digest, expected_error) {
		(ParachainWorkDigest::Err { error, .. }, Some(expected)) => assert_eq!(error, expected),
		(ParachainWorkDigest::Ok { parent_head_hash, head_data, upward_messages, .. }, None) => {
			assert_eq!(parent_head_hash, [b'a'; 32]);
			assert_eq!(&head_data[..], &[b'a'; 4096]);
			if enabled && case == "send" {
				assert_eq!(upward_messages.len(), 1);
				assert_eq!(upward_messages[0].encode(), message);
			} else {
				assert!(upward_messages.is_empty());
			}
		},
		(other, expected) => panic!("unexpected digest {other:?}, expected error {expected:?}"),
	}
	outcome.gas_used
}

#[test]
fn measured_host_calls_works() {
	use parachain_service::pvf::executor::host_call_gas_cost;
	use parachain_service_core::host_call::HostCall;
	let preimage = vec![b'p'; SAMPLE];
	let message = (
		MessageId::from(0u64),
		UpwardMessage::SetKV { key: vec![1], value: vec![2; MAX_UPWARD_MESSAGE_BYTES - 16] },
	)
		.encode();
	// The §4.3 budget counts the encoded `(message_id, message)` pair.
	assert!(message.len() <= MAX_UPWARD_MESSAGE_BYTES);
	for (case, call) in [
		("gas", HostCall::Gas),
		("grow_heap", HostCall::GrowHeap),
		("fetch", HostCall::Fetch),
		("historical_lookup", HostCall::HistoricalLookup),
		("compile", HostCall::Compile),
		("machine", HostCall::Machine),
		("peek", HostCall::Peek),
		("poke", HostCall::Poke),
		("pages", HostCall::Pages),
		("invoke", HostCall::Invoke),
		("expunge", HostCall::Expunge),
		("export", HostCall::Export),
		("parent", HostCall::SetParentHeadHash),
		("head", HostCall::SetHead),
		("send", HostCall::SendUpwardMessage),
		("jam_log", HostCall::JamLog),
		("log", HostCall::Log),
		("report_error", HostCall::ReportError),
	] {
		let baseline = measure(case, false, &preimage, &message);
		let measured = measure(case, true, &preimage, &message);
		let count = if case == "gas" { 4 } else { 1 };
		let delta = i128::from(measured) - i128::from(baseline);
		eprintln!("{case}: baseline={baseline} measured={measured} delta={delta} calls={count} per_call={:.2}", delta as f64 / count as f64);
		// Aborting skips successful digest assembly.
		let cost = if case == "report_error" {
			i128::from(measured)
		} else {
			assert!(delta > 0, "{case}: differential must include handling");
			(delta + count - 1) / count
		};
		assert!(
			i128::from(host_call_gas_cost(&call)) >= cost * 5 / 4,
			"{case}: charge must retain 25% measured headroom",
		);
	}
}
