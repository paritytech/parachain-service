//! Opt-in streaming differential fuzzing. Each worker owns a persistent Quint
//! process and isolated Rust storage; stdout pipes provide bounded backpressure.
use std::{
	env, fs,
	io::{BufRead, BufReader, Read},
	path::{Path, PathBuf},
	process::{Child, Command, Stdio},
	sync::atomic::{AtomicBool, Ordering},
	thread,
};

use serde_json::Value;

use super::itf::replay;

mod progress;
use progress::Progress;

fn root() -> PathBuf {
	Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn number(name: &str, default: u64) -> u64 {
	env::var(name)
		.map(|v| v.parse().unwrap_or_else(|_| panic!("invalid {name}")))
		.unwrap_or(default)
}

fn generator(seed: u64, stride: u64, count: u64, steps: u64) -> Command {
	generator_profile("fuzz", seed, stride, count, steps)
}

fn generator_profile(profile: &str, seed: u64, stride: u64, count: u64, steps: u64) -> Command {
	let input = match profile {
		"fuzz" => "service/bin/tests/fixtures/quint/fuzz.qnt",
		"designation" => "service/bin/tests/fixtures/quint/designation_fuzz.qnt",
		"self_payment" => "service/bin/tests/fixtures/quint/self_payment_fuzz.qnt",
		"outgoing_boundary" => "service/bin/tests/fixtures/quint/outgoing_boundary_fuzz.qnt",
		"storage" => "service/bin/tests/fixtures/quint/storage_fuzz.qnt",
		"code_storage" => "service/bin/tests/fixtures/quint/code_storage_fuzz.qnt",
		_ => panic!("unknown QUINT_FUZZ_PROFILE: {profile}"),
	};
	let mut command = Command::new("node");
	command.current_dir(root()).args([
		"scripts/quint-replay-stream.cjs",
		input,
		&seed.to_string(),
		&stride.to_string(),
		&count.to_string(),
		&steps.to_string(),
	]);
	if profile == "outgoing_boundary" {
		command.arg("outgoing_boundary_fuzz");
	} else if profile == "self_payment" {
		command.arg("self_payment_fuzz");
	}
	command.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::inherit());
	command
}

struct Generator(Child);
impl Drop for Generator {
	fn drop(&mut self) {
		// In particular, stop a generator blocked on a full pipe after a mismatch.
		let _ = self.0.kill();
		let _ = self.0.wait();
	}
}

fn preserve(envelope: &Value, error: &str) -> Result<PathBuf, String> {
	let directory = env::var_os("QUINT_FUZZ_FAILURE_DIR")
		.map(PathBuf::from)
		.unwrap_or_else(|| root().join("target/quint-fuzz"));
	fs::create_dir_all(&directory).map_err(|e| e.to_string())?;
	let seed = envelope["seed"].as_str().unwrap_or("unknown");
	let path = directory.join(format!("failure-{}-{seed}.json", std::process::id()));
	let mut report = envelope.clone();
	report["error"] = Value::String(error.into());
	let revisions = if let Some(path) = env::var_os("QUINT_FUZZ_REVISIONS_FILE") {
		fs::read_to_string(path).map_err(|e| e.to_string())?
	} else {
		let revision = Command::new("git")
			.current_dir(root())
			.args(["rev-parse", "HEAD", "HEAD:vendor/polkadot-sdk-quint"])
			.output()
			.map_err(|e| e.to_string())?;
		String::from_utf8_lossy(&revision.stdout).into_owned()
	};
	report["revisions"] = Value::String(revisions);
	fs::write(&path, serde_json::to_vec(&report).map_err(|e| e.to_string())?)
		.map_err(|e| e.to_string())?;
	Ok(path)
}

