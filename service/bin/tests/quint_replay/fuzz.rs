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
		"storage" => "service/bin/tests/fixtures/quint/storage_fuzz.qnt",
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
		let envelope: Value =
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
	let stream = generator(63304, 1, 1, 50).output().expect("stream generator");
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
	let stream = generator(8, 1, 1, 50).output().expect("stream generator");
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
	let stream = generator(1, 1, 5, 30).output().expect("stream generator");
	assert!(stream.status.success(), "{}", String::from_utf8_lossy(&stream.stderr));
	let mut stops = std::collections::BTreeSet::new();
	let mut rejected = false;
	let mut exact = false;
	for line in stream.stdout.split(|b| *b == b'\n').filter(|line| !line.is_empty()) {
		let envelope: Value = serde_json::from_slice(line).unwrap();
		let trace = &envelope["trace"];
		for state in trace["states"].as_array().unwrap() {
			stops.insert(super::itf::replay::integer(&state["replayInterrupt"]).unwrap());
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
	let stream = generator_profile("storage", 1, 1, 10, 30).output().expect("storage generator");
	assert!(stream.status.success());
	let mut reasons = std::collections::BTreeSet::new();
	let mut failed_heads = false;
	for line in String::from_utf8(stream.stdout).unwrap().lines() {
		let envelope: Value = serde_json::from_str(line).unwrap();
		for frame in envelope["trace"]["states"].as_array().unwrap() {
			failed_heads |= !frame["replayFailedHeads"].as_array().unwrap().is_empty();
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
	assert!(failed_heads, "campaign must reject a head write");
	assert!(reasons.contains("KVWrite") && reasons.contains("QueueWrite"), "{reasons:?}");
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
	assert!(recovered && arrivals && skipped && resumed,
  "host-budget coverage: recovered={recovered}, arrivals={arrivals}, skipped={skipped}, resumed={resumed}");
}
