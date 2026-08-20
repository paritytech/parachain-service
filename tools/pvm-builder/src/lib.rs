//! Builder logic for creating PVM code blobs for execution on the JAM PVM instances (service code
//! and authorizer code).
//!
//! Local reimplementation of `jam_pvm_builder`'s build orchestration with two behavioural changes:
//! 1. The `rustup` detection branch is deleted entirely — ambient `cargo` is always used
//!    unconditionally; no `cargo +<toolchain>` is ever invoked.
//! 2. `-Z json-target-spec` is appended to the cargo invocation when the ambient `cargo --version`
//!    is >= 1.95, mirroring
//!    `vendor/polkadot-sdk-companion/substrate/utils/wasm-builder/src/wasm_project.rs:1013-1036`.
//!
//! Everything else (the `--remap-path-prefix` reproducibility flags, the ELF→`.jam`-blob
//! packaging via `polkavm_linker`/`jam_program_blob_common`, the `SKIP_PVM_BUILDS` dummy-blob
//! path, the `RUSTC_WRAPPER` handling) carries over unchanged from the reference implementation.
//!
//! # Reproducibility
//!
//! Two pieces here drive cross-machine reproducibility of the produced blob:
//!
//! - **Path remapping (`build_encoded_rustflags`).** Adds `--remap-path-prefix` directives that
//!   rewrite `$HOME`, `$RUSTUP_HOME`, `$CARGO_HOME`, and each workspace-member path to stable roots
//!   (`~/`, `~/.rustup`, `~/.cargo`, `/crate/<name>`). This prevents the user's actual home, cargo
//!   cache, and workspace location from leaking into `file!()` strings and embedded debuginfo.
//!   Sufficient to make builds byte-identical across different machines running the same host OS
//!   (tested on Linux).
//!
//! - **`RUSTC_WRAPPER` forwarding (`build_pvm_blob`).** The inner cargo invocation does
//!   `env_clear()` and then selectively forwards a few env vars; `RUSTC_WRAPPER` is one of them
//!   when the caller sets it. When not specified, default wrapper is used. It normalises cross-OS
//!   (Linux vs macOS) blob output by rewriting cargo's per-crate `-Cmetadata` to a host-independent
//!   value. Default wrapper lives at `scripts/rustc-wrapper.sh` in this crate; see its header for
//!   details.

#![allow(clippy::unwrap_used)]

// This is jam-codec's `Encode` trait (re-exported at the crate root), NOT parity-scale-codec's
// `Encode`. `ConventionalMetadata` implements the jam-codec one.
use jam_codec::Encode;
use jam_program_blob_common::{ConventionalMetadata, CoreVmProgramBlob, CrateInfo, ProgramBlob};
use std::{
	fmt::Display,
	fs,
	path::{Path, PathBuf},
	process::Command,
	sync::OnceLock,
};

/// Program blob type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum BlobType {
	/// JAM service (`jam_pvm_common::Service`).
	Service,
	/// JAM authorizer (`jam_pvm_common::Authorizer`).
	Authorizer,
	/// CoreVM guest program (`corevm_guest` crate).
	CoreVmGuest,
	/// A generic runtime (e.g. a parachain validation function) with a custom export.
	Runtime,
}

impl Display for BlobType {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		std::fmt::Debug::fmt(self, f)
	}
}

impl BlobType {
	pub fn dispatch_table(&self) -> Vec<Vec<u8>> {
		match self {
			Self::Service => vec![b"refine_ext".into(), b"accumulate_ext".into()],
			Self::Authorizer => vec![b"is_authorized_ext".into()],
			Self::CoreVmGuest | Self::Runtime => Vec::new(),
		}
	}

	/// Get output file path for the specified crate name and output directory.
	pub fn output_file(&self, out_dir: &Path, crate_name: &str) -> PathBuf {
		let suffix = match self {
			Self::Service | Self::Authorizer | Self::Runtime => "jam",
			Self::CoreVmGuest => "corevm",
		};
		out_dir.join(format!("{crate_name}.{suffix}"))
	}
}

pub enum ProfileType {
	Debug,
	Release,
	Other(&'static str),
}
impl ProfileType {
	fn as_str(&self) -> &'static str {
		match self {
			ProfileType::Debug => "debug",
			ProfileType::Release => "release",
			ProfileType::Other(s) => s,
		}
	}
	fn to_arg(&self) -> String {
		match self {
			ProfileType::Debug => "--debug".into(),
			ProfileType::Release => "--release".into(),
			ProfileType::Other(s) => format!("--profile={s}"),
		}
	}

