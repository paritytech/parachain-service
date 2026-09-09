//! Why [`crate::ParachainServiceSpec::build`] failed.

/// Why a parachain-service chain spec could not be built.
#[derive(Debug, thiserror::Error)]
pub enum Error {
	/// The para's head data exceeds the 4 KiB `HeadData` bound (spec §3.1).
	#[error("head data of para {para} is {len} bytes, over the 4 KiB bound")]
	HeadDataTooLarge { para: u32, len: usize },
	/// Two parachain specs with the same id: their `ParaInfo` rows would collapse
	/// into one storage entry while the preimage registry kept both referencers.
	#[error("parachain spec added twice for id {0}")]
	DuplicateParaId(u32),
}
