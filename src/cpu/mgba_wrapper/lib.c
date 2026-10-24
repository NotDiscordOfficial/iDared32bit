/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/.
 */

#include <stdlib.h>
#include <string.h>

#include "wrapper.h"

/* ARMStep fetches through memory.activeRegion unconditionally -- there is no
 * callback path for instruction fetch, and a NULL region would dereference
 * NULL. touchHLE maps the whole 4GiB guest address space contiguously, so
 * that mapping *is* the active region, and the mask is the identity. */
static void set_fetch_region(struct ARMCore *cpu, void *guest_memory_base) {
  cpu->memory.activeRegion = (const uint32_t *)guest_memory_base;
  cpu->memory.activeMask = 0xFFFFFFFF;
}

/* ARMReset prefetches, so something readable has to be in place before the
 * caller has handed us the guest mapping. A zero mask sends every fetch to
 * this one word. */
static const uint32_t fetch_placeholder[2];

static void set_placeholder_fetch_region(struct ARMCore *cpu) {
  cpu->memory.activeRegion = fetch_placeholder;
  cpu->memory.activeMask = 0;
}

static void halt(struct ARMCore *cpu, int32_t result) {
  struct touchHLE_MgbaWrapper *w = wrapper_of(cpu);
  w->result = result;
  w->halted = true;
}

/* mGBA's memory interface has no way to report a fault, so record it and stop
 * after the current instruction. The faulting access yields zero and the
 * instruction runs to completion on that garbage, which is tolerable because
 * touchHLE treats a memory error as fatal anyway. */
static uint32_t fault(struct ARMCore *cpu) {
  halt(cpu, RESULT_MEMORY_ERROR);
  return 0;
}

/* Data accesses outside the null page go straight to the guest mapping when
 * touchHLE allows direct memory access, which it does unless asked not to
 * (--disable-direct-memory-access). The whole 4GiB is mapped read-write in
 * one piece, so that is exactly what the callbacks below would do once they
 * had checked the address; skipping them saves a call into Rust and its
 * unwind guard on every load and store. Anything the callbacks would reject
 * -- an access starting in the null page, or one running off the top of the
 * address space -- still goes to them, so it is reported the same way. */
static inline uint8_t *direct_data(const struct touchHLE_MgbaWrapper *w,
                                   uint32_t address, uint32_t size) {
  if (w->data_base && address >= w->data_floor &&
      address <= UINT32_MAX - (size - 1)) {
    return w->data_base + address;
  }
  return NULL;
}

/* Word and halfword accesses go to the address as given, not to the aligned
 * address. ARMv4T rotated an unaligned word read and mGBA's GBA memory map
 * implements that, but ARMv6 does a true unaligned access whenever CP15 c1's
 * U bit is set, which is how Darwin configures it -- so that is what a guest
 * built for iPhone OS expects. touchHLE's callbacks
 * read and write unaligned natively. */
static uint32_t load32(struct ARMCore *cpu, uint32_t address, int *cycles) {
  UNUSED(cycles);
  const uint8_t *direct = direct_data(wrapper_of(cpu), address, 4);
  if (direct) {
    uint32_t value;
    memcpy(&value, direct, sizeof(value));
    return value;
  }
  bool error = false;
  uint32_t value = touchHLE_cpu_read_u32(wrapper_of(cpu)->mem, address, &error);
  if (error) {
    return fault(cpu);
  }
  return value;
}

static uint32_t load16(struct ARMCore *cpu, uint32_t address, int *cycles) {
  UNUSED(cycles);
  const uint8_t *direct = direct_data(wrapper_of(cpu), address, 2);
  if (direct) {
    uint16_t value;
    memcpy(&value, direct, sizeof(value));
    return value;
  }
  bool error = false;
  uint32_t value = touchHLE_cpu_read_u16(wrapper_of(cpu)->mem, address, &error);
  if (error) {
    return fault(cpu);
  }
  return value;
}

static uint32_t load8(struct ARMCore *cpu, uint32_t address, int *cycles) {
  UNUSED(cycles);
  const uint8_t *direct = direct_data(wrapper_of(cpu), address, 1);
  if (direct) {
    uint8_t value;
    memcpy(&value, direct, sizeof(value));
    return value;
  }
  bool error = false;
  uint32_t value = touchHLE_cpu_read_u8(wrapper_of(cpu)->mem, address, &error);
  if (error) {
    return fault(cpu);
  }
  return value;
}