	/// Whether this profile should optimise for a small, opaque blob.
	/// Disabled for debug builds.
	fn is_release_like(&self) -> bool {
		!matches!(self, ProfileType::Debug)
	}
}

fn build_pvm_blob_in_build_script(
	crate_dir: &Path,
	blob_type: BlobType,
	inner_name: Option<String>,
	no_default_features: bool,
) {
	let out_dir: PathBuf = std::env::var("OUT_DIR").expect("No OUT_DIR").into();
	println!("cargo:rerun-if-env-changed=SKIP_PVM_BUILDS");
	println!("cargo:rerun-if-env-changed=PVM_BUILDER_STRIP");
	println!("cargo:rerun-if-env-changed=RUSTC_WRAPPER");
	if std::env::var_os("SKIP_PVM_BUILDS").is_some() {
		let crate_name = get_crate_info(crate_dir).name;
		let output_file = blob_type.output_file(&out_dir, &crate_name);
		fs::write(&output_file, []).expect("error creating dummy program blob");
		println!("cargo:rustc-env=PVM_BINARY_{crate_name}={}", output_file.display());
		let hash_output_file = out_dir.join("{crate_name}.hash");
		fs::write(&hash_output_file, [0_u8; 32]).expect("error creating dummy program blob hash");
		println!("cargo:rustc-env=PVM_BINARY_HASH_{crate_name}={}", hash_output_file.display());
	} else {
		println!("cargo:rerun-if-changed={}", crate_dir.to_str().unwrap());
		let (crate_name, output_file, hash_output_file) = build_pvm_blob(
			crate_dir,
			blob_type,
			&out_dir,
			ProfileType::Other("production"),
			inner_name,
			no_default_features,
		);
		println!("cargo:rustc-env=PVM_BINARY_{crate_name}={}", output_file.display());
		println!("cargo:rustc-env=PVM_BINARY_HASH_{crate_name}={}", hash_output_file.display());
	}
}

/// Build the service crate in `crate_dir` for PVM.
///
/// Outputs
/// - `{out_dir}/{crate_name}.jam` - JAM program blob,
/// - `{out_dir}/{crate_name}.polkavm` - PolkaVM program blob for debugging.
///
/// The blob may be included in the relevant crate by using the [`pvm_binary!`] macro from
/// `jam_pvm_builder`.
pub fn build_service(crate_dir: &Path) {
	build_pvm_blob_in_build_script(crate_dir, BlobType::Service, None, false);
}

/// Build the authorizer crate in `crate_dir` for PVM.
///
/// Outputs
/// - `{out_dir}/{crate_name}.jam` - JAM program blob,
/// - `{out_dir}/{crate_name}.polkavm` - PolkaVM program blob for debugging.
///
/// The blob may be included in the relevant crate by using the [`pvm_binary!`] macro from
/// `jam_pvm_builder`.
pub fn build_authorizer(crate_dir: &Path) {
	build_pvm_blob_in_build_script(crate_dir, BlobType::Authorizer, None, false);
}

/// Build a generic runtime crate in `crate_dir` for PVM, without a service/authorizer
/// dispatch table (e.g. the `runtimes/frameless` parachain validation function, which
/// exports its own `jam_validate_block` entry point).
///
/// Unlike [`build_service`]/[`build_authorizer`], the guest is built with
/// `--no-default-features`: runtimes gate their `std`-only host embedding on a `std`
/// feature, which must be off when compiling for the bare Riscv target.
pub fn build_runtime(crate_dir: &Path) {
	build_pvm_blob_in_build_script(crate_dir, BlobType::Runtime, None, true);
}

