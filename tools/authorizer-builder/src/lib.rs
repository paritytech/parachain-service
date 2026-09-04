//! Compiles an authorizer guest crate into a PVM blob under the `production-authorizer` profile.
//!
//! `jam_pvm_builder::build_authorizer` would do exactly this, except that it hardcodes the
//! `production` profile — and an authorizer built that way is ~90 kB, well past JAM's 64 kB
//! `C_maxauthcodesize`. The same guest under `production-authorizer` (`opt-level = "z"`) fits with
//! room to spare. So this is that function's build-script wrapper with the profile as its one
//! difference; services keep using `jam_pvm_builder::build_service`, whose gas budget wants
//! `production`.

use jam_pvm_builder::{build_pvm_blob, BlobType, ProfileType};
use std::{
	fs,
	path::{Path, PathBuf},
};

/// Build the authorizer guest crate at `crate_dir`, whose package is named `crate_name`.
///
/// Exports the blob's path as `PVM_BINARY_<crate_name>` and its code hash's as
/// `PVM_BINARY_HASH_<crate_name>`, which is what `jam_pvm_builder::pvm_binary!` and
/// `pvm_binary_hash!` read. Set `SKIP_PVM_BUILDS=1` to write empty stand-ins instead, for a fast
/// `cargo check`.
pub fn build_authorizer(crate_dir: &Path, crate_name: &str) {
	let out_dir = PathBuf::from(std::env::var("OUT_DIR").expect("cargo sets OUT_DIR; qed"));
	println!("cargo:rerun-if-env-changed=SKIP_PVM_BUILDS");

	let (blob, hash) = if std::env::var_os("SKIP_PVM_BUILDS").is_some() {
		let blob = out_dir.join(format!("{crate_name}.jam"));
		let hash = out_dir.join(format!("{crate_name}.hash"));
		fs::write(&blob, []).expect("error creating dummy program blob");
		fs::write(&hash, [0u8; 32]).expect("error creating dummy program blob hash");
		(blob, hash)
	} else {
		println!("cargo:rerun-if-changed={}", crate_dir.display());
		let (built, blob, hash) = build_pvm_blob(
			crate_dir,
			BlobType::Authorizer,
			&out_dir,
			false,
			ProfileType::Other("production-authorizer"),
		);
		assert_eq!(built, crate_name, "the guest crate at {crate_dir:?} is not {crate_name}");
		(blob, hash)
	};

	println!("cargo:rustc-env=PVM_BINARY_{crate_name}={}", blob.display());
	println!("cargo:rustc-env=PVM_BINARY_HASH_{crate_name}={}", hash.display());
}