static void store32(struct ARMCore *cpu, uint32_t address, int32_t value,
                    int *cycles) {
  UNUSED(cycles);
  uint8_t *direct = direct_data(wrapper_of(cpu), address, 4);
  if (direct) {
    uint32_t v = (uint32_t)value;
    memcpy(direct, &v, sizeof(v));
    return;
  }
  if (touchHLE_cpu_write_u32(wrapper_of(cpu)->mem, address, (uint32_t)value)) {
    halt(cpu, RESULT_MEMORY_ERROR);
  }
}

static void store16(struct ARMCore *cpu, uint32_t address, int16_t value,
                    int *cycles) {
  UNUSED(cycles);
  uint8_t *direct = direct_data(wrapper_of(cpu), address, 2);
  if (direct) {
    uint16_t v = (uint16_t)value;
    memcpy(direct, &v, sizeof(v));
    return;
  }
  if (touchHLE_cpu_write_u16(wrapper_of(cpu)->mem, address, (uint16_t)value)) {
    halt(cpu, RESULT_MEMORY_ERROR);
  }
}

static void store8(struct ARMCore *cpu, uint32_t address, int8_t value,
                   int *cycles) {
  UNUSED(cycles);
  uint8_t *direct = direct_data(wrapper_of(cpu), address, 1);
  if (direct) {
    uint8_t v = (uint8_t)value;
    memcpy(direct, &v, sizeof(v));
    return;
  }
  if (touchHLE_cpu_write_u8(wrapper_of(cpu)->mem, address, (uint8_t)value)) {
    halt(cpu, RESULT_MEMORY_ERROR);
  }
}

/* LDM/STM. mGBA's GBA implementation counts cycles per register and consults
 * the memory map; going one register at a time through the same callbacks is
 * slower but needs no memory map, and touchHLE doesn't model timing. */
static uint32_t loadMultiple(struct ARMCore *cpu, uint32_t baseAddress,
                             int mask, enum LSMDirection direction,
                             int *cycles) {
  uint32_t address = baseAddress;
  int total = __builtin_popcount(mask & 0xFFFF);
  switch (direction) {
  case LSM_IA:
    break;
  case LSM_IB:
    address += 4;
    break;
  case LSM_DA:
    address -= (total - 1) * 4;
    break;
  case LSM_DB:
    address -= total * 4;
    break;
  }
  for (int i = 0; i < 16; i++) {
    if (mask & (1 << i)) {
      cpu->gprs[i] = (int32_t)load32(cpu, address, cycles);
      address += 4;
    }
  }
  switch (direction) {
  case LSM_IA:
  case LSM_IB:
    return baseAddress + total * 4;
  case LSM_DA:
  case LSM_DB:
    return baseAddress - total * 4;
  }
  return baseAddress;
}

static uint32_t storeMultiple(struct ARMCore *cpu, uint32_t baseAddress,
                              int mask, enum LSMDirection direction,
                              int *cycles) {
  uint32_t address = baseAddress;
  int total = __builtin_popcount(mask & 0xFFFF);
  switch (direction) {
  case LSM_IA:
    break;
  case LSM_IB:
    address += 4;
    break;
  case LSM_DA:
    address -= (total - 1) * 4;
    break;
  case LSM_DB:
    address -= total * 4;
    break;
  }
  for (int i = 0; i < 16; i++) {
    if (mask & (1 << i)) {
      store32(cpu, address, cpu->gprs[i], cycles);
      address += 4;
    }
  }
  switch (direction) {
  case LSM_IA:
  case LSM_IB:
    return baseAddress + total * 4;
  case LSM_DA:
  case LSM_DB:
    return baseAddress - total * 4;
  }
  return baseAddress;
}

static void setActiveRegion(struct ARMCore *cpu, uint32_t address) {
  UNUSED(address);
  UNUSED(cpu);
  /* The whole guest address space is one region, so there is nothing to
   * switch; touchHLE_MgbaWrapper_set_fetch_region established it. */
}

static int32_t stall(struct ARMCore *cpu, int32_t wait) {
  UNUSED(cpu);
  return wait;
}

static void swi(struct ARMCore *cpu, int immediate) {
  /* touchHLE dispatches the SVC itself; see Dyld::SVC_*. */
  halt(cpu, immediate);
}

static void hitIllegal(struct ARMCore *cpu, uint32_t opcode) {
  wrapper_of(cpu)->last_undefined_opcode = opcode;
  halt(cpu, RESULT_UNDEFINED_INSTRUCTION);
}