/// Build the `CARGO_ENCODED_RUSTFLAGS` value for the PVM build.
///
/// `--remap-path-prefix` directives prevent absolute paths (rust-src, cargo registry, workspace)
/// from leaking into `file!()` strings embedded in the PVM blob. Without them the blob - and
/// therefore its hash - is not reproducible across machines or toolchain installs.
///
/// Remap targets are chosen so PolkaVM's `source_cache` can resolve embedded paths back to local
/// files on whatever machine later inspects the blob: it strips a leading `~/` and joins with the
/// running process's own `$HOME`, so an embedded `~/.rustup/toolchains/<channel>/...` finds the
/// equivalent file under any user who has that toolchain installed via rustup. Workspace members
/// each get a stable `/crate/<name>` root.
///
/// Note: in release-like builds `panic_immediate_abort` (set in `build-std-features` below)
/// lowers panics to `abort()` without consuming the `&Location` propagated by `#[track_caller]`,
/// so the file/line/column literals are DCE'd. In debug builds those literals reach rodata, but
/// always in remapped form, which keeps cross-host reproducibility.
fn build_encoded_rustflags(crate_dir: &Path) -> String {
	// rustc >= 1.92 turned the `panic_immediate_abort` build-std feature into a real panic
	// strategy (`-C panic=immediate-abort`, see rust-lang/rust#146317); the old feature hard-errors
	// there. The strategy form sets both `cfg(panic = "abort")` and `cfg(panic =
	// "immediate-abort")`, so it is a drop-in replacement for the `-C panic=abort` +
	// build-std-feature pair.
	let mut flags: Vec<String> = if rustc_uses_new_panic_immediate_abort() {
		vec!["-Zunstable-options".into(), "-C".into(), "panic=immediate-abort".into()]
	} else {
		vec!["-C".into(), "panic=abort".into()]
	};

	// Order matters: rustc applies the LAST matching `--remap-path-prefix`, so the broadest
	// catch-all goes first and the most specific override goes last.
	let home = std::env::var("HOME").ok();
	if let Some(h) = home.as_deref() {
		flags.push(format!("--remap-path-prefix={h}=~"));
	}
	let rustup = std::env::var("RUSTUP_HOME")
		.ok()
		.or_else(|| home.as_deref().map(|h| format!("{h}/.rustup")));
	if let Some(p) = rustup {
		flags.push(format!("--remap-path-prefix={p}=~/.rustup"));
	}
	let cargo = std::env::var("CARGO_HOME")
		.ok()
		.or_else(|| home.as_deref().map(|h| format!("{h}/.cargo")));
	if let Some(p) = cargo {
		flags.push(format!("--remap-path-prefix={p}=~/.cargo"));
	}
	for (name, path) in workspace_members(crate_dir) {
		flags.push(format!("--remap-path-prefix={}=/crate/{name}", path.display()));
	}

	flags.join("\x1f")
}

/// Enumerate workspace members visible from `crate_dir` as `(name, manifest_dir)` pairs.
fn workspace_members(crate_dir: &Path) -> impl Iterator<Item = (String, PathBuf)> + use<> {
	let packages = (|| -> Option<Vec<serde_json::Value>> {
		let output = Command::new("cargo")
			.current_dir(crate_dir)
			.args(["metadata", "--no-deps", "--format-version", "1"])
			.output()
			.ok()
			.filter(|o| o.status.success())?;
		let mut meta: serde_json::Value = serde_json::from_slice(&output.stdout).ok()?;
		match meta.get_mut("packages")?.take() {
			serde_json::Value::Array(arr) => Some(arr),
			_ => None,
		}
	})()
	.unwrap_or_default();

	packages.into_iter().filter_map(|pkg| {
		let name = pkg.get("name")?.as_str()?.to_string();
		let manifest = pkg.get("manifest_path")?.as_str()?;
		let path = Path::new(manifest).parent()?.to_path_buf();
		Some((name, path))
	})
}

fn get_crate_info(crate_dir: &Path) -> CrateInfo {
	let read_manifest_output = Command::new("cargo")
		.current_dir(crate_dir)
		.arg("read-manifest")
		.output()
		.unwrap_or_else(|err| {
			panic!("Failed to run `cargo read-manifest` in {}: {err}", crate_dir.display());
		});
	if !read_manifest_output.status.success() {
		panic!(
			"Failed to read Cargo.toml manifest in {}:\n{}",
			crate_dir.display(),
			String::from_utf8_lossy(&read_manifest_output.stderr)
		);
	}
	let man = serde_json::from_slice::<serde_json::Value>(&read_manifest_output.stdout).unwrap();
	// read-manifest output should always contain a valid name/version
	let name = man.get("name").unwrap().as_str().unwrap().to_string();
	let version = man.get("version").unwrap().as_str().unwrap().to_string();
	// read-manifest output contains "license": null when no license is specified in the Cargo.toml
	let license = man
		.get("license")
		.unwrap()
		.as_str()
		.unwrap_or_else(|| {
			panic!("No license specified in Cargo.toml manifest in {}", crate_dir.display());
		})
		.to_string();
	// read-manifest output should always contain a valid authors list
	let authors = man
		.get("authors")
		.unwrap()
		.as_array()
		.unwrap()
		.iter()
		.map(|x| x.as_str().unwrap().to_owned())
		.collect::<Vec<String>>();
	CrateInfo { name, version, license, authors }
}

