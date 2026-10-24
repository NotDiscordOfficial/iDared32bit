/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/.
 */
//! CPU emulation.
//!
//! Implemented using mGBA's ARM interpreter, extended to ARMv7-A as the
//! Cortex-A8 has it: the integer instruction set, Thumb-2 included, VFPv3 and
//! Advanced SIMD (NEON); and to armv7s, the iPhone 5's, with VFPv4's fused
//! multiply-add, half-precision conversions and integer divide (see
//! `vendor/mgba_arm/README.md` and `src/cpu/mgba_wrapper/`). It is a plain
//! interpreter, with no decoded-instruction cache. Like the rest of touchHLE,
//! it is under the Mozilla Public License 2.0.
//!
//! iPhone OS apps used ARMv6 or ARMv7-A; see [crate::mach_o] for fat-binary
//! slice selection.

use crate::abi::GuestFunction;
use crate::mem::{ConstPtr, GuestUSize, Mem, MutPtr, Ptr, SafeRead, SafeWrite};

// Import functions from C
use touchHLE_mgba_wrapper::*;

type VAddr = u32;
pub type CpuContext = touchHLE_MgbaContext;

fn touchHLE_cpu_read_impl<T: SafeRead + Default>(
    mem: *mut touchHLE_Mem,
    addr: VAddr,
    error: *mut bool,
) -> T {
    // If a panic occurs (probably due to a null-pointer access), we can't let
    // it keep unwinding as it will hit non-Rust stack frames (the
    // interpreter). Instead we catch the unwind and then tell the C code a
    // problem occurred, so that it halts CPU execution after the current
    // instruction (see lib.c in the wrapper) and reports a memory error, and
    // touchHLE can then panic with only Rust stack frames to worry about and
    // with CPU state information available that's useful for debugging.
    //
    // I'm not sure if this actually is unwind-safe, but considering
    // the emulator will crash anyway, maybe this is okay.
    let res = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let mem = unsafe { &mut *mem.cast::<Mem>() };
        let ptr: ConstPtr<T> = Ptr::from_bits(addr);
        mem.read(ptr)
    }));
    unsafe {
        error.write(res.is_err());
    }
    res.unwrap_or_default()
}

fn touchHLE_cpu_write_impl<T: SafeWrite>(mem: *mut touchHLE_Mem, addr: VAddr, value: T) -> bool {
    // See comments above about catch_unwind
    let res = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let mem = unsafe { &mut *mem.cast::<Mem>() };
        let ptr: MutPtr<T> = Ptr::from_bits(addr);
        mem.write(ptr, value)
    }));
    res.is_err()
}

// Export functions for use by C++
#[no_mangle]
extern "C" fn touchHLE_cpu_read_u8(mem: *mut touchHLE_Mem, addr: VAddr, error: *mut bool) -> u8 {
    touchHLE_cpu_read_impl(mem, addr, error)
}
#[no_mangle]
extern "C" fn touchHLE_cpu_read_u16(mem: *mut touchHLE_Mem, addr: VAddr, error: *mut bool) -> u16 {
    touchHLE_cpu_read_impl(mem, addr, error)
}
#[no_mangle]
extern "C" fn touchHLE_cpu_read_u32(mem: *mut touchHLE_Mem, addr: VAddr, error: *mut bool) -> u32 {
    touchHLE_cpu_read_impl(mem, addr, error)
}
#[no_mangle]
extern "C" fn touchHLE_cpu_read_u64(mem: *mut touchHLE_Mem, addr: VAddr, error: *mut bool) -> u64 {
    touchHLE_cpu_read_impl(mem, addr, error)
}
#[no_mangle]
extern "C" fn touchHLE_cpu_write_u8(mem: *mut touchHLE_Mem, addr: VAddr, value: u8) -> bool {
    touchHLE_cpu_write_impl(mem, addr, value)
}
#[no_mangle]
extern "C" fn touchHLE_cpu_write_u16(mem: *mut touchHLE_Mem, addr: VAddr, value: u16) -> bool {
    touchHLE_cpu_write_impl(mem, addr, value)
}
#[no_mangle]
extern "C" fn touchHLE_cpu_write_u32(mem: *mut touchHLE_Mem, addr: VAddr, value: u32) -> bool {
    touchHLE_cpu_write_impl(mem, addr, value)
}
#[no_mangle]
extern "C" fn touchHLE_cpu_write_u64(mem: *mut touchHLE_Mem, addr: VAddr, value: u64) -> bool {
    touchHLE_cpu_write_impl(mem, addr, value)
}

pub struct Cpu {
    mgba_wrapper: *mut touchHLE_MgbaWrapper,
    /// Copy of the direct memory access pointer used to check it has not
    /// changed. If this is null, direct memory access is not in use.
    direct_memory_access_ptr: *const std::ffi::c_void,
}

impl Drop for Cpu {
    fn drop(&mut self) {
        unsafe { touchHLE_MgbaWrapper_delete(self.mgba_wrapper) }
    }
}

/// Why CPU execution ended.
#[derive(Debug)]
pub enum CpuState {
    /// Execution halted due to using up all remaining ticks (normal execution)
    /// or after the single instruction was executed (step execution).
    Normal,
    /// SVC instruction encountered.
    Svc(u32),
    /// An error was encountered.
    Error(CpuError),
}

/// A reason that can cause CPU execution to be interrupted.
#[derive(Debug, Clone, PartialEq)]
pub enum CpuError {
    /// Memory error during execution (probably a null page access).
    MemoryError,
    /// Undefined instruction (perhaps from a GDB software breakpoint).
    UndefinedInstruction,
    /// Breakpoint (`bkpt` instruction).
    Breakpoint,
}

impl Cpu {
    /// The register number of the stack pointer.
    pub const SP: usize = 13;
    /// The register number of the link register.
    #[allow(unused)]
    pub const LR: usize = 14;
    /// The register number of the program counter.
    pub const PC: usize = 15;

    /// When this bit is set in CPSR, the CPU is in Thumb mode.
    pub const CPSR_THUMB: u32 = 0x00000020;

    /// When this bit is set in CPSR, the CPU is in user mode.
    pub const CPSR_USER_MODE: u32 = 0x00000010;