static void bkpt(struct ARMCore *cpu, int immediate) {
  UNUSED(immediate);
  /* Unlike an SVC, where touchHLE wants to resume after the instruction, a
   * breakpoint has to report its own address: GDB stops there and resumes by
   * re-executing it. sync_out already removes one instruction of mGBA's
   * prefetch lead, so back up by one more. */
  cpu->gprs[ARM_PC] -=
      cpu->executionMode == MODE_THUMB ? WORD_SIZE_THUMB : WORD_SIZE_ARM;
  halt(cpu, RESULT_BREAKPOINT);
}

static void nop_core(struct ARMCore *cpu) { UNUSED(cpu); }

/* _ARMSetMode pulls nextEvent down to cycles on every ARM/Thumb switch to
 * force an event check, and ARMRun spins here until the handler pushes the
 * horizon back out -- a no-op would hang on the first interworking branch.
 * touchHLE schedules nothing off this, so the handler has nothing to do but
 * push the horizon out again and reset the counter.
 *
 * The horizon has to be finite even so. cpu->cycles is an int32_t that only
 * ever gets reset here, and ARMRun only calls us once it has reached the
 * horizon -- so at INT32_MAX a multi-cycle instruction starting just short of
 * it overflows the counter before the check can fire, which is undefined
 * behaviour. A guest that stays in ARM mode reaches that in under a minute,
 * since nothing else resets the counter. A lower horizon just means this runs
 * occasionally, which costs nothing. */
#define EVENT_HORIZON 0x40000000
static void processEvents(struct ARMCore *cpu) {
  cpu->cycles = 0;
  cpu->nextEvent = EVENT_HORIZON;
}

static void nop_opcode(struct ARMCore *cpu, uint32_t opcode) {
  wrapper_of(cpu)->last_undefined_opcode = opcode;
  halt(cpu, RESULT_UNDEFINED_INSTRUCTION);
}

/* Move the caller's register file into the core, and refill the prefetch
 * from the new PC. ARMWritePC/ThumbWritePC read r15 as the branch target and
 * leave it one instruction ahead, which is the convention the core runs on. */
static void sync_in(struct touchHLE_MgbaWrapper *w) {
  memcpy(w->cpu.gprs, w->regs, sizeof(w->cpu.gprs));
  if (w->cpu.executionMode == MODE_THUMB) {
    ThumbWritePC(&w->cpu);
  } else {
    ARMWritePC(&w->cpu);
  }
}

/* And back, undoing that one-instruction lead. After an SVC this leaves r15
 * pointing just past the svc, which is what src/environment.rs expects (it
 * subtracts 4 to recover the svc's own address). */
static void sync_out(struct touchHLE_MgbaWrapper *w) {
  memcpy(w->regs, w->cpu.gprs, sizeof(w->regs));
  w->regs[15] -=
      w->cpu.executionMode == MODE_THUMB ? WORD_SIZE_THUMB : WORD_SIZE_ARM;
}

struct touchHLE_MgbaWrapper *
touchHLE_MgbaWrapper_new(void *direct_memory_access_ptr,
                         size_t null_page_count) {
  struct touchHLE_MgbaWrapper *w = calloc(1, sizeof(*w));
  if (!w) {
    return NULL;
  }

  /* With direct memory access, data outside the null page bypasses the
   * callbacks; see direct_data. The null page is set up when the executable
   * is loaded, before the Cpu is created, and never changes. */
  if (direct_memory_access_ptr) {
    w->data_base = direct_memory_access_ptr;
    w->data_floor = null_page_count >= (1u << 20)
                        ? UINT32_MAX
                        : (uint32_t)null_page_count * 0x1000u;
  }

  w->cpu.memory.load32 = load32;
  w->cpu.memory.load16 = load16;
  w->cpu.memory.load8 = load8;
  w->cpu.memory.store32 = store32;
  w->cpu.memory.store16 = store16;
  w->cpu.memory.store8 = store8;
  w->cpu.memory.loadMultiple = loadMultiple;
  w->cpu.memory.storeMultiple = storeMultiple;
  w->cpu.memory.setActiveRegion = setActiveRegion;
  w->cpu.memory.stall = stall;
  if (direct_memory_access_ptr) {
    set_fetch_region(&w->cpu, direct_memory_access_ptr);
  } else {
    set_placeholder_fetch_region(&w->cpu);
  }

  w->cpu.irqh.reset = nop_core;
  w->cpu.irqh.processEvents = processEvents;
  w->cpu.irqh.swi16 = swi;
  w->cpu.irqh.swi32 = swi;
  w->cpu.irqh.hitIllegal = hitIllegal;
  w->cpu.irqh.bkpt16 = bkpt;
  w->cpu.irqh.bkpt32 = bkpt;
  w->cpu.irqh.readCPSR = nop_core;
  w->cpu.irqh.hitStub = nop_opcode;

  /* Deliberately not ARMInit: all it does is zero cp[] (calloc already did)
   * and call into mGBA's component system via cpu->master, which is NULL
   * here and would be dereferenced. ARMDeinit is skipped for the same
   * reason. */
  /* VFP is coprocessors 10 (single precision) and 11 (double). See vfp.c. */
  w->cpu.cp[10].raw = touchHLE_vfp_raw;
  w->cpu.cp[11].raw = touchHLE_vfp_raw;
  /* Advanced SIMD. See neon.c. */
  w->cpu.advancedSimd = touchHLE_neon_raw;
  /* And CP15, for the barriers and thread ID registers. See cp15.c. */
  w->cpu.cp[15].raw = touchHLE_cp15_raw;
  touchHLE_vfp_reset(&w->vfp);

  ARMReset(&w->cpu);
  processEvents(&w->cpu);
  ARMSetPrivilegeMode(&w->cpu, MODE_USER);
  sync_out(w);
  return w;
}

