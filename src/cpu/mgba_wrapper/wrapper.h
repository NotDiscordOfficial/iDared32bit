/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/.
 */

#ifndef TOUCHHLE_MGBA_WRAPPER_H
#define TOUCHHLE_MGBA_WRAPPER_H

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

#include <mgba/internal/arm/arm.h>
#include <mgba/internal/arm/isa-inlines.h>

typedef void touchHLE_Mem;

/* Exported from the main crate, see src/cpu.rs. Note the polarity: these
 * report *failure*. A read does so through its out-parameter, since any
 * value it could return is a legitimate result; a write returns true. */
uint8_t touchHLE_cpu_read_u8(touchHLE_Mem *mem, uint32_t addr, bool *error);
uint16_t touchHLE_cpu_read_u16(touchHLE_Mem *mem, uint32_t addr, bool *error);
uint32_t touchHLE_cpu_read_u32(touchHLE_Mem *mem, uint32_t addr, bool *error);
bool touchHLE_cpu_write_u8(touchHLE_Mem *mem, uint32_t addr, uint8_t value);
bool touchHLE_cpu_write_u16(touchHLE_Mem *mem, uint32_t addr, uint16_t value);
bool touchHLE_cpu_write_u32(touchHLE_Mem *mem, uint32_t addr, uint32_t value);

/* Mirrors the values src/cpu.rs decodes from run_or_step. */
#define RESULT_NORMAL (-1)
#define RESULT_MEMORY_ERROR (-2)
#define RESULT_UNDEFINED_INSTRUCTION (-3)
#define RESULT_BREAKPOINT (-4)

/* The FPSCR a thread starts with: flush-to-zero on, as iOS sets it (Apple's
 * TN2293), and otherwise the architectural reset value. Mirrored as
 * DEFAULT_FPSCR in lib.rs, for new threads' contexts. */
#define TOUCHHLE_DEFAULT_FPSCR 0x01000000u

/* VFPv3-D32, as on the Cortex-A8 of the iPhone 3GS and later: 32
 * double-precision registers, the low 16 of which are also the 32
 * single-precision ones. (The ARM1176 of earlier devices has VFPv2, with only
 * those 16, which is a subset.) The union gives the aliasing for free on a
 * little-endian host, which every platform touchHLE targets is; `s` is 64
 * words to span all of `d`, but only s[0] to s[31] are architectural. */
struct touchHLE_Vfp {
  union {
    uint32_t s[64];
    uint64_t d[32];
  } regs;
  uint32_t fpscr;
  uint32_t fpsid;
  uint32_t fpexc;
  /* Where an Advanced SIMD load or store whose register list runs past D31
   * puts, and gets, those registers; see neon_dreg in neon.c. Not part of
   * the architectural state, so not saved with a thread's context. */
  uint64_t phantom[8];
};

struct touchHLE_MgbaWrapper {
  /* Must stay first: wrapper_of recovers the wrapper from the core. */
  struct ARMCore cpu;
  struct touchHLE_Vfp vfp;

  /* touchHLE's view of the register file, and the authoritative copy
   * between calls to run_or_step. The core's own gprs cannot be handed out
   * directly, for two reasons:
   *
   * - mGBA keeps r15 one instruction *ahead* of the one that will execute
   *   next, because it maintains a two-entry prefetch. touchHLE expects r15
   *   to be the address that will execute next.
   * - Any change to r15 has to be followed by refilling that prefetch, or
   *   the core goes on executing whatever it had already fetched. touchHLE
   *   writes r15 directly through regs_mut (see Cpu::branch), so there is
   *   no call to hang that refill off.
   *
   * So the shadow is copied in, PC-adjusted and prefetched at the start of
   * a run, and copied back out at the end. */
  uint32_t regs[16];

  /* Only non-NULL for the duration of a run_or_step call. */
  touchHLE_Mem *mem;

  /* Direct access for data (see direct_data in lib.c): the guest mapping,
   * or NULL to use the callbacks for everything, and the end of the null
   * page, below which the callbacks must see the access. */
  uint8_t *data_base;
  uint32_t data_floor;
  int32_t result;
  bool halted;
  /* The opcode that last hit an undefined encoding, for diagnostics. */
  uint32_t last_undefined_opcode;

  /* CP15 state; see cp15.c. */
  uint32_t cp15_tpidrurw;
  /* The last CP15 access that is not modelled, or zero. Cleared when read,
   * so each is logged once. Zero is safe as "none": it does not encode an
   * MCR or MRC. */
  uint32_t unknown_cp15_opcode;

  /* Bounded guest PC history for crash diagnostics. Entries include the Thumb
   * bit. */
  uint32_t recent_instruction_pcs[64];
  uint32_t recent_instruction_pcs_next;
  uint32_t recent_instruction_pcs_count;
};

static inline struct touchHLE_MgbaWrapper *wrapper_of(struct ARMCore *cpu) {
  return containerof(cpu, struct touchHLE_MgbaWrapper, cpu);
}

/* Write a core register from VFP or Advanced SIMD. Rt == 15 is
 * UNPREDICTABLE for all of these transfers; it is taken as a plain branch
 * (see ARMWritePCUnpredictable). */
static inline void touchHLE_write_core_reg(struct ARMCore *cpu, int reg,
                                           uint32_t value) {
  if (reg == ARM_PC) {
    ARMWritePCUnpredictable(cpu, value);
  } else {
    cpu->gprs[reg] = (int32_t)value;
  }
}

/* Mirrors CpuContext in src/cpu.rs. extregs holds D0 to D31, which is the
 * layout touchHLE has always used. */
struct touchHLE_MgbaContext {
  uint32_t regs[16];
  uint32_t extregs[64];
  uint32_t cpsr;
  uint32_t fpscr;
};

size_t touchHLE_MgbaWrapper_copy_recent_instruction_pcs(
    const struct touchHLE_MgbaWrapper *w, uint32_t *output, size_t capacity);

/* src/cpu/mgba_wrapper/vfp.c */
void touchHLE_vfp_reset(struct touchHLE_Vfp *vfp);
bool touchHLE_vfp_raw(struct ARMCore *cpu, uint32_t opcode);

/* src/cpu/mgba_wrapper/neon.c */
bool touchHLE_neon_raw(struct ARMCore *cpu, uint32_t opcode);
bool touchHLE_neon_transfer(struct ARMCore *cpu, uint32_t opcode);

/* src/cpu/mgba_wrapper/cp15.c */
bool touchHLE_cp15_raw(struct ARMCore *cpu, uint32_t opcode);

#endif