fn worker(
	seed: u64,
	stride: u64,
	count: u64,
	steps: u64,
	stop: &AtomicBool,
	progress: &Progress,
) -> Result<u64, String> {
	let mut child = Generator(
		generator_profile(
			&env::var("QUINT_FUZZ_PROFILE").unwrap_or_else(|_| "fuzz".into()),
			seed,
			stride,
			count,
			steps,
		)
		.spawn()
		.map_err(|e| e.to_string())?,
	);
	let mut reader = BufReader::new(child.0.stdout.take().ok_or("missing generator stdout")?);
	let mut completed = 0;
	let mut line = String::new();
	loop {
		if stop.load(Ordering::Relaxed) {
			return Ok(completed);
		}
		line.clear();
		if reader.read_line(&mut line).map_err(|e| e.to_string())? == 0 {
			break;
		}
		let mut envelope: Value =
			serde_json::from_str(&line).map_err(|e| format!("invalid Quint stream: {e}"))?;
		let expected_seed = seed
			.checked_add(completed.checked_mul(stride).ok_or("seed overflow")?)
			.ok_or("seed overflow")?
			.to_string();
		if envelope["seed"].as_str() != Some(expected_seed.as_str()) ||
			envelope["steps"].as_u64() != Some(steps) ||
			envelope["version"] != "0.32.0"
		{
			return Err("Quint stream metadata differs from request".into());
		}
		let trace = &envelope["trace"];
		if trace["states"].as_array().map(|s| s.len() as u64) != Some(steps + 1) {
			return Err("Quint returned a truncated trace".into());
		}
		super::instruction_gas::sample(&mut envelope["trace"], expected_seed.parse().unwrap());
		let trace = &envelope["trace"];
		let result =
			std::panic::catch_unwind(|| replay::document_trace(trace)).unwrap_or_else(|panic| {
				Err(format!(
					"replay panicked: {}",
					panic
						.downcast_ref::<String>()
						.map(String::as_str)
						.or_else(|| panic.downcast_ref::<&str>().copied())
						.unwrap_or("unknown panic")
				))
			});
		if let Err(error) = result {
			stop.store(true, Ordering::Relaxed);
			let artifact = preserve(&envelope, &error).map_err(|save| {
				format!("seed {expected_seed}: {error}; could not save failure: {save}")
			})?;
			return Err(format!("seed {expected_seed}: {error}; saved {}", artifact.display()));
		}
		envelope["generation_ms"].as_f64().ok_or("missing generation timing")?;
		completed += 1;
		progress.completed();
	}
	let status = child.0.wait().map_err(|e| e.to_string())?;
	if !status.success() || count == 0 || completed != count {
		return Err(format!(
			"Quint worker {seed} exited {status} after {completed}/{count} traces"
		));
	}
	Ok(completed)
}

#[test]
#[ignore = "long-running fuzz test; requires Node and Quint 0.32.0"]
fn generated_traces_works() {
	let count = number("QUINT_FUZZ_TRACES", 100); // zero means run until failure/interruption
	let steps = number("QUINT_FUZZ_STEPS", 15);
	let seed = number("QUINT_FUZZ_SEED", 1);
	let workers = number("QUINT_FUZZ_WORKERS", 1);
	assert!(workers > 0 && steps > 0 && steps <= 10000, "invalid worker/step limit");
	// Keep numeric CLI limits exactly representable in JavaScript.
	assert!(count <= (1u64 << 53) - 1 && workers <= (1u64 << 53) - 1);
	// Build once before launching workers: a build failure is a setup error, not
	// a trace mismatch, and must not poison the shared blob cache mid-replay.
	let _ = parachain_service_bin::hash();
	let stop = AtomicBool::new(false);
	let progress = Progress::new(count, steps);
	let results = thread::scope(|scope| {
		let handles: Vec<_> = (0..workers.min(if count == 0 { workers } else { count }))
			.map(|i| {
				let stop = &stop;
				let progress = &progress;
				scope.spawn(move || {
					let n = if count == 0 {
						0
					} else {
						count / workers + u64::from(i < count % workers)
					};
					let result = worker(
						seed.checked_add(i).expect("seed overflow"),
						workers,
						n,
						steps,
						stop,
						progress,
					);
					if result.is_err() {
						stop.store(true, Ordering::Relaxed);
					}
					result
				})
			})
			.collect();
		handles
			.into_iter()
			.map(|h| h.join().expect("fuzz worker panicked"))
			.collect::<Vec<_>>()
	});
	let mut completed = 0;
	let mut failures = Vec::new();
	for result in results {
		match result {
			Ok(n) => completed += n,
			Err(e) => failures.push(e),
		}
	}
	progress.finish(failures.is_empty() && completed == count);
	assert!(failures.is_empty(), "{}", failures.join("\n"));
	assert_eq!(completed, count);
}

#[test]
#[ignore = "reads a saved trace/envelope from QUINT_REPLAY_TRACE or stdin"]
fn replay_input_works() {
	let input = match env::var_os("QUINT_REPLAY_TRACE") {
		Some(path) => fs::read_to_string(path).expect("read saved trace"),
		None => {
			let mut s = String::new();
			std::io::stdin().read_to_string(&mut s).unwrap();
			s
		},
	};
	let mut value: Value = serde_json::from_str(&input).expect("parse trace");
	let mut trace = value.as_object_mut().expect("trace document").remove("trace").unwrap_or(value);
	trace.as_object_mut().expect("ITF document").remove("#meta");
	for state in trace["states"].as_array_mut().expect("states") {
		state.as_object_mut().expect("state").remove("#meta");
	}
	replay::document_trace(&trace).expect("Quint and Rust should agree");
}