/// Returns true when the ambient `rustc --version` is >= 1.92.
///
/// rustc 1.92 (rust-lang/rust#146317) turned the `panic_immediate_abort` build-std feature into a
/// real panic strategy (`-C panic=immediate-abort`); using the old `-Z build-std-features=...`
/// flag hard-errors there. Older toolchains only know the old feature.
fn rustc_uses_new_panic_immediate_abort() -> bool {
	static RESULT: OnceLock<bool> = OnceLock::new();
	*RESULT.get_or_init(|| {
		let output = Command::new("rustc")
			.args(["--version"])
			.output()
			.ok()
			.filter(|o| o.status.success());
		let Some(output) = output else { return false };
		let stdout = String::from_utf8(output.stdout).unwrap_or_default();
		// "rustc 1.97.1 (8bab26f4f 2026-07-14)"
		let version = stdout.split_whitespace().nth(1).unwrap_or("");
		let mut parts = version.split('.');
		let major: u32 = parts.next().and_then(|s| s.parse().ok()).unwrap_or(0);
		let minor: u32 = parts.next().and_then(|s| s.parse().ok()).unwrap_or(0);
		major > 1 || (major == 1 && minor >= 92)
	})
}

/// Returns true when the ambient `cargo --version` is >= 1.95.
///
/// `--target` for the Riscv runtime points at a JSON target spec produced by
/// `polkavm-linker`. Rust 1.95 added `-Z json-target-spec`; later versions
/// require it to be opted into explicitly. Older versions don't know the flag,
/// so guard on the version. `RUSTC_BOOTSTRAP=1` is already set by the
/// `-Z build-std` block above (Riscv always opts into `build-std`).
///
/// Mirrors `vendor/polkadot-sdk-companion/substrate/utils/wasm-builder/src/wasm_project.rs:
/// 1013-1036`.
fn cargo_supports_json_target_spec() -> bool {
	static RESULT: OnceLock<bool> = OnceLock::new();
	*RESULT.get_or_init(|| {
		let output = Command::new("cargo")
			.args(["--version"])
			.output()
			.ok()
			.filter(|o| o.status.success());
		let Some(output) = output else { return false };
		let stdout = String::from_utf8(output.stdout).unwrap_or_default();
		// "cargo 1.97.1 (c980f4866 2026-06-30)"
		let version = stdout.split_whitespace().nth(1).unwrap_or("");
		let mut parts = version.split('.');
		let major: u32 = parts.next().and_then(|s| s.parse().ok()).unwrap_or(0);
		let minor: u32 = parts.next().and_then(|s| s.parse().ok()).unwrap_or(0);
		major > 1 || (major == 1 && minor >= 95)
	})
}

