//! Real-engine Refine gas measurements. Run with `--test refine_gas_reserve -- --nocapture`.
//!
//! Total gas includes child execution and host handling, so it is a conservative
//! estimate of service overhead for these inputs, not a universal bound: PVF code
//! and memory can be much larger, and the outer execution may run out of gas.

use codec::{Decode, Encode};
use executor::pj;
use frameless::{blake2_256, hash_state, BlockData, Config, HeadData, State, ValidationParams};
use parachain_service::{
	refine::ParachainCandidate,
	work_digest::{validation_code_hash, ParachainWorkDigest},
};
use parachain_service_bin::{
	authorizer_blob, blob,
	mock::{good_config_for, good_token, good_trace, refine_args, refine_work_item},
};
use parachain_service_core::{
	types::ParaId,
	upward_message::{UpwardMessage, UpwardMessages},
};
use refine_replay_pvf::Call;

fn measure(name: &str, pvf: &[u8], pov: Vec<u8>) -> ParachainWorkDigest {
	let service = blob();
	let para_id = ParaId::new(0);
	let code = validation_code_hash(pvf);
	let payload = ParachainCandidate { para_id, validation_code: code }.encode();
	let pov_len = pov.len();
	let item = refine_work_item(&service, payload, vec![pov]);
	let (engine, code_hash, mut context) = refine_args(
		&service,
		&authorizer_blob(),
		good_config_for(vec![para_id]),
		good_token(),
		good_trace(),
		vec![item],
		&[pvf],
		0,
	);
	let outcome = pj::refine(&engine, code_hash, &mut context).expect("refine execution failed");
	let digest_len = outcome.digest.encode().len();
	eprintln!(
		"{name}: gas_used={}, pvf_bytes={}, pov_bytes={pov_len}, digest_bytes={digest_len}",
		outcome.gas_used,
		pvf.len(),
	);
	assert!(
		outcome.gas_used * 5 / 4 <= parachain_service::pvf::pvm::REFINE_GAS_RESERVE,
		"{name}: reserve must retain 25% measured headroom",
	);
	assert_eq!(outcome.digest.para_id(), para_id);
	assert_eq!(outcome.digest.validation_code(), code);
	outcome.digest
}
// A minimal executable with a real RO data image. Padding is part of the
// container's mapped data section, not ignored bytes after the blob.
fn synthetic_pvf(ro_bytes: usize) -> Vec<u8> {
	use polkavm_common::{
		abi::MemoryMapBuilder,
		program::{asm, InstructionSetKind, Reg},
		writer::ProgramBlobBuilder,
	};

	let ro_size = u32::try_from(ro_bytes).unwrap();
	let address = MemoryMapBuilder::new(jam_types::PAGE_SIZE)
		.ro_data_size(ro_size)
		.build()
		.unwrap()
		.ro_data_address();
	let mut builder = ProgramBlobBuilder::new(InstructionSetKind::JamV1);
	builder.set_ro_data_size(ro_size);
	builder.set_ro_data(vec![0; ro_bytes]);
	builder.set_code(
		&[
			asm::load_imm(Reg::A0, address as i32),
			asm::ecalli(200),
			asm::load_imm(Reg::A0, address as i32),
			asm::load_imm(Reg::A1, 0),
			asm::ecalli(201),
			asm::jump_indirect(Reg::RA, 0),
		],
		&[],
	);
	builder.add_export_by_instruction(0, parachain_service::pvf::PVF_ENTRY_POINT.as_bytes());
	builder.into_vec().expect("valid synthetic PVF")
}

fn measure_synthetic(name: &str, ro_bytes: usize) {
	let pvf = synthetic_pvf(ro_bytes);
	let digest = measure(name, &pvf, Vec::new());
	match digest {
		ParachainWorkDigest::Ok { parent_head_hash, head_data, upward_messages, lookup_anchor, .. } => {
			assert_eq!(parent_head_hash, [0; 32]);
			assert!(head_data.is_empty());
			assert_eq!(upward_messages, UpwardMessages::new());
			assert_eq!(lookup_anchor, 0);
		},
		other => panic!("expected successful synthetic digest, got {other:?}"),
	}
}

#[test]
fn measured_scenarios_works() {
	measure_synthetic("minimal_pvf", 32);
	// Close to the 16 MiB code cap without allocating extreme guest memory.
	measure_synthetic("large_ro_pvf", 15 * 1024 * 1024);

	let pvf = cargo_jam_build::blob("refine-replay-pvf");
	let parent = [0x42; 32];
	// The smallest successful script makes only the two mandatory declarations.
	let script = vec![Call::Parent(parent), Call::Head(Vec::new())];
	let digest = measure("minimal_script", &pvf, script.encode());
	match digest {
		ParachainWorkDigest::Ok { parent_head_hash, head_data, upward_messages, lookup_anchor, .. } => {
			assert_eq!(parent_head_hash, parent);
			assert!(head_data.is_empty());
			assert_eq!(upward_messages, UpwardMessages::new());
			assert_eq!(lookup_anchor, 0);
		},
		other => panic!("expected successful minimal digest, got {other:?}"),
	}

	// Exercise bounded input copying and digest assembly near the output budget.
	let head = vec![0x55; 4096];
	let message = UpwardMessage::SetKV { key: vec![0x77; 64], value: vec![0x99; 32 * 1024] };
	let script = vec![Call::Parent(parent), Call::Head(head.clone()), Call::Send(message.encode())];
	let digest = measure("large_output", &pvf, script.encode());
	match digest {
		ParachainWorkDigest::Ok { parent_head_hash, head_data, upward_messages, lookup_anchor, .. } => {
			assert_eq!(parent_head_hash, parent);
			assert_eq!(head_data.into_inner(), head);
			assert_eq!(upward_messages, UpwardMessages::try_from(vec![message]).unwrap());
			assert_eq!(lookup_anchor, 0);
		},
		other => panic!("expected successful large digest, got {other:?}"),
	}

	// A separately built realistic runtime exercises a different, larger program.
	let pvf = parachain_service_bin::frameless_pvf();
	let config = Config::Coretime;
	let parent = HeadData {
		number: 0,
		parent_hash: [0; 32],
		post_state: hash_state(&State { config: config.clone(), counter: 0 }),
	};
	let params = ValidationParams {
		parent_head: parent.encode(),
		block_data: BlockData { state: State { config: config.clone(), counter: 0 }, add: 512 }.encode(),
	};
	let digest = measure("frameless", &pvf, params.encode());
	match digest {
		ParachainWorkDigest::Ok { parent_head_hash, head_data, upward_messages, lookup_anchor, .. } => {
			assert_eq!(parent_head_hash, blake2_256(&parent.encode()));
			assert_eq!(upward_messages, UpwardMessages::new());
			assert_eq!(lookup_anchor, 0);
			let head = HeadData::decode(&mut &head_data[..]).expect("invalid frameless head");
			assert_eq!(head.number, 1);
			assert_eq!(head.parent_hash, parent.hash());
			assert_eq!(head.post_state, hash_state(&State { config, counter: 512 }));
		},
		other => panic!("expected successful frameless digest, got {other:?}"),
	}
}