#[test]
#[ignore = "checks pinned internal streaming API against the Quint CLI; requires /dev/stdout"]
fn stream_matches_cli_works() {
	let stream = generator(1, 3, 2, 10).output().expect("stream generator");
	assert!(stream.status.success());
	let stream = String::from_utf8(stream.stdout).unwrap();
	let lines: Vec<_> = stream.lines().collect();
	assert_eq!(lines.len(), 2);
	for (line, seed) in lines.into_iter().zip(["1", "4"]) {
		let stream: Value = serde_json::from_str(line).unwrap();
		assert_eq!(stream["seed"], seed);
		let cli = Command::new("quint")
			.current_dir(root())
			.args([
				"run",
				"service/bin/tests/fixtures/quint/fuzz.qnt",
				"--init",
				"replayInit",
				"--step",
				"replayStep",
				"--backend",
				"typescript",
				"--seed",
				seed,
				"--max-samples",
				"1",
				"--max-steps",
				"10",
				"--out-itf",
				"/dev/stdout",
				"--verbosity",
				"0",
			])
			.output()
			.expect("Quint CLI");
		assert!(cli.status.success(), "{}", String::from_utf8_lossy(&cli.stderr));
		let mut cli: Value = serde_json::from_slice(&cli.stdout).unwrap();
		for state in cli["states"].as_array_mut().unwrap() {
			state.as_object_mut().unwrap().remove("#meta");
		}
		assert_eq!(stream["trace"]["states"], cli["states"]);
	}
}

#[test]
#[ignore = "checks generated validator-key coverage; requires Node and Quint 0.32.0"]
fn validator_keys_generated_works() {
	// A pinned seed that reaches a successful designation through replayStep,
	// guarding against accidentally removing keys or their effect from fuzzing.
	let stream = generator(5236461, 1, 1, 50).output().expect("stream generator");
	assert!(stream.status.success(), "{}", String::from_utf8_lossy(&stream.stderr));
	let envelope: Value = serde_json::from_slice(&stream.stdout).expect("stream envelope");
	let trace = &envelope["trace"];
	assert!(
		trace["states"].as_array().unwrap().iter().any(|state| {
			state["replayDesignate"].as_array().is_some_and(|keys| !keys.is_empty())
		}),
		"fuzz seed must exercise a successful validator-key designation"
	);
	replay::document_trace(trace).expect("generated validator keys should match Quint");
}

#[test]
#[ignore = "checks generated outgoing-transfer coverage; requires Node and Quint 0.32.0"]
fn outgoing_generated_works() {
	let stream = generator(1, 1, 1, 50).output().expect("stream generator");
	assert!(stream.status.success(), "{}", String::from_utf8_lossy(&stream.stderr));
	let envelope: Value = serde_json::from_slice(&stream.stdout).expect("stream envelope");
	let trace = &envelope["trace"];
	assert!(
		trace["states"].as_array().unwrap().iter().any(|state| {
			state["replayTransfers"]
				.as_array()
				.is_some_and(|transfers| !transfers.is_empty())
		}),
		"fuzz seed must emit a successful outgoing transfer"
	);
	replay::document_trace(trace).expect("generated transfers should match Quint");
}

#[test]
#[ignore = "checks generated service-upgrade coverage; requires Node and Quint 0.32.0"]
fn service_upgrade_generated_works() {
	let stream = generator(523656, 1, 1, 50).output().expect("stream generator");
	assert!(stream.status.success(), "{}", String::from_utf8_lossy(&stream.stderr));
	let envelope: Value = serde_json::from_slice(&stream.stdout).expect("stream envelope");
	let trace = &envelope["trace"];
	let states = trace["states"].as_array().unwrap();
	assert!(
		states
			.iter()
			.any(|state| state["svc"]["serviceCodeHash"]["hashBytes"]["#bigint"] != "0"),
		"fuzz seed must install new service code"
	);
	replay::document_trace(trace).expect("generated service upgrades should match Quint");
}

#[test]
#[ignore = "checks generated mixed-invocation coverage; requires Node and Quint 0.32.0"]
fn mixed_generated_works() {
	let stream = generator(1, 1, 1, 15).output().expect("stream generator");
	assert!(stream.status.success(), "{}", String::from_utf8_lossy(&stream.stderr));
	let envelope: Value = serde_json::from_slice(&stream.stdout).expect("stream envelope");
	let trace = &envelope["trace"];
	assert!(
		trace["states"].as_array().unwrap().iter().any(|state| {
			state["replayIncoming"].as_array().is_some_and(|items| !items.is_empty()) &&
				state["lastStepWorkResults"].as_array().is_some_and(|items| !items.is_empty())
		}),
		"fuzz seed must combine arrivals and work reports in one invocation"
	);
	replay::document_trace(trace).expect("generated mixed invocation should match Quint");
}