/// Build the PVM crate in `crate_dir` for the RISCV target.
///
/// Outputs
/// - depending on the `blob_type` either [JAM program blob](jam_program_blob_common::ProgramBlob)
///   or [CoreVM program blob](jam_program_blob_common::CoreVmProgramBlob) as
///   `{out_dir}/{crate_name}.jam` or `{out_dir}/{crate_name}.corevm` respectively;
/// - [PolkaVM program blob](polkavm_linker::ProgramBlob) as `{out_dir}/{crate_name}.polkavm` for
///   debugging.
/// - Blake2b hash of the PolkaVM program blob as `{out_dir}/{crate_name}.hash`.
///
/// `out_dir` is used to store any intermediate build files.
fn build_pvm_blob(
	crate_dir: &Path,
	blob_type: BlobType,
	out_dir: &Path,
	profile: ProfileType,
	inner_name: Option<String>,
	no_default_features: bool,
) -> (String, PathBuf, PathBuf) {
	let mut args = polkavm_linker::TargetJsonArgs::default();
	args.is_64_bit = true;
	// When cargo >= 1.95 (`-Z json-target-spec` is active and JSON target specs are parsed
	// strictly), the target JSON must use the new format (integer `target-pointer-width`,
	// rustc >= 1.91 per polkavm-linker's own `check_feature(91, ...)` switch). Pair the format
	// with the flag: new format on modern cargo, `Legacy` (original behaviour) otherwise.
	if cargo_supports_json_target_spec() {
		args.rustc_version = polkavm_linker::RustcVersion::Rustc_1_91;
	} else {
		args.rustc_version = polkavm_linker::RustcVersion::Legacy;
	}

	let (target_name, target_json_path) =
		("riscv64emac-unknown-none-polkavm", polkavm_linker::target_json_path(args).unwrap());

	println!("🪤 PVM module type: {blob_type}");
	println!("🎯 Target name: {target_name}");

	// No rustup detection — always use ambient cargo unconditionally. The
	// `RUSTC_BOOTSTRAP=1` + `-Z build-std` + (when cargo >= 1.95) `-Z json-target-spec`
	// combination is sufficient on stable cargo; no nightly toolchain install is required.

	let mut info = get_crate_info(crate_dir);
	let inner_name = inner_name.unwrap_or_else(|| info.name.clone());
	println!("📦 Crate name: {}", info.name);
	println!("🏷️ Inner name: {}", inner_name);
	println!("🔖 Build profile: {}", profile.as_str());

	let mut child = Command::new("cargo");

	// Runtimes previously built via `substrate-wasm-builder` are compiled with
	// `--cfg substrate_runtime` (drops sp-io's `secp256k1` C sources, which don't
	// cross-compile to the bare Riscv target). Replicate that cfg for `Runtime` blobs.
	let mut rustflags = build_encoded_rustflags(crate_dir);
	if matches!(blob_type, BlobType::Runtime) {
		rustflags.push_str("\x1f--cfg\x1fsubstrate_runtime");
	}

	child
		.current_dir(crate_dir)
		.env_clear()
		.env("PATH", std::env::var("PATH").unwrap())
		.env("CARGO_ENCODED_RUSTFLAGS", rustflags)
		.env("CARGO_TARGET_DIR", out_dir)
		// Support building on stable. (required for `-Zbuild-std`)
		.env("RUSTC_BOOTSTRAP", "1");

	// Forward `RUSTC_WRAPPER` if set in the parent env. Used by callers that need
	// cross-host-reproducible PVM blobs.
	if let Some(w) = std::env::var_os("RUSTC_WRAPPER") {
		child.env("RUSTC_WRAPPER", w);
	} else {
		// The wrapper rewrites `-Cmetadata` so rustc's `StableCrateId` is identical on Linux and
		// macOS.
		set_default_rustc_wrapper(out_dir, &mut child);
	}

	// Note: no `cargo +<toolchain>` arg — always use ambient cargo unconditionally.

	child.args(["rustc", "--lib", "--crate-type=cdylib", "-Z", "build-std=core,alloc"]);
	if no_default_features {
		child.arg("--no-default-features");
	}
	if profile.is_release_like() && !rustc_uses_new_panic_immediate_abort() {
		// Lowers every panic site to a direct `intrinsics::abort()`,
		// so panic messages and `track_caller` `&Location` data get DCE'd out of rodata.
		// On rustc >= 1.92 this is done via `-C panic=immediate-abort` in the encoded
		// rustflags instead (see `build_encoded_rustflags`).
		child.args(["-Z", "build-std-features=panic_immediate_abort"]);
	}

	// `--target` for the Riscv runtime points at a JSON target spec produced by
	// `polkavm-linker`. Rust 1.95 added `-Z json-target-spec`; later versions
	// require it to be opted into explicitly. Older versions don't know the flag,
	// so guard on the version. `RUSTC_BOOTSTRAP=1` is already set above.
	if cargo_supports_json_target_spec() {
		child.args(["-Z", "json-target-spec"]);
	}

	child.arg(profile.to_arg()).arg("--target").arg(target_json_path);

	// Use job server to not oversubscribe CPU cores when compiling multiple PVM binaries in
	// parallel.
	if let Some(client) = get_job_server_client() {
		client.configure(&mut child);
	}

	let mut child = child.spawn().expect("Failed to execute cargo process");
	let status = child.wait().expect("Failed to execute cargo process");

	if !status.success() {
		eprintln!("Failed to build RISC-V ELF due to cargo execution error");
		std::process::exit(1);
	}

	// Post processing
	println!("Converting RISC-V ELF to PVM blob...");
	let mut config = polkavm_linker::Config::default();
	config.set_strip(std::env::var("PVM_BUILDER_STRIP").map(|value| value == "1").unwrap_or(true));
	config.set_dispatch_table(blob_type.dispatch_table());

	let input_root = &out_dir.join(target_name).join(profile.as_str());
	let input_path_bin = input_root.join(&info.name);
	let input_path_cdylib = input_root.join(format!("{}.elf", info.name.replace("-", "_")));

	let input_path = if input_path_cdylib.exists() {
		if input_path_bin.exists() {
			eprintln!(
				"Both {} and {} exist; run 'cargo clean' to get rid of old artifacts!",
				input_path_cdylib.display(),
				input_path_bin.display()
			);
			std::process::exit(1);
		}
		input_path_cdylib
	} else if input_path_bin.exists() {
		input_path_bin
	} else {
		eprintln!(
			"Failed to build: neither {} nor {} exist",
			input_path_cdylib.display(),
			input_path_bin.display()
		);
		std::process::exit(1);
	};

	let orig =
		fs::read(&input_path).unwrap_or_else(|e| panic!("Failed to read {input_path:?} :{e:?}"));
	let linked = polkavm_linker::program_from_elf(
		config,
		polkavm_linker::TargetInstructionSet::JamV1,
		orig.as_ref(),
	)
	.expect("Failed to link pvm program:");

	// Write out a full `.polkavm` blob for debugging/inspection.
	let output_path_pvm = out_dir.join(format!("{}.polkavm", info.name));
	let hash_output_file = out_dir.join(format!("{}.hash", info.name));
	fs::write(&output_path_pvm, &linked).expect("Error writing resulting binary");
	let name = info.name.clone();
	info.name = inner_name;
	let metadata = ConventionalMetadata::Info(info).encode().into();
	let output_file = blob_type.output_file(out_dir, &name);
	let blob = if !matches!(blob_type, BlobType::CoreVmGuest) {
		let parts = polkavm_linker::ProgramParts::from_bytes(linked.into())
			.expect("failed to deserialize linked PolkaVM program");
		let blob = ProgramBlob::from_pvm(&parts, metadata)
			.to_vec()
			.expect("error serializing the .jam blob");
		fs::write(&output_file, &blob).expect("error writing the .jam blob");
		blob
	} else {
		let blob = CoreVmProgramBlob { metadata, pvm_blob: linked.into() }
			.to_vec()
			.expect("error serializing the CoreVM blob");
		fs::write(&output_file, &blob).expect("error writing the CoreVM blob");
		blob
	};
	let hash = code_hash(&blob);
	fs::write(&hash_output_file, hash).expect("error writing blob hash");

	(name, output_file, hash_output_file)
}