    /// Construct a new CPU instance. If a mutable reference to a [Mem] instance
    /// is provided, direct memory access is enabled, and the CPU instance
    /// becomes bound to that [Mem] instance (subsequent calls must use the same
    /// one).
    pub fn new(direct_memory_access: Option<&mut Mem>) -> Cpu {
        // Null page count is in pages rather than bytes. Mem ensures it is
        // page aligned.
        let null_page_count: usize = direct_memory_access
            .as_ref()
            .map_or(0, |mem| mem.null_segment_size() / 0x1000)
            .try_into()
            .unwrap();
        // The CPU core fetches instructions straight through the guest
        // mapping (see set_fetch_region in the wrapper, and run_or_step
        // below). With direct memory access, data accesses outside the null
        // page do too, and only the rest go through the
        // touchHLE_cpu_read/write_* callbacks (see direct_data in the
        // wrapper); the null page must therefore already be set up. This
        // pointer is checked for consistency in run_or_step.
        let direct_memory_access_ptr = direct_memory_access
            .map_or(std::ptr::null_mut(), |mem| unsafe {
                mem.direct_memory_access_ptr()
            });
        let mgba_wrapper =
            unsafe { touchHLE_MgbaWrapper_new(direct_memory_access_ptr, null_page_count) };
        Cpu {
            mgba_wrapper,
            direct_memory_access_ptr,
        }
    }

    pub fn regs(&self) -> &[u32; 16] {
        unsafe {
            let ptr = touchHLE_MgbaWrapper_regs_const(self.mgba_wrapper);
            &*(ptr as *const [u32; 16])
        }
    }
    pub fn regs_mut(&mut self) -> &mut [u32; 16] {
        unsafe {
            let ptr = touchHLE_MgbaWrapper_regs_mut(self.mgba_wrapper);
            &mut *(ptr as *mut [u32; 16])
        }
    }

    /// Dump the registers of the current cpu to the log output.
    /// Silently ignores panics.
    pub fn dump_regs(&self) {
        let regs = self.regs();
        Self::echo_regs(regs);

        let mut recent_pcs = [0; 512];
        let count = self.copy_recent_instruction_pcs(&mut recent_pcs);
        if count > 0 {
            echo!("Recent guest instruction PCs (oldest first):");
            for row in recent_pcs[..count].chunks(8) {
                let line = row
                    .iter()
                    .map(|pc| format!("{pc:#010x}"))
                    .collect::<Vec<_>>()
                    .join(" ");
                echo!("  {line}");
            }

            let mut trace = [touchHLE_MgbaInstructionTrace::default(); 512];
            let trace_count = self.copy_recent_instruction_trace(&mut trace);
            echo!("Recent guest register snapshots (last 256 instructions):");
            for entry in trace[trace_count.saturating_sub(256)..trace_count].iter() {
                let regs = &entry.regs;
                let instruction = if entry.pc & 1 == 0 {
                    Self::disassemble_arm(entry.opcode)
                        .unwrap_or_else(|| "unknown ARM instruction".into())
                } else {
                    Self::disassemble_thumb(entry.opcode as u16)
                };
                echo!(
                    "  PC {:#010x}: {} | r0={:#010x} r1={:#010x} r2={:#010x} r3={:#010x} r4={:#010x} r5={:#010x} r6={:#010x} r7={:#010x} r8={:#010x} r9={:#010x} r10={:#010x} r11={:#010x} r12={:#010x} sp={:#010x} lr={:#010x}",
                    entry.pc,
                    instruction,
                    regs[0],
                    regs[1],
                    regs[2],
                    regs[3],
                    regs[4],
                    regs[5],
                    regs[6],
                    regs[7],
                    regs[8],
                    regs[9],
                    regs[10],
                    regs[11],
                    regs[12],
                    regs[13],
                    regs[14],
                );
            }

            let mut calls = [touchHLE_MgbaCallTrace::default(); 128];
            let call_count = self.copy_recent_call_trace(&mut calls);
            if call_count > 0 {
                echo!("Recent guest call-site register snapshots (last 48 calls):");
                for call in calls[call_count.saturating_sub(48)..call_count].iter() {
                    let regs = &call.regs;
                    echo!(
                        "  PC {:#010x}: r0={:#010x} r1={:#010x} r2={:#010x} r3={:#010x} r12={:#010x} sp={:#010x} lr={:#010x}",
                        call.pc,
                        regs[0],
                        regs[1],
                        regs[2],
                        regs[3],
                        regs[12],
                        regs[13],
                        regs[14],
                    );
                }
            }
        }
    }

    fn copy_recent_instruction_pcs(&self, output: &mut [u32]) -> usize {
        unsafe {
            touchHLE_MgbaWrapper_copy_recent_instruction_pcs(
                self.mgba_wrapper,
                output.as_mut_ptr(),
                output.len(),
            )
        }
    }

    fn copy_recent_instruction_trace(&self, output: &mut [touchHLE_MgbaInstructionTrace]) -> usize {
        unsafe {
            touchHLE_MgbaWrapper_copy_recent_instruction_trace(
                self.mgba_wrapper,
                output.as_mut_ptr(),
                output.len(),
            )
        }
    }

    fn copy_recent_call_trace(&self, output: &mut [touchHLE_MgbaCallTrace]) -> usize {
        unsafe {
            touchHLE_MgbaWrapper_copy_recent_call_trace(
                self.mgba_wrapper,
                output.as_mut_ptr(),
                output.len(),
            )
        }
    }

    fn disassemble_arm(opcode: u32) -> Option<String> {
        let cond = opcode >> 28;
        let cond_suffix = match cond {
            0x0 => "eq",
            0x1 => "ne",
            0x2 => "cs",
            0x3 => "cc",
            0x4 => "mi",
            0x5 => "pl",
            0x6 => "vs",
            0x7 => "vc",
            0x8 => "hi",
            0x9 => "ls",
            0xa => "ge",
            0xb => "lt",
            0xc => "gt",
            0xd => "le",
            0xe => "",
            _ => return None,
        };
        if opcode & 0x0c00_0000 == 0x0400_0000 && opcode & (1 << 25) == 0 {
            let load = opcode & (1 << 20) != 0;
            let byte = opcode & (1 << 22) != 0;
            let pre_index = opcode & (1 << 24) != 0;
            let add = opcode & (1 << 23) != 0;
            let writeback = opcode & (1 << 21) != 0;
            let rn = (opcode >> 16) & 0xf;
            let rd = (opcode >> 12) & 0xf;
            let offset = opcode & 0xfff;
            let sign = if add { "" } else { "-" };
            let suffix = if byte { "b" } else { "" };
            let mnemonic = format!(
                "{}{}{}",
                if load { "ldr" } else { "str" },
                suffix,
                cond_suffix
            );
            let operand = if offset == 0 {
                format!("r{rd}, [r{rn}]")
            } else {
                format!("r{rd}, [r{rn}, #{sign}{offset}]")
            };
            return Some(if pre_index {
                format!("{mnemonic} {operand}{}", if writeback { "!" } else { "" })
            } else {
                format!("{mnemonic} r{rd}, [r{rn}], #{sign}{offset}")
            });
        }
        None
    }

