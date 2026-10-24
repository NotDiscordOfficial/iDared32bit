/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/.
 */
//! Bindings for touchHLE's CPU core, an ARM interpreter derived from mGBA's
//! (see `vendor/mgba_arm/README.md`). Separated into its own package for
//! build-time parallelism, and to avoid rebuilding the interpreter more often
//! than necessary.

// Allow the crate to have a non-snake-case name (touchHLE).
#![allow(non_snake_case)]

/// Opaque type from C
#[allow(non_camel_case_types)]
pub type touchHLE_MgbaWrapper = std::ffi::c_void;
/// Opaque type from Rust (this is the `Mem` type from the main crate, but
/// `c_void` is used here to avoid depending on it directly)
#[allow(non_camel_case_types)]
pub type touchHLE_Mem = std::ffi::c_void;

/// Guest-visible CPU state, exchanged wholesale when touchHLE switches guest
/// threads. `extregs` holds the VFP registers D0 to D31.
#[repr(C)]
#[allow(non_camel_case_types)]
#[derive(Debug)]
pub struct touchHLE_MgbaContext {
    pub regs: [u32; 16],
    pub extregs: [u32; 64],
    pub cpsr: u32,
    pub fpscr: u32,
}

/// One bounded entry in the recent guest instruction trace.
#[repr(C)]
#[allow(non_camel_case_types)]
#[derive(Clone, Copy, Debug, Default)]
pub struct touchHLE_MgbaInstructionTrace {
    pub pc: u32,
    pub regs: [u32; 16],
}

/// The FPSCR a thread starts with: flush-to-zero on, as iOS sets it (Apple's
/// TN2293). Mirrors `TOUCHHLE_DEFAULT_FPSCR` in `wrapper.h`.
pub const DEFAULT_FPSCR: u32 = 0x0100_0000;

impl Default for touchHLE_MgbaContext {
    fn default() -> Self {
        Self {
            regs: [0; 16],
            extregs: [0; 64],
            cpsr: 0,
            fpscr: DEFAULT_FPSCR,
        }
    }
}

impl touchHLE_MgbaContext {
    pub fn new() -> Self {
        Self::default()
    }
}

type VAddr = u32;

// Import functions from lib.c, see build.rs. Note that lib.c depends on the
// touchHLE_cpu_read/write_* functions being exported from Rust, but those are
// in the main crate.
extern "C" {
    pub fn touchHLE_MgbaWrapper_new(
        direct_memory_access_ptr: *mut std::ffi::c_void,
        null_page_count: usize,
    ) -> *mut touchHLE_MgbaWrapper;
    pub fn touchHLE_MgbaWrapper_delete(cpu: *mut touchHLE_MgbaWrapper);
    pub fn touchHLE_MgbaWrapper_regs_const(cpu: *const touchHLE_MgbaWrapper) -> *const u32;
    pub fn touchHLE_MgbaWrapper_regs_mut(cpu: *mut touchHLE_MgbaWrapper) -> *mut u32;
    pub fn touchHLE_MgbaWrapper_cpsr(cpu: *const touchHLE_MgbaWrapper) -> u32;
    pub fn touchHLE_MgbaWrapper_set_cpsr(cpu: *mut touchHLE_MgbaWrapper, cpsr: u32);
    /// The encoding that last came back undefined. Only meaningful right
    /// after `run_or_step` returned -3.
    pub fn touchHLE_MgbaWrapper_last_undefined_opcode(cpu: *const touchHLE_MgbaWrapper) -> u32;
    /// The last CP15 access the core does not model, or zero, clearing it.
    pub fn touchHLE_MgbaWrapper_take_unknown_cp15(cpu: *mut touchHLE_MgbaWrapper) -> u32;
    /// Points instruction fetch at the guest's memory. Must be called with a
    /// valid mapping before the first `run_or_step`; see the comment in
    /// `lib.c`.
    pub fn touchHLE_MgbaWrapper_set_fetch_region(
        cpu: *mut touchHLE_MgbaWrapper,
        guest_memory_base: *mut std::ffi::c_void,
    );
    /// Exchanges the core's state with `context`, leaving the old state
    /// there.
    pub fn touchHLE_MgbaWrapper_swap_context(
        cpu: *mut touchHLE_MgbaWrapper,
        context: *mut touchHLE_MgbaContext,
    );
    pub fn touchHLE_MgbaWrapper_copy_recent_instruction_pcs(
        cpu: *const touchHLE_MgbaWrapper,
        output: *mut u32,
        capacity: usize,
    ) -> usize;
    pub fn touchHLE_MgbaWrapper_copy_recent_instruction_trace(
        cpu: *const touchHLE_MgbaWrapper,
        output: *mut touchHLE_MgbaInstructionTrace,
        capacity: usize,
    ) -> usize;
    pub fn touchHLE_MgbaWrapper_invalidate_cache_range(
        cpu: *mut touchHLE_MgbaWrapper,
        start: VAddr,
        size: u32,
    );
    /// Returns the SVC number, or one of the negative values mirrored in
    /// `lib.c`: -1 ran out of ticks, -2 memory error, -3 undefined
    /// instruction, -4 breakpoint.
    pub fn touchHLE_MgbaWrapper_run_or_step(
        cpu: *mut touchHLE_MgbaWrapper,
        mem: *mut touchHLE_Mem,
        ticks: Option<&mut u64>,
    ) -> i32;
}