void touchHLE_MgbaWrapper_delete(struct touchHLE_MgbaWrapper *w) {
  if (w) {
    free(w);
  }
}

const uint32_t *
touchHLE_MgbaWrapper_regs_const(const struct touchHLE_MgbaWrapper *w) {
  return w->regs;
}

uint32_t *touchHLE_MgbaWrapper_regs_mut(struct touchHLE_MgbaWrapper *w) {
  return w->regs;
}

uint32_t touchHLE_MgbaWrapper_cpsr(const struct touchHLE_MgbaWrapper *w) {
  return (uint32_t)w->cpu.cpsr.packed;
}

void touchHLE_MgbaWrapper_set_cpsr(struct touchHLE_MgbaWrapper *w,
                                   uint32_t cpsr) {
  w->cpu.cpsr.packed = (int32_t)cpsr;
  /* Applies the T bit through _ARMSetMode, which also keeps the fetch mask
   * in step, and the mode bits through ARMSetPrivilegeMode. */
  _ARMReadCPSR(&w->cpu);
}

/* The encoding that last came back undefined. Only meaningful right after
 * run_or_step has returned RESULT_UNDEFINED_INSTRUCTION. */
uint32_t touchHLE_MgbaWrapper_last_undefined_opcode(
    const struct touchHLE_MgbaWrapper *w) {
  return w->last_undefined_opcode;
}

/* The last unmodelled CP15 access since this was last called, or zero. */
uint32_t
touchHLE_MgbaWrapper_take_unknown_cp15(struct touchHLE_MgbaWrapper *w) {
  uint32_t opcode = w->unknown_cp15_opcode;
  w->unknown_cp15_opcode = 0;
  return opcode;
}

/* Point instruction fetch at the guest's memory. This is separate from the
 * caller's "direct memory access" setting, which is about whether *data*
 * accesses may bypass the touchHLE_cpu_read/write_* callbacks: mGBA fetches
 * instructions straight through the mapping either way, and has no callback
 * path that could replace it. touchHLE can also build a Cpu with no Mem at
 * all, so this cannot be settled at construction. */
void touchHLE_MgbaWrapper_set_fetch_region(struct touchHLE_MgbaWrapper *w,
                                           void *guest_memory_base) {
  set_fetch_region(&w->cpu, guest_memory_base);
}

/* Exchange the whole guest-visible CPU state with the caller's, which is how
 * touchHLE switches guest threads. The extreg array is D0 to D31, exactly the
 * VFP register file. */
void touchHLE_MgbaWrapper_swap_context(struct touchHLE_MgbaWrapper *w,
                                       struct touchHLE_MgbaContext *context) {
  struct touchHLE_MgbaContext tmp;
  memset(&tmp, 0, sizeof(tmp));
  memcpy(tmp.regs, w->regs, sizeof(tmp.regs));
  _Static_assert(sizeof(tmp.extregs) == sizeof(w->vfp.regs),
                 "extregs must be exactly the VFP register file");
  memcpy(tmp.extregs, w->vfp.regs.s, sizeof(w->vfp.regs.s));
  tmp.cpsr = (uint32_t)w->cpu.cpsr.packed;
  tmp.fpscr = w->vfp.fpscr;

  memcpy(w->regs, context->regs, sizeof(context->regs));
  memcpy(w->vfp.regs.s, context->extregs, sizeof(w->vfp.regs.s));
  w->vfp.fpscr = context->fpscr;
  touchHLE_MgbaWrapper_set_cpsr(w, context->cpsr);

  *context = tmp;
}