    fn disassemble_thumb(opcode: u16) -> String {
        let op = opcode as u32;
        if op & 0xf800 == 0x6800 || op & 0xf800 == 0x6000 {
            let load = op & 0x0800 != 0;
            let imm5 = (op >> 6) & 0x1f;
            let rn = (op >> 3) & 7;
            let rd = op & 7;
            let offset = imm5 * 4;
            return format!(
                "{} r{rd}, [r{rn}, #{offset}]",
                if load { "ldr" } else { "str" }
            );
        }
        "unknown Thumb instruction".into()
    }

    pub fn echo_regs(regs: &[u32; 16]) {
        // Silently ignore panics so it's safe to use in contexts where we
        // can't panic.
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            for row in 0..4 {
                use std::fmt::Write;
                let mut line = String::new();
                for col in 0..4 {
                    let reg_idx = row * 4 + col;
                    match reg_idx {
                        Self::SP => write!(&mut line, "\t SP: "),
                        Self::LR => write!(&mut line, "\t LR: "),
                        Self::PC => write!(&mut line, "\t PC: "),
                        _ if reg_idx <= 9 => write!(&mut line, "\t R{reg_idx}: "),
                        _ => write!(&mut line, "\tR{reg_idx}: "),
                    }
                    .unwrap();
                    write!(&mut line, "{:#010x}", regs[reg_idx]).unwrap();
                }
                echo!("{}", line);
            }
        }));
    }

    pub fn cpsr(&self) -> u32 {
        unsafe { touchHLE_MgbaWrapper_cpsr(self.mgba_wrapper) }
    }

    pub fn set_cpsr(&mut self, cpsr: u32) {
        unsafe { touchHLE_MgbaWrapper_set_cpsr(self.mgba_wrapper, cpsr) }
    }

    /// Swap the current state of the CPU (registers etc) with the state stored
    /// in the context object.
    pub fn swap_context(&mut self, context: &mut CpuContext) {
        unsafe { touchHLE_MgbaWrapper_swap_context(self.mgba_wrapper, context) }
    }

    /// Get PC with the Thumb bit appropriately set.
    pub fn pc_with_thumb_bit(&self) -> GuestFunction {
        let pc = self.regs()[Self::PC];
        let thumb = (self.cpsr() & Self::CPSR_THUMB) == Self::CPSR_THUMB;
        GuestFunction::from_addr_and_thumb_flag(pc, thumb)
    }

    /// Set PC and the Thumb flag for executing a guest function. Note that this
    /// does not touch LR.
    pub fn branch(&mut self, new_pc: GuestFunction) {
        self.regs_mut()[Self::PC] = new_pc.addr_without_thumb_bit();
        let cpsr_without_thumb = self.cpsr() & (!Self::CPSR_THUMB);
        self.set_cpsr(cpsr_without_thumb | ((new_pc.is_thumb() as u32) * Self::CPSR_THUMB))
    }

    /// Set the PC and Thumb flag (like [Self::branch]), but also set the LR,
    /// and return the original PC and LR.
    pub fn branch_with_link(
        &mut self,
        new_pc: GuestFunction,
        new_lr: GuestFunction,
    ) -> (GuestFunction, GuestFunction) {
        let old_pc = self.pc_with_thumb_bit();
        let old_lr = GuestFunction::from_addr_with_thumb_bit(self.regs()[Self::LR]);
        self.branch(new_pc);
        self.regs_mut()[Self::LR] = new_lr.addr_with_thumb_bit();
        (old_pc, old_lr)
    }

    /// Clear the interpreter's decoded-instruction cache for some range of
    /// addresses. This is of interest to the dynamic linker, which will
    /// sometimes rewrite code.
    pub fn invalidate_cache_range(&mut self, base: VAddr, size: GuestUSize) {
        unsafe { touchHLE_MgbaWrapper_invalidate_cache_range(self.mgba_wrapper, base, size) }
    }

    /// Start CPU execution.
    ///
    /// If `ticks` is [Some], it is used as an abstract time limit. The value
    /// will be reduced proportionately with the amount of ticks expended.
    ///
    /// If `ticks` is [None], the CPU executes only a single instruction. This
    /// is also known as "stepping".
    ///
    /// This will return either because the CPU ran out of time, or because
    /// something else happened which requires attention from the host.
    #[must_use]
    pub fn run_or_step(&mut self, mem: &mut Mem, ticks: Option<&mut u64>) -> CpuState {
        // See ::new() for why this is done.
        if !self.direct_memory_access_ptr.is_null() {
            assert!(self.direct_memory_access_ptr == unsafe { mem.direct_memory_access_ptr() });
        }

        // mGBA fetches instructions straight through the guest mapping, with
        // no callback path, so it needs this whether or not direct memory
        // access is enabled for data. Setting it per-run also means a Cpu
        // built without a Mem (see ::new) still works.
        unsafe {
            touchHLE_MgbaWrapper_set_fetch_region(self.mgba_wrapper, mem.direct_memory_access_ptr())
        };

        let res = unsafe {
            touchHLE_MgbaWrapper_run_or_step(
                self.mgba_wrapper,
                mem as *mut Mem as *mut touchHLE_Mem,
                ticks,
            )
        };
        // CP15 accesses the core doesn't model succeed rather than failing,
        // but are logged so a missing one can be found; see cp15.c in the
        // wrapper.
        let unknown_cp15 = unsafe { touchHLE_MgbaWrapper_take_unknown_cp15(self.mgba_wrapper) };
        if unknown_cp15 != 0 {
            log!(
                "Unimplemented CP15 access {:#010x} (CRn={}, opc1={}, CRm={}, opc2={}), {}",
                unknown_cp15,
                (unknown_cp15 >> 16) & 0xF,
                (unknown_cp15 >> 21) & 7,
                unknown_cp15 & 0xF,
                (unknown_cp15 >> 5) & 7,
                if (unknown_cp15 >> 20) & 1 != 0 {
                    "read as zero"
                } else {
                    "write ignored"
                }
            );
        }

        match res {
            -1 => CpuState::Normal,
            -2 => CpuState::Error(CpuError::MemoryError),
            -3 => {
                // Worth spelling out: an undefined encoding here usually
                // means a gap in this core rather than bad guest code.
                let opcode =
                    unsafe { touchHLE_MgbaWrapper_last_undefined_opcode(self.mgba_wrapper) };
                // PC is past the instruction at this point (see
                // Environment::debug_cpu_error), so step back to it.
                let thumb = (self.cpsr() & Self::CPSR_THUMB) != 0;
                log!(
                    "Undefined instruction {:#010x} at {:#010x} ({})",
                    opcode,
                    self.regs()[Self::PC] - if thumb { 2 } else { 4 },
                    if thumb { "Thumb" } else { "ARM" }
                );
                CpuState::Error(CpuError::UndefinedInstruction)
            }
            -4 => CpuState::Error(CpuError::Breakpoint),
            _ if res < -4 => panic!("Unexpected CPU execution result"),
            svc => CpuState::Svc(svc as u32),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Sanity check: hand-assembled ARMv6 program runs basic
    /// data-processing instructions and halts on `svc`. Doesn't need a real
    /// Mach-O binary or toolchain, unlike the integration tests.
    #[test]
    fn disassembles_arm_load_from_null_plus_eight() {
        assert_eq!(
            Cpu::disassemble_arm(0xe591_0008).as_deref(),
            Some("ldr r0, [r1, #8]")
        );
    }

    #[test]
    fn runs_basic_arm_program() {
        let mut mem = Mem::new();

        let code_addr: VAddr = 0x1000;
        let instructions: [u32; 4] = [
            0xe3a0002a, // mov r0, #42
            0xe3a0103a, // mov r1, #58
            0xe0802001, // add r2, r0, r1
            0xef123456, // svc #0x123456
        ];
        for (i, instruction) in instructions.iter().enumerate() {
            let addr: MutPtr<u32> = Ptr::from_bits(code_addr + (i as u32) * 4);
            mem.write(addr, *instruction);
        }

        let mut cpu = Cpu::new(None);
        cpu.branch(GuestFunction::from_addr_and_thumb_flag(code_addr, false));

        let mut ticks = 10u64;
        let state = cpu.run_or_step(&mut mem, Some(&mut ticks));

        assert_eq!(cpu.regs()[0], 42);
        assert_eq!(cpu.regs()[1], 58);
        assert_eq!(cpu.regs()[2], 100);
        // PC should be just after the svc instruction, matching real ARM
        // hardware SVC-exception-entry semantics (see handle_cpu_state() in
        // src/environment.rs, which subtracts 4 to recover the svc address).
        assert_eq!(cpu.regs()[Cpu::PC], code_addr + 4 * 4);
        let mut recent_pcs = [0; 4];
        let count = cpu.copy_recent_instruction_pcs(&mut recent_pcs);
        assert_eq!(count, 4);
        assert_eq!(
            recent_pcs,
            [code_addr, code_addr + 4, code_addr + 8, code_addr + 12]
        );

        let mut recent_trace = [touchHLE_MgbaInstructionTrace::default(); 4];
        let count = cpu.copy_recent_instruction_trace(&mut recent_trace);
        assert_eq!(count, 4);
        assert_eq!(recent_trace[0].pc, code_addr);
        assert_eq!(recent_trace[0].regs[0], 0);
        assert_eq!(recent_trace[1].pc, code_addr + 4);
        assert_eq!(recent_trace[1].regs[0], 42);
        assert_eq!(recent_trace[2].regs[1], 58);
        assert_eq!(recent_trace[3].regs[2], 100);
        match state {
            CpuState::Svc(svc) => assert_eq!(svc, 0x123456),
            other => panic!("Expected an SVC halt, got {other:?}"),
        }
    }

    #[test]
    fn records_arm_call_site_registers() {
        let mut mem = Mem::new();
        let code_addr: VAddr = 0x1000;
        let instructions = [
            0xe3a0_0007, // mov r0, #7
            0xeb00_0001, // bl 0x1010
            0xef12_3456, // svc #0x123456
            0xe1a0_0000, // nop
            0xe12f_ff1e, // bx lr
        ];
        for (i, instruction) in instructions.iter().enumerate() {
            let addr: MutPtr<u32> = Ptr::from_bits(code_addr + (i as u32) * 4);
            mem.write(addr, *instruction);
        }

        let mut cpu = Cpu::new(None);
        cpu.branch(GuestFunction::from_addr_and_thumb_flag(code_addr, false));
        let mut ticks = 10u64;
        cpu.run_or_step(&mut mem, Some(&mut ticks));

        let mut calls = [touchHLE_MgbaCallTrace::default(); 4];
        assert_eq!(cpu.copy_recent_call_trace(&mut calls), 1);
        assert_eq!(calls[0].pc, code_addr + 4);
        assert_eq!(calls[0].regs[0], 7);
        // The snapshot is taken before BL executes, so LR still holds its
        // incoming value.
        assert_eq!(calls[0].regs[14], 0);
    }

    /// An `svc` must halt before the core looks at whatever follows it, even
    /// in batched (ticks > 1) mode: here that is an undecodable word, so a
    /// core that read ahead would report UndefinedInstruction instead of Svc.
    #[test]
    fn svc_stops_block_translation_before_undecodable_data() {
        let mut mem = Mem::new();

        let code_addr: VAddr = 0x1000;
        let instructions: [u32; 3] = [
            0xe3a0002a, // mov r0, #42
            0xef123456, // svc #0x123456
            0x79706f63, // not a valid ARM instruction ("copy" as ASCII bytes)
        ];
        for (i, instruction) in instructions.iter().enumerate() {
            let addr: MutPtr<u32> = Ptr::from_bits(code_addr + (i as u32) * 4);
            mem.write(addr, *instruction);
        }

        let mut cpu = Cpu::new(None);
        cpu.branch(GuestFunction::from_addr_and_thumb_flag(code_addr, false));

        let mut ticks = 10u64;
        let state = cpu.run_or_step(&mut mem, Some(&mut ticks));

        assert_eq!(cpu.regs()[0], 42);
        match state {
            CpuState::Svc(svc) => assert_eq!(svc, 0x123456),
            other => panic!("Expected an SVC halt, got {other:?}"),
        }
    }

    /// A `bkpt` instruction should halt execution (as CpuError::Breakpoint)
    /// rather than being treated as a no-op -- touchHLE's GDB support depends
    /// on `bkpt` actually trapping.
    #[test]
    fn bkpt_halts_execution() {
        let mut mem = Mem::new();

        let code_addr: VAddr = 0x1000;
        let addr: MutPtr<u32> = Ptr::from_bits(code_addr);
        mem.write(addr, 0xe1200070u32); // bkpt #0

        let mut cpu = Cpu::new(None);
        cpu.branch(GuestFunction::from_addr_and_thumb_flag(code_addr, false));

        let mut ticks = 10u64;
        let state = cpu.run_or_step(&mut mem, Some(&mut ticks));

        // PC should still point at the bkpt itself, not past it.
        assert_eq!(cpu.regs()[Cpu::PC], code_addr);
        assert!(matches!(state, CpuState::Error(CpuError::Breakpoint)));
    }

    /// An undefined instruction encoding should halt as
    /// CpuError::UndefinedInstruction rather than aborting the process.
    #[test]
    fn undefined_instruction_halts_execution() {
        let mut mem = Mem::new();

        let code_addr: VAddr = 0x1000;
        let addr: MutPtr<u32> = Ptr::from_bits(code_addr);
        // The same permanently-undefined encoding touchHLE itself uses
        // elsewhere as a deliberate trap instruction (see encode_a32_trap()
        // in src/dyld.rs).
        mem.write(addr, 0xe7ffdefeu32);

        let mut cpu = Cpu::new(None);
        cpu.branch(GuestFunction::from_addr_and_thumb_flag(code_addr, false));

        let mut ticks = 10u64;
        let state = cpu.run_or_step(&mut mem, Some(&mut ticks));

        assert!(matches!(
            state,
            CpuState::Error(CpuError::UndefinedInstruction)
        ));
    }

    fn write_halfwords(mem: &mut Mem, addr: VAddr, halfwords: &[u16]) {
        for (i, halfword) in halfwords.iter().enumerate() {
            let ptr: MutPtr<u16> = Ptr::from_bits(addr + (i as u32) * 2);
            mem.write(ptr, *halfword);
        }
    }

    fn write_words(mem: &mut Mem, addr: VAddr, words: &[u32]) {
        for (i, word) in words.iter().enumerate() {
            let ptr: MutPtr<u32> = Ptr::from_bits(addr + (i as u32) * 4);
            mem.write(ptr, *word);
        }
    }

    fn run_to_svc(cpu: &mut Cpu, mem: &mut Mem) -> u32 {
        let mut ticks = 100u64;
        match cpu.run_or_step(mem, Some(&mut ticks)) {
            CpuState::Svc(svc) => svc,
            other => panic!("Expected an SVC halt, got {other:?}"),
        }
    }

    /// Thumb-2: 32-bit instructions, IT blocks (including a 16-bit
    /// instruction that sets the flags outside one but not inside), TBB, BL
    /// and CBZ. Assembled with llvm-mc; the branch offsets are its.
    #[test]
    fn runs_thumb2_program() {
        let mut mem = Mem::new();
        let code_addr: VAddr = 0x1000;
        write_halfwords(
            &mut mem,
            code_addr,
            &[
                0xf245, 0x6078, // 00: movw r0, #0x5678
                0xf2c1, 0x2034, // 04: movt r0, #0x1234
                0x2100, // 08: movs r1, #0 (sets Z)
                0xbf0c, // 0a: ite eq
                0xf100, 0x0201, // 0c: addeq.w r2, r0, #1
                0x2207, // 10: movne r2, #7 (skipped)
                0xbf08, // 12: it eq
                0x1d4b, // 14: addeq r3, r1, #5 (no flags in an IT block)
                0xbf14, // 16: ite ne
                0x2409, // 18: movne r4, #9
                0x2401, // 1a: moveq r4, #1
                0xf3c0, 0x2507, // 1c: ubfx r5, r0, #8, #8
                0xa607, // 20: adr r6, table
                0x2702, // 22: movs r7, #2
                0xe8d6, 0xf007, // 24: tbb [r6, r7]
                0xe005, // 28: b fail
                0xe004, // 2a: b fail
                0xf000, 0xf805, // 2c: bl func
                0xb101, // 30: cbz r1, zero
                0xe000, // 32: b fail
                0xdf00, // 34: zero: svc #0
                0x2000, // 36: fail: movs r0, #0
                0xdf01, // 38: svc #1
                0xf04f, 0x2cab, // 3a: func: mov.w r12, #0xab00ab00
                0x4770, // 3e: bx lr
                0x0100, 0x0002, // 40: table: .byte 0, 1, 2, 0
            ],
        );

        let mut cpu = Cpu::new(None);
        cpu.branch(GuestFunction::from_addr_and_thumb_flag(code_addr, true));
        assert_eq!(run_to_svc(&mut cpu, &mut mem), 0);

        let regs = cpu.regs();
        assert_eq!(regs[0], 0x12345678);
        assert_eq!(regs[2], 0x12345679);
        assert_eq!(regs[3], 5);
        assert_eq!(regs[4], 1, "the IT block's ADD must not clear Z");
        assert_eq!(regs[5], 0x56);
        assert_eq!(regs[12], 0xab00ab00);
        assert_eq!(regs[Cpu::LR], (code_addr + 0x30) | 1);
        assert_eq!(regs[Cpu::PC], code_addr + 0x36);
        assert_eq!(cpu.cpsr() & Cpu::CPSR_THUMB, Cpu::CPSR_THUMB);
    }

    /// ARM to Thumb and back, and Thumb to ARM and back, by every ARMv5T
    /// route: BLX (register), BLX (immediate) from Thumb, and returns by
    /// POP {pc} in Thumb and a load to the PC in ARM.
    #[test]
    fn interworks_between_arm_and_thumb() {
        let mut mem = Mem::new();
        let code_addr: VAddr = 0x1000;
        write_words(
            &mut mem,
            code_addr,
            &[
                0xe28f000c, // 00: add r0, pc, #12 (thumb_func)
                0xe3800001, // 04: orr r0, r0, #1
                0xe12fff30, // 08: blx r0
                0xe3a05005, // 0c: mov r5, #5
                0xef000000, // 10: svc #0
            ],
        );
        write_halfwords(
            &mut mem,
            code_addr + 0x14,
            &[
                0xb500, // 14: thumb_func: push {lr}
                0x2101, // 16: movs r1, #1
                0xf000, 0xe802, // 18: blx arm_func
                0xbd00, // 1c: pop {pc}
                0x0000, // 1e: padding
            ],
        );
        write_words(
            &mut mem,
            code_addr + 0x20,
            &[
                0xe52de004, // 20: arm_func: push {lr}
                0xe3a02002, // 24: mov r2, #2
                0xe49df004, // 28: pop {pc}
            ],
        );

        let mut cpu = Cpu::new(None);
        cpu.branch(GuestFunction::from_addr_and_thumb_flag(code_addr, false));
        cpu.regs_mut()[Cpu::SP] = 0x3000;
        assert_eq!(run_to_svc(&mut cpu, &mut mem), 0);

        let regs = cpu.regs();
        assert_eq!(regs[1], 1);
        assert_eq!(regs[2], 2);
        assert_eq!(regs[5], 5);
        assert_eq!(regs[Cpu::SP], 0x3000);
        assert_eq!(regs[Cpu::PC], code_addr + 0x14);
        assert_eq!(cpu.cpsr() & Cpu::CPSR_THUMB, 0);
    }

    /// Flag behaviour that differs from the ARMv4T that mGBA modelled, each
    /// once wrong here: the Q flag is sticky, MULS leaves C alone, and SMLAxy
    /// wraps rather than saturating.
    #[test]
    fn keeps_armv5_flag_semantics() {
        let mut mem = Mem::new();
        let code_addr: VAddr = 0x1000;
        write_words(
            &mut mem,
            code_addr,
            &[
                0xe3e01102, // mvn r1, #0x80000000
                0xe1010051, // qadd r0, r1, r1 (saturates: sets Q)
                0xe2922001, // adds r2, r2, #1 (must not clear Q)
                0xe3a04003, // mov r4, #3
                0xe1540004, // cmp r4, r4 (sets C)
                0xe0150494, // muls r5, r4, r4 (must not change C)
                0xe3a06c7f, // mov r6, #0x7f00
                0xe38660ff, // orr r6, r6, #0xff
                0xe1071686, // smlabb r7, r6, r6, r1
                0xe10f8000, // mrs r8, apsr
                0xef000000, // svc #0
            ],
        );

        let mut cpu = Cpu::new(None);
        cpu.branch(GuestFunction::from_addr_and_thumb_flag(code_addr, false));
        assert_eq!(run_to_svc(&mut cpu, &mut mem), 0);

        let regs = cpu.regs();
        assert_eq!(regs[0], 0x7fffffff);
        assert_eq!(regs[5], 9);
        assert_eq!(regs[7], 0x3fff0001u32.wrapping_add(0x7fffffff));
        // NZCVQ: C and Q set.
        assert_eq!(regs[8] & 0xf8000000, 0x28000000);
    }

    /// VFPv3: D16 to D31, VMOV immediate, a fixed-point conversion, and moves
    /// to and from either half of a double register. D31 must also survive a
    /// thread switch, since CpuContext now carries all 32 double registers.
    #[test]
    fn runs_vfpv3() {
        let mut mem = Mem::new();
        let code_addr: VAddr = 0x1000;
        write_words(
            &mut mem,
            code_addr,
            &[
                0xeef70b08, // vmov.f64 d16, #1.5
                0xeebd0a00, // vmov.f32 s0, #-0.25
                0xeef7fac0, // vcvt.f64.f32 d31, s0
                0xee701baf, // vadd.f64 d17, d16, d31
                0xec510b31, // vmov r0, r1, d17
                0xeef02b08, // vmov.f64 d18, #3.0
                0xeefe2bcc, // vcvt.s32.f64 d18, d18, #8 (fixed point)
                0xee122b90, // vmov.32 r2, d18[0]
                0xee3f3b90, // vmov.32 r3, d31[1]
                0xef000000, // svc #0
            ],
        );

        let mut cpu = Cpu::new(None);
        cpu.branch(GuestFunction::from_addr_and_thumb_flag(code_addr, false));
        assert_eq!(run_to_svc(&mut cpu, &mut mem), 0);

        let regs = cpu.regs();
        let sum = f64::from_bits(((regs[1] as u64) << 32) | regs[0] as u64);
        assert_eq!(sum, 1.25);
        assert_eq!(regs[2], 3 << 8);
        assert_eq!(regs[3], ((-0.25f64).to_bits() >> 32) as u32);

        let mut context = CpuContext::new();
        cpu.swap_context(&mut context);
        let d31 = ((context.extregs[63] as u64) << 32) | context.extregs[62] as u64;
        assert_eq!(f64::from_bits(d31), -0.25);
    }

    /// FPSCR's controls: flush-to-zero, on by default as on iOS, flushing a
    /// tiny result and a denormal input; a directed rounding mode; and short
    /// vectors, including wrapping within a bank and a scalar Vm.
    #[test]
    fn honours_fpscr_controls() {
        let mut mem = Mem::new();
        let code_addr: VAddr = 0x1000;
        write_words(
            &mut mem,
            code_addr,
            &[
                0xeef10a10, // vmrs r0, fpscr
                0xec421a10, // vmov s0, s1, r1, r2
                0xee201a20, // vmul.f32 s2, s0, s1
                0xee013a90, // vmov s3, r3
                0xee312aa0, // vadd.f32 s4, s3, s1
                0xeef14a10, // vmrs r4, fpscr
                0xeee15a10, // vmsr fpscr, r5
                0xec476a32, // vmov s5, s6, r6, r7
                0xee723a83, // vadd.f32 s7, s5, s6
                0xec4a9a18, // vmov s16, s17, r9, r10
                0xec4cba19, // vmov s18, s19, r11, r12
                0xeee18a10, // vmsr fpscr, r8
                0xee387a08, // vadd.f32 s14, s16, s16
                0xee28ca20, // vmul.f32 s24, s16, s1
                0xee320aa2, // vadd.f32 s0, s5, s5
                0xef000000, // svc #0
            ],
        );

        let mut cpu = Cpu::new(None);
        cpu.branch(GuestFunction::from_addr_and_thumb_flag(code_addr, false));
        let inputs: [(usize, u32); 11] = [
            (1, f32::MIN_POSITIVE.to_bits()),
            (2, 0.5f32.to_bits()),
            (3, 1),           // the smallest denormal
            (5, 0x0040_0000), // round toward +infinity, FZ off
            (6, 1.0f32.to_bits()),
            (7, 2f32.powi(-30).to_bits()),
            (8, 0x0003_0000), // LEN=3 (four elements), stride 1
            (9, 1.0f32.to_bits()),
            (10, 2.0f32.to_bits()),
            (11, 3.0f32.to_bits()),
            (12, 4.0f32.to_bits()),
        ];
        for (reg, value) in inputs {
            cpu.regs_mut()[reg] = value;
        }
        assert_eq!(run_to_svc(&mut cpu, &mut mem), 0);

        let regs = cpu.regs();
        assert_eq!(regs[0], DEFAULT_FPSCR);
        // FZ, plus the input-denormal and underflow flags.
        assert_eq!(regs[4], DEFAULT_FPSCR | 0x80 | 0x08);

        let mut context = CpuContext::new();
        cpu.swap_context(&mut context);
        let s = |n: usize| f32::from_bits(context.extregs[n]);
        assert_eq!(context.extregs[2], 0, "tiny product flushed to +0");
        assert_eq!(s(4), 0.5, "denormal input flushed");
        assert_eq!(context.extregs[7], 0x3f80_0001, "rounded up");
        // A destination in the scalar bank keeps the operation scalar.
        assert_eq!(s(0), 2.0);
        assert_eq!(s(1), 0.5, "untouched by the scalar VADD");
        // s14 + s15, then wrapping to s8 and s9.
        assert_eq!([s(14), s(15), s(8), s(9)], [2.0, 4.0, 6.0, 8.0]);
        // s1 is in the scalar bank, so it multiplies every element.
        assert_eq!([s(24), s(25), s(26), s(27)], [0.5, 1.0, 1.5, 2.0]);
        assert_eq!(context.fpscr & 0x0037_0000, 0x0003_0000);
    }

    /// Advanced SIMD: loads and stores (including VLD4's de-interleaving), a
    /// floating-point multiply by scalar, a saturating add setting QC, lane
    /// moves and VDUP; then the Thumb-2 encodings, one skipped by IT.
    #[test]
    fn runs_neon() {
        let mut mem = Mem::new();
        let code_addr: VAddr = 0x1000;
        write_words(
            &mut mem,
            code_addr,
            &[
                0xf4200a8d, // vld1.32 {d0, d1}, [r0]!
                0xf2862f50, // vmov.f32 q1, #0.5
                0xf3a04942, // vmul.f32 q2, q0, d2[0]
                0xf4014a8f, // vst1.32 {d4, d5}, [r1]
                0xeec62b10, // vdup.8 d6, r2
                0xf2067016, // vqadd.s8 d7, d6, d6
                0xeef73b30, // vmov.u8 r3, d7[5]
                0xeef14a10, // vmrs r4, fpscr
                0xf465000f, // vld4.8 {d16, d17, d18, d19}, [r5]
                0xee116b90, // vmov.32 r6, d17[0]
                0xef000000, // svc #0
            ],
        );
        let floats = [1.0f32, 2.0, 3.0, 4.0].map(f32::to_bits);
        write_words(&mut mem, 0x2000, &floats);
        for i in 0..32u32 {
            let ptr: MutPtr<u8> = Ptr::from_bits(0x4000 + i);
            mem.write(ptr, i as u8);
        }

        let mut cpu = Cpu::new(None);
        cpu.branch(GuestFunction::from_addr_and_thumb_flag(code_addr, false));
        cpu.regs_mut()[0] = 0x2000;
        cpu.regs_mut()[1] = 0x3000;
        cpu.regs_mut()[2] = 0x7f;
        cpu.regs_mut()[5] = 0x4000;
        assert_eq!(run_to_svc(&mut cpu, &mut mem), 0);

        let regs = cpu.regs();
        assert_eq!(regs[0], 0x2010, "writeback");
        assert_eq!(regs[3], 0x7f, "saturated");
        assert_eq!(regs[4], DEFAULT_FPSCR | (1 << 27), "QC set");
        assert_eq!(regs[6], 0x0d090501, "de-interleaved");
        let products: Vec<f32> = (0..4)
            .map(|i| {
                let ptr: ConstPtr<u32> = Ptr::from_bits(0x3000 + i * 4);
                f32::from_bits(mem.read(ptr))
            })
            .collect();
        assert_eq!(products, [0.5, 1.0, 1.5, 2.0]);

        let code_addr: VAddr = 0x1100;
        write_halfwords(
            &mut mem,
            code_addr,
            &[
                0xff00, 0x0150, // veor q0, q0, q0
                0xef80, 0x2057, // vmov.i32 q1, #7
                0x4280, // cmp r0, r0
                0xbf18, // it ne
                0xef20, 0x0842, // vaddne.i32 q0, q0, q1 (skipped)
                0xef20, 0x0842, // vadd.i32 q0, q0, q1
                0xee31, 0x0b10, // vmov.32 r0, d1[1]
                0xdf00, // svc #0
            ],
        );
        cpu.branch(GuestFunction::from_addr_and_thumb_flag(code_addr, true));
        assert_eq!(run_to_svc(&mut cpu, &mut mem), 0);
        assert_eq!(cpu.regs()[0], 7);
    }

    /// With direct memory access, data outside the null page bypasses the
    /// callbacks, including unaligned accesses; an access starting in the
    /// null page, or running off the top of the address space, still goes to
    /// them and is reported as a memory error.
    #[test]
    fn direct_data_access() {
        let mut mem = Mem::new();
        mem.set_null_segment_size(0x1000);
        let code_addr: VAddr = 0x1000;
        write_words(
            &mut mem,
            code_addr,
            &[
                0xe5801000, // str r1, [r0]
                0xe1d020b1, // ldrh r2, [r0, #1]
                0xe5903000, // ldr r3, [r0]
                0xef000000, // svc #0
                0xe5954000, // ldr r4, [r5]
                0xef000000, // svc #0
            ],
        );

        let mut cpu = Cpu::new(Some(&mut mem));
        cpu.branch(GuestFunction::from_addr_and_thumb_flag(code_addr, false));
        cpu.regs_mut()[0] = 0x2001;
        cpu.regs_mut()[1] = 0x12345678;
        assert_eq!(run_to_svc(&mut cpu, &mut mem), 0);
        assert_eq!(cpu.regs()[2], 0x3456);
        assert_eq!(cpu.regs()[3], 0x12345678);
        let stored: ConstPtr<u32> = Ptr::from_bits(0x2001);
        assert_eq!(mem.read(stored), 0x12345678);

        for bad_addr in [0x10, 0xfffffffe] {
            cpu.branch(GuestFunction::from_addr_and_thumb_flag(
                code_addr + 16,
                false,
            ));
            cpu.regs_mut()[5] = bad_addr;
            let mut ticks = 100u64;
            assert!(matches!(
                cpu.run_or_step(&mut mem, Some(&mut ticks)),
                CpuState::Error(CpuError::MemoryError)
            ));
        }
    }

    /// UNPREDICTABLE encodings have defined behaviour (see "UNPREDICTABLE
    /// encodings" in vendor/mgba_arm/README.md): MOVW to the PC is a branch,
    /// and a set should-be-zero field makes CMP undefined.
    #[test]
    fn unpredictable_forms_are_defined() {
        let mut mem = Mem::new();
        write_words(
            &mut mem,
            0x1000,
            &[
                0xe301f100, // movw pc, #0x1100
                0xef000000, // svc #0 (skipped)
                0xe3501000, // cmp r0, #0, with Rd = r1
            ],
        );
        write_words(&mut mem, 0x1100, &[0xef000001]); // svc #1

        let mut cpu = Cpu::new(None);
        cpu.branch(GuestFunction::from_addr_and_thumb_flag(0x1000, false));
        assert_eq!(run_to_svc(&mut cpu, &mut mem), 1);

        cpu.branch(GuestFunction::from_addr_and_thumb_flag(0x1008, false));
        let mut ticks = 100u64;
        assert!(matches!(
            cpu.run_or_step(&mut mem, Some(&mut ticks)),
            CpuState::Error(CpuError::UndefinedInstruction)
        ));
    }

    /// The armv7s additions: VFPv4's fused multiply-add rounds once, where
    /// the separate multiply and add would round the product away, and the
    /// half-precision conversions round (65520 is past half precision's
    /// largest value, 65504, so it becomes infinity) and widen exactly.
    #[test]
    fn runs_vfpv4() {
        let mut mem = Mem::new();
        let code_addr: VAddr = 0x1000;
        write_words(
            &mut mem,
            code_addr,
            &[
                0xec410a10, // vmov s0, s1, r0, r1
                0xee012a10, // vmov s2, r2
                0xeea01a20, // vfma.f32 s2, s0, s1
                0xee113a10, // vmov r3, s2
                0xee014a90, // vmov s3, r4
                0xeeb32a61, // vcvtb.f16.f32 s4, s3
                0xee125a10, // vmov r5, s4
                0xee036a90, // vmov s7, r6
                0xeeb23ae3, // vcvtt.f32.f16 s6, s7
                0xee137a10, // vmov r7, s6
                0xef000000, // svc #0
            ],
        );

        let mut cpu = Cpu::new(None);
        cpu.branch(GuestFunction::from_addr_and_thumb_flag(code_addr, false));
        let inputs: [(usize, u32); 5] = [
            (0, (1.0f32 + 2f32.powi(-13)).to_bits()),
            (1, (1.0f32 - 2f32.powi(-13)).to_bits()),
            (2, (-1.0f32).to_bits()),
            (4, 65520.0f32.to_bits()),
            (6, 0x3c00_0000), // half-precision 1.0 in the top half
        ];
        for (reg, value) in inputs {
            cpu.regs_mut()[reg] = value;
        }
        assert_eq!(run_to_svc(&mut cpu, &mut mem), 0);

        let regs = cpu.regs();
        assert_eq!(f32::from_bits(regs[3]), -(2f32.powi(-26)), "one rounding");
        assert_eq!(regs[5] & 0xffff, 0x7c00, "rounded to infinity");
        assert_eq!(f32::from_bits(regs[7]), 1.0);
    }

    /// Thumb NEGS of the most negative number overflows, and a Thumb LDRSH
    /// from an odd address loads a halfword, not ARM7TDMI's sign-extended
    /// byte. The first depends on the core being built with -fwrapv (see its
    /// build.rs), and only an optimised build shows the difference, so run
    /// `cargo test --release` to check that.
    #[test]
    fn thumb_neg_overflow_and_unaligned_ldrsh() {
        let mut mem = Mem::new();
        let code_addr: VAddr = 0x1000;
        write_halfwords(
            &mut mem,
            code_addr,
            &[
                0x4241, // rsbs r1, r0, #0
                0x5f13, // ldrsh r3, [r2, r4]
                0xdf00, // svc #0
            ],
        );
        let data: MutPtr<u8> = Ptr::from_bits(0x2001);
        mem.write(data, 0x34);
        mem.write(data + 1, 0x92);

        let mut cpu = Cpu::new(None);
        cpu.branch(GuestFunction::from_addr_and_thumb_flag(code_addr, true));
        cpu.regs_mut()[0] = 0x80000000;
        cpu.regs_mut()[2] = 0x2001;
        cpu.regs_mut()[4] = 0;
        assert_eq!(run_to_svc(&mut cpu, &mut mem), 0);

        assert_eq!(cpu.regs()[1], 0x80000000);
        assert_eq!(cpu.cpsr() >> 28, 0b1001, "NZCV after NEGS");
        assert_eq!(cpu.regs()[3], 0xffff9234);
    }
}