#[test]
#[ignore = "checks gas and checkpoint coverage; requires Node and Quint 0.32.0"]
fn gas_generated_works() {
	let stream = generator(1, 1, 20, 30).output().expect("stream generator");
	assert!(stream.status.success(), "{}", String::from_utf8_lossy(&stream.stderr));
	let mut stops = std::collections::BTreeSet::new();
	let mut rejected = false;
	let mut exact = false;
	for line in stream.stdout.split(|b| *b == b'\n').filter(|line| !line.is_empty()) {
		let envelope: Value = serde_json::from_slice(line).unwrap();
		let trace = &envelope["trace"];
		for state in trace["states"].as_array().unwrap() {
			if state["replayPanic"] != true {
				stops.insert(super::itf::replay::integer(&state["replayInterrupt"]).unwrap());
			}
			for limit in state["replayGasLimits"].as_array().unwrap() {
				let limit = super::itf::replay::integer(limit).unwrap();
				// Cover both message-free reports and the mixed host action
				// with two messages (5,000,000 + 2 * 250,000 gas).
				rejected |= matches!(limit, 4_999_999 | 5_499_999);
				exact |= matches!(limit, 5_000_000 | 5_500_000);
			}
		}
		replay::document_trace(trace).expect("gas and checkpoint state should match Quint");
	}
	assert!(rejected && exact, "must reach below-budget and exact-budget reports");
	assert!(
		stops.is_superset(&[0, 1, 2].into()),
		"must interrupt first, middle, and last reports: {stops:?}"
	);
}

#[test]
#[ignore = "requires Quint 0.32.0"]
fn storage_generated_works() {
	let stream = generator_profile("storage", 1, 1, 30, 50).output().expect("storage generator");
	assert!(stream.status.success());
	let mut reasons = std::collections::BTreeSet::new();
	let mut failed_heads = false;
	let mut failed_queue = false;
	let mut rejected_writes = std::collections::BTreeSet::new();
	for line in String::from_utf8(stream.stdout).unwrap().lines() {
		let envelope: Value = serde_json::from_str(line).unwrap();
		for frame in envelope["trace"]["states"].as_array().unwrap() {
			failed_heads |= !frame["replayFailedHeads"].as_array().unwrap().is_empty();
			// This profile stays within the prepaid queue and never cleans it up.
			// Nonempty arrivals with no queue change therefore indicate a host rejection.
			failed_queue |= !frame["replayIncoming"].as_array().unwrap().is_empty() &&
				frame["svc"]["incomingTransfers"] == frame["prevSvc"]["incomingTransfers"];
			for failure in frame["replayFailedMessages"].as_array().unwrap() {
				let pair = failure["#tup"].as_array().unwrap();
				let report = super::itf::replay::integer(&pair[0]).unwrap() as usize;
				let message = super::itf::replay::integer(&pair[1]).unwrap() as usize;
				let msg = &frame["lastStepWorkResults"][report]["result"]["value"]["value"]["upwardMessages"]
					[message];
				let tag = msg["tag"].as_str().unwrap();
				let kind = if tag == "ParachainSetStateBalance" {
					let target = &msg["value"]["paraId"];
					let exists = frame["prevSvc"]["parachains"]["#map"]
						.as_array()
						.unwrap()
						.iter()
						.any(|entry| &entry[0] == target);
					if exists { "balance" } else { "registration" }
				} else {
					tag
				};
				rejected_writes.insert(kind.to_owned());
			}
			for entry in frame["replayStorageLogs"].as_array().unwrap() {
				for reason in entry["#tup"][1].as_array().unwrap() {
					reasons.insert(reason["tag"].as_str().unwrap().to_owned());
				}
			}
		}
		if let Err(error) = replay::document_trace(&envelope["trace"]) {
			let path = preserve(&envelope, &error).unwrap();
			panic!("{error}; {}", path.display());
		}
	}
	assert_eq!(
		rejected_writes,
		[
			"balance",
			"registration",
			"ParachainSetHead",
			"SetValidatorKeys",
			"ParachainSetValidationCode"
		]
		.into_iter()
		.map(str::to_owned)
		.collect(),
		"metadata rejection coverage"
	);
	assert!(failed_heads, "campaign must reject a head write");
	assert!(failed_queue, "campaign must reject a queue write");
	assert_eq!(reasons, std::collections::BTreeSet::from(["KVWrite".to_owned()]));
}

#[test]
#[ignore = "requires Node and Quint 0.32.0"]
fn deregistering_head_generated_works() {
	// Scaleway failure: a matching-parent candidate for a retained, deregistering
	// para must leave its head unchanged (frame 15).
	let stream = generator(3123773523, 1, 1, 15).output().expect("stream generator");
	assert!(stream.status.success(), "{}", String::from_utf8_lossy(&stream.stderr));
	let envelope: Value = serde_json::from_slice(&stream.stdout).expect("stream envelope");
	replay::document_trace(&envelope["trace"]).expect("deregistering head should remain frozen");
}

