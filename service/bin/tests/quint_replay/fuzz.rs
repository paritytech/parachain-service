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
	let mut command = Command::new("node");
	command.current_dir(root()).args([
		"scripts/quint-replay-stream.cjs",
		"service/bin/tests/fixtures/quint/fuzz.qnt",
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
	let revision = Command::new("git")
		.current_dir(root())
		.args(["rev-parse", "HEAD", "HEAD:vendor/polkadot-sdk-quint"])
		.output()
		.map_err(|e| e.to_string())?;
	report["revisions"] = Value::String(String::from_utf8_lossy(&revision.stdout).into_owned());
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
	let mut child =
		Generator(generator(seed, stride, count, steps).spawn().map_err(|e| e.to_string())?);
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
	let steps = number("QUINT_FUZZ_STEPS", 30);
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