void touchHLE_MgbaWrapper_invalidate_cache_range(struct touchHLE_MgbaWrapper *w,
                                                 uint32_t start,
                                                 uint32_t size) {
  UNUSED(start);
  UNUSED(size);
  /* Nothing to do: this is a plain interpreter with no decoded-instruction
   * cache. The prefetch pair is refilled from the callbacks on every
   * branch. */
  UNUSED(w);
}

int32_t touchHLE_MgbaWrapper_run_or_step(struct touchHLE_MgbaWrapper *w,
                                         touchHLE_Mem *mem, uint64_t *ticks) {
  w->mem = mem;
  w->result = RESULT_NORMAL;
  w->halted = false;
  /* The caller may have changed the registers -- above all r15, which
   * Cpu::branch writes directly -- since the last run. */
  sync_in(w);

  uint64_t budget = ticks ? *ticks : 1;
  uint64_t executed = 0;
  /* Stepping one instruction at a time rather than using ARMRunLoop, so
   * that a halt is noticed immediately instead of at the next event. */
  while (executed < budget && !w->halted) {
    uint32_t instruction_pc = (uint32_t)w->cpu.gprs[ARM_PC] -
                              (w->cpu.executionMode == MODE_THUMB ? 2u : 4u);
    if (w->cpu.executionMode == MODE_THUMB) {
      instruction_pc |= 1u;
    }
    w->recent_instruction_pcs[w->recent_instruction_pcs_next] = instruction_pc;
    struct touchHLE_MgbaInstructionTrace *trace =
        &w->recent_instruction_trace[w->recent_instruction_pcs_next];
    trace->pc = instruction_pc;
    const uint8_t *instruction =
        (const uint8_t *)w->cpu.memory.activeRegion + (instruction_pc & ~1u);
    if (w->cpu.executionMode == MODE_THUMB) {
      uint16_t opcode;
      memcpy(&opcode, instruction, sizeof(opcode));
      trace->opcode = opcode;
    } else {
      memcpy(&trace->opcode, instruction, sizeof(trace->opcode));
    }
    for (size_t i = 0; i < 16; i++) {
      trace->regs[i] = (uint32_t)w->cpu.gprs[i];
    }
    bool is_call = false;
    uint32_t call_target = 0;
    if (w->cpu.executionMode == MODE_THUMB) {
      uint32_t previous =
          (w->recent_instruction_pcs_next +
           TOUCHHLE_MGBA_INSTRUCTION_TRACE_CAPACITY - 1u) %
          TOUCHHLE_MGBA_INSTRUCTION_TRACE_CAPACITY;
      bool has_previous = w->recent_instruction_pcs_count > 0 &&
                          (w->recent_instruction_pcs[previous] & 1u) != 0;
      uint16_t previous_opcode =
          has_previous ? (uint16_t)w->recent_instruction_trace[previous].opcode
                       : 0;
      uint16_t current_opcode = (uint16_t)trace->opcode;
      if ((current_opcode & 0xff87u) == 0x4780u) {
        uint32_t rm = (current_opcode >> 3) & 0xfu;
        is_call = true;
        call_target = trace->regs[rm];
      } else if (has_previous && (previous_opcode & 0xf800u) == 0xf000u &&
                 ((current_opcode & 0xf800u) == 0xf800u ||
                  (current_opcode & 0xf800u) == 0xe800u)) {
        uint32_t s = (previous_opcode >> 10) & 1u;
        uint32_t j1 = (current_opcode >> 13) & 1u;
        uint32_t j2 = (current_opcode >> 11) & 1u;
        uint32_t i1 = !(j1 ^ s);
        uint32_t i2 = !(j2 ^ s);
        uint32_t immediate = (s << 24) | (i1 << 23) | (i2 << 22) |
                             ((previous_opcode & 0x03ffu) << 12) |
                             ((current_opcode & 0x07ffu) << 1);
        int32_t offset = ((int32_t)(immediate << 7)) >> 7;
        uint32_t base = (w->recent_instruction_pcs[previous] & ~1u) + 4u;
        bool blx_immediate = (current_opcode & 0xf800u) == 0xe800u;
        call_target = (blx_immediate ? (base & ~3u) : base) + offset;
        if (!blx_immediate) {
          call_target |= 1u;
        }
        is_call = true;
      }
    } else {
      uint32_t opcode = trace->opcode;
      if ((opcode & 0x0ffffff0u) == 0x012fff30u) {
        uint32_t rm = opcode & 0xfu;
        is_call = true;
        call_target = trace->regs[rm];
      } else if ((opcode & 0x0f000000u) == 0x0b000000u) {
        int32_t offset = ((int32_t)(opcode << 8)) >> 6;
        is_call = true;
        call_target = instruction_pc + 8u + offset;
      } else if ((opcode & 0xfe000000u) == 0xfa000000u) {
        uint32_t immediate =
            ((opcode & 0x00ffffffu) << 2) | ((opcode >> 23) & 2u);
        int32_t offset = ((int32_t)(immediate << 6)) >> 6;
        is_call = true;
        call_target = ((instruction_pc + 8u + offset) & ~1u) | 1u;
      }
    }
    if (is_call) {
      struct touchHLE_MgbaCallTrace *call =
          &w->recent_call_trace[w->recent_call_trace_next];
      call->pc = instruction_pc;
      call->target = call_target;
      memcpy(call->regs, trace->regs, sizeof(call->regs));
      w->recent_call_trace_next = (w->recent_call_trace_next + 1u) % 128u;
      if (w->recent_call_trace_count < 128u) {
        w->recent_call_trace_count++;
      }
    }
    w->recent_instruction_pcs_next =
        (w->recent_instruction_pcs_next + 1u) %
        TOUCHHLE_MGBA_INSTRUCTION_TRACE_CAPACITY;
    if (w->recent_instruction_pcs_count <
        TOUCHHLE_MGBA_INSTRUCTION_TRACE_CAPACITY) {
      w->recent_instruction_pcs_count++;
    }

    ARMRun(&w->cpu);
    executed++;
  }

  sync_out(w);
  w->mem = NULL;
  if (ticks) {
    *ticks = (executed > *ticks) ? 0 : (*ticks - executed);
  }
  return w->result;
}