#[test]
#[ignore = "checks mixed host budgets; requires Node and Quint 0.32.0"]
fn host_budget_generated_works() {
	let stream = generator(1, 1, 10, 30).output().expect("mixed budget generator");
	assert!(stream.status.success(), "{}", String::from_utf8_lossy(&stream.stderr));
	let mut recovered = false;
	let mut arrivals = false;
	let mut skipped = false;
	let mut resumed = false;
	for line in String::from_utf8(stream.stdout).unwrap().lines() {
		let envelope: Value = serde_json::from_str(line).unwrap();
		let states = envelope["trace"]["states"].as_array().unwrap();
		for pair in states.windows(2) {
			let state = &pair[1];
			let active = state["replayHostBudget"]["#bigint"] != "-1";
			let rejects =
				state["replayHostRejects"]["#bigint"].as_str().unwrap().parse::<u64>().unwrap();
			if active && rejects > 0 {
				recovered |= state["replayInterrupt"]["#bigint"] != "-1";
				arrivals |= !state["replayIncoming"].as_array().unwrap().is_empty();
			}
			if active {
				skipped |= state["replayGasLimits"][0]["#bigint"] == "5499999";
			}
			resumed |= pair[0]["replayHostBudget"]["#bigint"] != "-1" && !active;
		}
		if let Err(error) = replay::document_trace(&envelope["trace"]) {
			let path = preserve(&envelope, &error).unwrap();
			panic!("{error}; {}", path.display());
		}
	}
	assert!(
		recovered && arrivals && skipped && resumed,
		"host-budget coverage: recovered={recovered}, arrivals={arrivals}, skipped={skipped}, resumed={resumed}"
	);
}

#[test]
#[ignore = "checks creation/ejection in default replay fuzzing; requires Node and Quint 0.32.0"]
fn services_generated_works() {
	let mut created = false;
	let mut interrupted_creation = false;
	let mut refusals = std::collections::BTreeSet::new();
	// Fixed seeds reach creation recovery and all five reachable refusal classes.
	for seed in [31, 3665526] {
		let stream = generator(seed, 1, 1, 50).output().expect("service generator");
		assert!(stream.status.success(), "{}", String::from_utf8_lossy(&stream.stderr));
		let line = String::from_utf8(stream.stdout).unwrap();
		let envelope: Value = serde_json::from_str(&line).unwrap();
		for state in envelope["trace"]["states"].as_array().unwrap() {
			let has_creation = !state["replayCreations"].as_array().unwrap().is_empty();
			created |= has_creation;
			interrupted_creation |= has_creation && state["replayInterrupt"]["#bigint"] != "-1";
			let logs = state["svc"]["parachainLog"].to_string();
			for tag in [
				"CannotAfford",
				"IdTaken",
				"TargetIsSelf",
				"EjectUnknownService",
				"EjectNotSupervised",
			] {
				if logs.contains(&format!("\"tag\":\"{tag}\"")) {
					refusals.insert(tag);
				}
			}
		}
		if let Err(error) = replay::document_trace(&envelope["trace"]) {
			let path = preserve(&envelope, &error).unwrap();
			panic!("{error}; {}", path.display());
		}
	}
	assert!(
		created && interrupted_creation && refusals.len() == 5,
		"service coverage: created={created}, checkpoint={interrupted_creation}, refusals={refusals:?}"
	);
}

#[test]
#[ignore = "checks service preimage refusals in default replay fuzzing; requires Node and Quint 0.32.0"]
fn service_preimages_generated_works() {
	let stream = generator(2, 6, 2, 50).output().expect("service preimage generator");
	assert!(stream.status.success(), "{}", String::from_utf8_lossy(&stream.stderr));
	let mut refusals = std::collections::BTreeSet::new();
	for line in stream.stdout.split(|b| *b == b'\n').filter(|line| !line.is_empty()) {
		let envelope: Value = serde_json::from_slice(line).unwrap();
		for state in envelope["trace"]["states"].as_array().unwrap() {
			let logs = state["svc"]["parachainLog"].to_string();
			for tag in [
				"SolicitUnknownService",
				"SolicitNotSupervised",
				"StoreUnknownService",
				"StoreNotSupervised",
			] {
				if logs.contains(&format!("\"tag\":\"{tag}\"")) {
					refusals.insert(tag);
				}
			}
		}
		if let Err(error) = replay::document_trace(&envelope["trace"]) {
			let path = preserve(&envelope, &error).unwrap();
			panic!("{error}; {}", path.display());
		}
	}
	assert_eq!(refusals.len(), 4, "service preimage refusals: {refusals:?}");
}

#[test]
#[ignore = "checks service management refusals in default replay fuzzing; requires Node and Quint 0.32.0"]
fn service_management_generated_works() {
	let mut refusals = std::collections::BTreeSet::new();
	// These seeds reach both store errors and all three handoff errors.
	for seed in [31, 5] {
		let stream = generator(seed, 1, 1, 50).output().expect("service management generator");
		assert!(stream.status.success(), "{}", String::from_utf8_lossy(&stream.stderr));
		for line in stream.stdout.split(|b| *b == b'\n').filter(|line| !line.is_empty()) {
			let envelope: Value = serde_json::from_slice(line).unwrap();
			for state in envelope["trace"]["states"].as_array().unwrap() {
				let logs = state["svc"]["parachainLog"].to_string();
				for tag in [
					"HandoffUnknownService",
					"HandoffUnknownNewSupervisor",
					"HandoffNotSupervised",
					"StoreUnknownService",
					"StoreNotSupervised",
				] {
					if logs.contains(&format!("\"tag\":\"{tag}\"")) {
						refusals.insert(tag);
					}
				}
			}
			if let Err(error) = replay::document_trace(&envelope["trace"]) {
				let path = preserve(&envelope, &error).unwrap();
				panic!("{error}; {}", path.display());
			}
		}
	}
	assert_eq!(refusals.len(), 5, "service management refusals: {refusals:?}");
}

