// Copyright (C) Parity Technologies (UK) Ltd.
// This file is part of Cumulus.
// SPDX-License-Identifier: Apache-2.0

// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
// 	http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

//! The JAM refine host-call API, re-exported from `jam_pvm_common::refine`.
//!
//! This is the fetch surface the Parachain Service's own refine entry point
//! (`parachain-service`'s `service/src/refine.rs`) drives from, so a PVF guest
//! consuming these re-exports reads the work package, its refine context and the
//! work-item payloads through the very same fetch wrappers as the node — a
//! shared, single definition of the ABI for both sides instead of per-runtime
//! copies of the raw `fetch` import. `refine_context`, for example, backs on
//! `Fetch::RefineContext` and yields `lookup_anchor_slot` as the trusted relay
//! block number.

pub use jam_pvm_common::refine::*;