size_t touchHLE_MgbaWrapper_copy_recent_instruction_pcs(
    const struct touchHLE_MgbaWrapper *w, uint32_t *output, size_t capacity) {
  if (!output || capacity == 0) {
    return 0;
  }

  size_t count = w->recent_instruction_pcs_count;
  if (count > capacity) {
    count = capacity;
  }
  size_t start = (w->recent_instruction_pcs_next +
                  TOUCHHLE_MGBA_INSTRUCTION_TRACE_CAPACITY - count) %
                 TOUCHHLE_MGBA_INSTRUCTION_TRACE_CAPACITY;
  for (size_t i = 0; i < count; i++) {
    output[i] = w->recent_instruction_pcs[
        (start + i) % TOUCHHLE_MGBA_INSTRUCTION_TRACE_CAPACITY];
  }
  return count;
}

size_t touchHLE_MgbaWrapper_copy_recent_instruction_trace(
    const struct touchHLE_MgbaWrapper *w,
    struct touchHLE_MgbaInstructionTrace *output, size_t capacity) {
  if (!output || capacity == 0) {
    return 0;
  }

  size_t count = w->recent_instruction_pcs_count;
  if (count > capacity) {
    count = capacity;
  }
  size_t start = (w->recent_instruction_pcs_next +
                  TOUCHHLE_MGBA_INSTRUCTION_TRACE_CAPACITY - count) %
                 TOUCHHLE_MGBA_INSTRUCTION_TRACE_CAPACITY;
  for (size_t i = 0; i < count; i++) {
    output[i] = w->recent_instruction_trace[
        (start + i) % TOUCHHLE_MGBA_INSTRUCTION_TRACE_CAPACITY];
  }
  return count;
}

size_t touchHLE_MgbaWrapper_copy_recent_call_trace(
    const struct touchHLE_MgbaWrapper *w, struct touchHLE_MgbaCallTrace *output,
    size_t capacity) {
  if (!output || capacity == 0) {
    return 0;
  }
  size_t count = w->recent_call_trace_count;
  if (count > capacity) {
    count = capacity;
  }
  size_t start = (w->recent_call_trace_next + 128u - count) % 128u;
  for (size_t i = 0; i < count; i++) {
    output[i] = w->recent_call_trace[(start + i) % 128u];
  }
  return count;
}