#[test]
#[ignore = "checks real Accumulate traps and recovery; requires Node and Quint 0.32.0"]
fn panic_recovery_generated_works() {
	let stream = generator(1, 1, 10, 30).output().expect("panic recovery generator");
	assert!(stream.status.success(), "{}", String::from_utf8_lossy(&stream.stderr));
	let mut stops = std::collections::BTreeSet::new();
	let mut retained_creation = false;
	let mut zero_gas = false;
	let mut resumed = false;
	for line in stream.stdout.split(|b| *b == b'\n').filter(|line| !line.is_empty()) {
		let envelope: Value = serde_json::from_slice(line).unwrap();
		for pair in envelope["trace"]["states"].as_array().unwrap().windows(2) {
			let state = &pair[1];
			if state["replayPanic"] == true {
				let stop = super::itf::replay::integer(&state["replayInterrupt"]).unwrap();
				stops.insert(stop);
				retained_creation |= !state["replayCreations"].as_array().unwrap().is_empty();
				zero_gas |= state["replayGasLimits"][stop as usize]["#bigint"] == "0";
			}
			resumed |= pair[0]["replayPanic"] == true &&
				state["replayInterrupt"]["#bigint"] == "-1" &&
				!state["lastStepWorkResults"].as_array().unwrap().is_empty();
		}
		if let Err(error) = replay::document_trace(&envelope["trace"]) {
			let path = preserve(&envelope, &error).unwrap();
			panic!("{error}; {}", path.display());
		}
	}
	assert!(
		stops.is_superset(&[0, 1, 2].into()) && retained_creation && zero_gas && resumed,
		"panic coverage: stops={stops:?}, creation={retained_creation}, zero_gas={zero_gas}, resumed={resumed}"
	);
}

#[test]
#[ignore = "checks sampled instruction gas in mixed traces; requires Node and Quint 0.32.0"]
fn instruction_gas_generated_works() {
	let stream = generator(11, 104729, 12, 30).output().expect("instruction gas generator");
	assert!(stream.status.success(), "{}", String::from_utf8_lossy(&stream.stderr));
	let mut stops = std::collections::BTreeSet::new();
	let mut resumed = false;
	for line in stream.stdout.split(|b| *b == b'\n').filter(|line| !line.is_empty()) {
		let mut envelope: Value = serde_json::from_slice(line).unwrap();
		let seed = envelope["seed"].as_str().unwrap().parse().unwrap();
		super::instruction_gas::sample(&mut envelope["trace"], seed);
		for pair in envelope["trace"]["states"].as_array().unwrap().windows(2) {
			if pair[1].get("replayGasSample").is_some() {
				stops.insert(super::itf::replay::integer(&pair[1]["replayInterrupt"]).unwrap());
			}
			resumed |= pair[0].get("replayGasSample").is_some() &&
				pair[1]["replayInterrupt"]["#bigint"] == "-1" &&
				!pair[1]["lastStepWorkResults"].as_array().unwrap().is_empty();
		}
		if let Err(error) = replay::document_trace(&envelope["trace"]) {
			let path = preserve(&envelope, &error).unwrap();
			panic!("{error}; {}", path.display());
		}
	}
	assert!(
		stops.is_superset(&[0, 1, 2].into()) && resumed,
		"instruction gas coverage: reports={stops:?}, resumed={resumed}"
	);
}

#[test]
#[ignore = "checks pre-checkpoint gas recovery in mixed traces; requires Node and Quint 0.32.0"]
fn pre_checkpoint_generated_works() {
	let stream = generator(1, 1, 20, 40).output().expect("pre-checkpoint generator");
	assert!(stream.status.success(), "{}", String::from_utf8_lossy(&stream.stderr));
	let (mut failures, mut credits, mut empty, mut due, mut resumed) =
		(0, false, false, false, false);
	for line in stream.stdout.split(|b| *b == b'\n').filter(|line| !line.is_empty()) {
		let mut envelope: Value = serde_json::from_slice(line).unwrap();
		let seed = envelope["seed"].as_str().unwrap().parse().unwrap();
		super::instruction_gas::sample(&mut envelope["trace"], seed);
		for pair in envelope["trace"]["states"].as_array().unwrap().windows(2) {
			if pair[1]["replayInterrupt"]["#bigint"] == "-2" {
				failures += 1;
				let incoming = pair[1]["replayIncoming"].as_array().unwrap();
				empty |= incoming.is_empty();
				credits |= incoming.iter().any(|t| t["amount"]["#bigint"] != "0");
				let slot = replay::integer(&pair[1]["now"]).unwrap();
				due |= pair[0]["svc"]["pendingAssignCores"]["#map"]
					.as_array()
					.unwrap()
					.iter()
					.any(|entry| replay::integer(&entry[1]).unwrap() <= slot);
			}
			resumed |= pair[0]["replayInterrupt"]["#bigint"] == "-2" &&
				pair[1]["replayInterrupt"]["#bigint"] == "-1" &&
				!pair[1]["lastStepWorkResults"].as_array().unwrap().is_empty();
		}
		if let Err(error) = replay::document_trace(&envelope["trace"]) {
			let path = preserve(&envelope, &error).unwrap();
			panic!("{error}; {}", path.display());
		}
	}
	assert!(
		failures > 0 && credits && empty && due && resumed,
		"pre-checkpoint coverage: failures={failures}, credits={credits}, empty={empty}, due={due}, resumed={resumed}"
	);
}