/// Returns the hash of the code blob.
///
/// Should produce the same value as `jam_pvm_builder::pvm_binary_hash!` macro.
pub fn code_hash(data: &[u8]) -> [u8; 32] {
	let h = blake2b_simd::Params::new().hash_length(32).hash(data);
	h.as_bytes().try_into().expect("Hash length set to 32")
}

fn get_job_server_client() -> Option<&'static jobserver::Client> {
	static CLIENT: OnceLock<Option<jobserver::Client>> = OnceLock::new();
	CLIENT.get_or_init(|| unsafe { jobserver::Client::from_env() }).as_ref()
}

#[cfg(unix)]
fn set_default_rustc_wrapper(out_dir: &Path, command: &mut Command) {
	use std::os::unix::fs::PermissionsExt;
	let rustc_wrapper_path = out_dir.join("rustc-wrapper.sh");
	fs::write(&rustc_wrapper_path, include_bytes!("../scripts/rustc-wrapper.sh")).unwrap();
	let mut perms = fs::metadata(&rustc_wrapper_path).unwrap().permissions();
	perms.set_mode(0o755);
	fs::set_permissions(&rustc_wrapper_path, perms).unwrap();
	command.env("RUSTC_WRAPPER", rustc_wrapper_path);
}

#[cfg(not(unix))]
fn set_default_rustc_wrapper(_out_dir: &Path, _command: &mut Command) {
	// We need some portable solution...
}
