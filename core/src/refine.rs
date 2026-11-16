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
//! PVFs use the same wrappers to read work-package inputs and manage nested VMs.
//! Nested module and instance handles are private to the PVF; `invoke` spends its
//! remaining gas and refunds unused gas. A PVF handles its nested VM's host calls.

pub use jam_pvm_common::refine::*;