#[test]
#[ignore = "checks final-checkpoint recovery in mixed traces; requires Node and Quint 0.32.0"]
fn final_checkpoint_generated_works() {
	let stream = generator(1, 1, 20, 40).output().expect("final-checkpoint generator");
	assert!(stream.status.success(), "{}", String::from_utf8_lossy(&stream.stderr));
	let (mut failures, mut transfers, mut creations, mut resumed) = (0, false, false, false);
	for line in stream.stdout.split(|b| *b == b'\n').filter(|line| !line.is_empty()) {
		let mut envelope: Value = serde_json::from_slice(line).unwrap();
		let seed = envelope["seed"].as_str().unwrap().parse().unwrap();
		super::instruction_gas::sample(&mut envelope["trace"], seed);
		for pair in envelope["trace"]["states"].as_array().unwrap().windows(2) {
			let final_checkpoint = |state: &Value| {
				replay::integer(&state["replayInterrupt"]).unwrap() ==
					state["lastStepWorkResults"].as_array().unwrap().len() as i128
			};
			if final_checkpoint(&pair[1]) {
				failures += 1;
				transfers |= !pair[1]["replayTransfers"].as_array().unwrap().is_empty();
				creations |= !pair[1]["replayCreations"].as_array().unwrap().is_empty();
			}
			resumed |= final_checkpoint(&pair[0]) &&
				pair[1]["replayInterrupt"]["#bigint"] == "-1" &&
				!pair[1]["lastStepWorkResults"].as_array().unwrap().is_empty();
		}
		if let Err(error) = replay::document_trace(&envelope["trace"]) {
			let path = preserve(&envelope, &error).unwrap();
			panic!("{error}; {}", path.display());
		}
	}
	assert!(
		failures > 0 && transfers && creations && resumed,
		"final-checkpoint coverage: failures={failures}, transfers={transfers}, creations={creations}, resumed={resumed}"
	);
}

#[test]
#[ignore = "requires Quint 0.32.0"]
fn code_storage_generated_works() {
	let stream = generator_profile("code_storage", 1, 1, 30, 50)
		.output()
		.expect("code storage generator");
	assert!(stream.status.success());
	let mut failures = std::collections::BTreeSet::new();
	for line in String::from_utf8(stream.stdout).unwrap().lines() {
		let envelope: Value = serde_json::from_str(line).unwrap();
		for frame in envelope["trace"]["states"].as_array().unwrap() {
			let label = frame["replayCodeFailure"].as_str().unwrap();
			if !label.is_empty() {
				failures.insert(label.to_owned());
			}
		}
		if let Err(error) = replay::document_trace(&envelope["trace"]) {
			let path = preserve(&envelope, &error).unwrap();
			panic!("{error}; {}", path.display());
		}
	}
	for site in ["registry", "solicit", "forced-metadata", "announcement"] {
		assert!(failures.contains(site), "missing {site} rejection");
	}
}

#[test]
#[ignore = "requires Quint 0.32.0"]
fn mixed_head_queue_generated_works() {
	let mut head_recovery = false;
	let mut queue_recovery = false;
	let mut capacity = false;
	let mut resumed = false;
	for seed in [12, 21, 60] {
		let stream = generator(seed, 1, 1, 15).output().expect("mixed queue generator");
		assert!(stream.status.success());
		let envelope: Value = serde_json::from_slice(&stream.stdout).unwrap();
		for pair in envelope["trace"]["states"].as_array().unwrap().windows(2) {
			let s = &pair[1];
			let active = s["replayHostBudget"]["#bigint"] != "-1";
			let recovery = s["replayInterrupt"]["#bigint"] != "-1";
			head_recovery |=
				active && recovery && !s["replayHostFailedHeads"].as_array().unwrap().is_empty();
			queue_recovery |= active &&
				recovery && !s["replayIncoming"].as_array().unwrap().is_empty() &&
				s["svc"]["incomingTransfers"] == s["prevSvc"]["incomingTransfers"];
			capacity |= active && s["replayIncoming"].as_array().unwrap().len() > 1024;
			resumed |= pair[0]["replayHostBudget"]["#bigint"] != "-1" && !active;
		}
		if let Err(error) = replay::document_trace(&envelope["trace"]) {
			let path = preserve(&envelope, &error).unwrap();
			panic!("{error}; {}", path.display());
		}
	}
	assert!(head_recovery, "missing failed head write with gas recovery");
	assert!(queue_recovery, "missing failed queue write with gas recovery");
	assert!(capacity, "missing arrivals beyond reserved queue capacity");
	assert!(resumed, "missing ordinary invocation after a mixed host budget");
}

#[test]
#[ignore = "requires Quint 0.32.0"]
fn outgoing_boundary_generated_works() {
	let stream = generator_profile("outgoing_boundary", 1, 1, 10, 15)
		.output()
		.expect("outgoing boundary generator");
	assert!(stream.status.success());
	let mut boundaries = std::collections::BTreeSet::new();
	let mut recovered = false;
	let mut resumed = false;
	for line in stream.stdout.split(|b| *b == b'\n').filter(|line| !line.is_empty()) {
		let envelope: Value = serde_json::from_slice(line).unwrap();
		for pair in envelope["trace"]["states"].as_array().unwrap().windows(2) {
			let s = &pair[1];
			let active = s["replayHostBudget"]["#bigint"] != "-1";
			if active {
				let effects = s["replayTransfers"].as_array().unwrap();
				if effects.is_empty() {
					boundaries.insert("above");
				} else if replay::integer(&s["replayHostFree"]).unwrap() == 0 {
					boundaries.insert("exact");
				} else {
					boundaries.insert("below");
				}
				recovered |= s["replayInterrupt"]["#bigint"] == "2" && !effects.is_empty();
			}
			resumed |= pair[0]["replayHostBudget"]["#bigint"] != "-1" && !active;
		}
		if let Err(error) = replay::document_trace(&envelope["trace"]) {
			let path = preserve(&envelope, &error).unwrap();
			panic!("{error}; {}", path.display());
		}
	}
	assert_eq!(boundaries, ["below", "exact", "above"].into());
	assert!(recovered && resumed, "missing checkpoint recovery or ordinary resumption");
}

#[test]
#[ignore = "requires Quint 0.32.0"]
fn self_payment_generated_works() {
	let stream = generator_profile("self_payment", 1, 1, 10, 15)
		.output()
		.expect("self-payment generator");
	assert!(stream.status.success());
	let mut retained = false;
	let mut refused = false;
	let mut ordered = false;
	let mut resumed = false;
	for line in stream.stdout.split(|b| *b == b'\n').filter(|line| !line.is_empty()) {
		let envelope: Value = serde_json::from_slice(line).unwrap();
		for pair in envelope["trace"]["states"].as_array().unwrap().windows(2) {
			let s = &pair[1];
			let active = s["replayHostBudget"]["#bigint"] != "-1";
			let effects = s["replayTransfers"].as_array().unwrap();
			let self_payment = effects.iter().any(|t| t["dest"]["value"]["#bigint"] == "1");
			retained |= active && self_payment && s["replayInterrupt"]["#bigint"] == "2";
			refused |= active && !self_payment;
			ordered |= active && self_payment && effects.len() == 2;
			resumed |= pair[0]["replayHostBudget"]["#bigint"] != "-1" && !active;
		}
		if let Err(error) = replay::document_trace(&envelope["trace"]) {
			let path = preserve(&envelope, &error).unwrap();
			panic!("{error}; {}", path.display());
		}
	}
	assert!(
		retained && refused && ordered && resumed,
		"self-payment coverage: retained={retained}, refused={refused}, ordered={ordered}, resumed={resumed}"
	);
}

#[test]
#[ignore = "requires Quint 0.32.0"]
fn designation_privilege_generated_works() {
	let stream = generator_profile("designation", 1, 1, 10, 20)
		.output()
		.expect("designation generator");
	assert!(stream.status.success());
	let mut rejected = false;
	let mut staged = false;
	let mut resumed = false;
	for line in stream.stdout.split(|b| *b == b'\n').filter(|line| !line.is_empty()) {
		let envelope: Value = serde_json::from_slice(line).unwrap();
		for pair in envelope["trace"]["states"].as_array().unwrap().windows(2) {
			let s = &pair[1];
			let denied = s["replayCanDesignate"] == false;
			rejected |=
				denied && s["svc"]["parachainLog"].to_string().contains("DesignateRejected");
			staged |= denied && !s["svc"]["stagedValidatorKeys"].as_array().unwrap().is_empty();
			resumed |= pair[0]["replayCanDesignate"] == false &&
				!denied && !s["replayDesignate"].as_array().unwrap().is_empty();
		}
		if let Err(error) = replay::document_trace(&envelope["trace"]) {
			let path = preserve(&envelope, &error).unwrap();
			panic!("{error}; {}", path.display());
		}
	}
	assert!(
		rejected && staged && resumed,
		"missing privilege rejection, partial staging, or restoration"
	);
}
