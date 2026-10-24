Warning: truncated output (original token count: 25580)
Total output lines: 2497

/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/.
 */
//! The core of the emulator: management of state, execution, threading.
//!
//! Unlike its siblings, this module should be considered private and only used
//! via the re-exports one level up.

mod airplay_sdk;
pub mod app_picker;
mod mutex;
mod nullable_box;

use crate::abi::{CallFromHost, GuestFunction};
use crate::audio::openal::OpenALManager;
use crate::cpu::Cpu;
use crate::libc::semaphore::sem_t;
use crate::mem::{GuestUSize, MutPtr, MutVoidPtr};
use crate::{
    abi, bundle, cpu, dyld, frameworks, fs, gdb, image, libc, mach_o, mem, objc, options, stack,
    window,
};
use std::cell::Cell;
use std::collections::{HashMap, HashSet, VecDeque};
use std::net::TcpListener;
use std::rc::Rc;
use std::time::{Duration, Instant, SystemTime};

use crate::libc::pthread::cond::pthread_cond_t;
use crate::libc::stdio::FILE;
use crate::window::DeviceFamily;
use corosensei::{Coroutine, Yielder};
pub use mutex::{MutexId, MutexType, PTHREAD_MUTEX_DEFAULT};
use nullable_box::NullableBox;

/// SDL audio subsystem for the mixer: shares the window's context if one
/// exists, else creates a standalone one. `None` if SDL audio is unavailable.
fn build_audio_subsystem(window: Option<&window::Window>) -> Option<sdl2::AudioSubsystem> {
    match window {
        Some(w) => w.create_audio_subsystem().ok(),
        None => sdl2::init().ok().and_then(|sdl| sdl.audio().ok()),
    }
}

/// Index into the [Vec] of threads. Thread 0 is always the main thread.
pub type ThreadId = usize;

pub type HostContext = Coroutine<Environment, Environment, Environment>;

/// Bookkeeping for a thread.
pub struct Thread {
    /// The current running state of the thread. See [ThreadState].
    pub state: ThreadState,
    /// If this is not [ThreadBlock::NotBlocked], the thread is not executing
    /// until a certain condition is fufilled.
    pub blocked_by: ThreadBlock,
    /// Container for thread local state of various child modules
    pub framework_state: frameworks::ThreadLocalState,
    /// After a secondary thread finishes, this is set to the returned value.
    return_value: Option<MutVoidPtr>,
    /// Context object containing the CPU state for this thread.
    ///
    /// There should always be `(threads.len() - 1)` contexts in existence.
    /// When a thread is currently executing, its state is stored directly in
    /// the CPU, rather than in a context object. In that case, this field is
    /// None. See also: [std::mem::take] and [cpu::Cpu::swap_context].
    pub guest_context: Option<Box<cpu::CpuContext>>,
    /// The coroutine associated with this thread.
    ///
    /// In more typical rust, this is equivalent to to a [std::future::Future].
    /// Like a [std::future::Future], it holds the call stack so the inner
    /// function can (cooperatively) suspend execution and be resumed at a
    /// later time. Unlike a [std::future::Future], the call stack is actually
    /// stored as a stack, and not as an anonymous, compiler generated,
    /// (typically heap allocated) object.
    host_context: Option<HostContext>,
    /// Address range of this thread's stack, used to check if addresses are in
    /// range while producing a stack trace.
    pub stack: Option<std::ops::RangeInclusive<u32>>,
}

impl Thread {
    fn is_blocked(&self) -> bool {
        !matches!(self.blocked_by, ThreadBlock::NotBlocked)
    }
    pub fn is_alive(&self) -> bool {
        !matches!(self.state, ThreadState::Dead)
    }
    pub fn is_running(&self) -> bool {
        matches!(self.state, ThreadState::Running | ThreadState::Stepping)
    }
}

impl std::fmt::Debug for Thread {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Thread {{ state: {:?}, blocked_by: {:?}, return_value: {:?} }}",
            self.state, self.blocked_by, self.return_value
        )
    }
}

/// The struct containing the entire emulator state. Methods are provided for
/// execution and management of threads.
pub struct Environment {
    /// Reference point for various timing functions.
    pub startup_time: Instant,
    pub bundle: NullableBox<bundle::Bundle>,
    pub fs: NullableBox<fs::Fs>,
    /// The window is only absent when running in headless mode.
    pub window: Option<Box<window::Window>>,
    pub openal_manager: NullableBox<OpenALManager>,
    pub mem: NullableBox<mem::Mem>,
    /// Loaded binaries. Index `0` is always the app binary, other entries are
    /// dynamic libraries.
    pub bins: Vec<mach_o::MachO>,
    pub objc: NullableBox<objc::ObjC>,
    pub dyld: NullableBox<dyld::Dyld>,
    pub cpu: NullableBox<cpu::Cpu>,
    pub current_thread: ThreadId,
    pub threads: Vec<Thread>,
    pub libc_state: NullableBox<libc::State>,
    pub framework_state: NullableBox<frameworks::State>,
    pub mutex_state: NullableBox<mutex::MutexState>,
    pub options: NullableBox<options::Options>,
    gdb_server: Option<Box<gdb::GdbServer>>,
    pub env_vars: HashMap<Vec<u8>, MutPtr<u8>>,
    /// Set to [true] when created using [Environment::new_without_app].
    pub is_app_picker: bool,
    /// Set to [true] to signal a return to the app picker.
    pub return_to_app_picker: bool,
    /// When an app being quit is given up on, like iOS's watchdog does.
    pub quit_deadline: Option<Instant>,
    pub dump_file: Option<std::fs::File>,
    yielder: *const Yielder<Environment, Environment>,
    // The amount of ticks to run for Some(value), or single-stepping for None.
    // Sadly, setting ticks to 1 does not step properly, so Option is required.
    remaining_ticks: Option<u64>,
    panic_cell: Rc<Cell<Option<Environment>>>,
}

/// What to do next when executing this thread.
enum ThreadNextAction {
    /// Continue CPU emulation.
    Continue,
    /// Return to host.
    ReturnToHost,
    /// Debug the current CPU error.
    DebugCpuError(cpu::CpuError),
}

/// If/what a thread is blocked by.
#[derive(Debug, Clone, PartialEq)]
pub enum ThreadBlock {
    // Default state. (thread is not blocked)
    NotBlocked,
    // Thread is sleeping. (until Instant)
    Sleeping(Instant),
    // Thread is waiting for a mutex to unlock.
    Mutex(MutexId),
    // Thread is waiting on a semaphore.
    Semaphore(MutPtr<sem_t>),
    // Thread is waiting on a condition variable
    Condition(MutPtr<pthread_cond_t>, Option<Duration>),
    // Thread is waiting for another thread to finish (joining).
    Joining(ThreadId, MutPtr<MutVoidPtr>),
    // Thread has hit a cpu error, and is waiting to be debugged.
    WaitingForDebugger(Option<cpu::CpuError>),
    // Thread is suspended. We keep a suspend count and a previous thread state
    // (boxed to avoid cyclic dependency), which would be restored upon
    // resuming.
    Suspended(usize, Box<ThreadBlock>),
    // Thread is waiting on a FILE object lock.
    FileObjectLock(MutPtr<FILE>),
}

impl std::fmt::Display for ThreadBlock {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ThreadBlock::NotBlocked => write!(f, "Running"),
            ThreadBlock::Sleeping(wake_time) => {
                let remaining = wake_time.checked_duration_since(Instant::now());
                match remaining {
                    Some(dur) => write!(f, "Sleeping for {:.3}s", dur.as_secs_f32()),
                    None => write!(f, "Waking"),
                }
            }
            ThreadBlock::Mutex(ptr) => write!(f, "Blocked on mutex {ptr:?}"),
            ThreadBlock::Semaphore(ptr) => write!(f, "Blocked on semaphore {ptr:?}"),
            ThreadBlock::Condition(ptr, _) => write!(f, "Blocked on condition {ptr:?}"),
            // tid adds 1 to match gdb's thread numbers
            ThreadBlock::Joining(tid, _) => write!(f, "Joining on thread {}", tid + 1),
            ThreadBlock::Suspended(count, old) => {
                write!(f, "Suspended (count {}, previously {})", count, old)
            }
            ThreadBlock::FileObjectLock(ptr) => write!(f, "Waiting for file to unlock {ptr:?}"),
            // Unlikely to be seen
            ThreadBlock::WaitingForDebugger(_) => write!(f, "Waiting for debugger"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ThreadState {
    /// The thread is currently running (but may be blocked by a [ThreadBlock])
    Running,
    /// The thread is being stepped by the gdb server.
    Stepping,
    /// The thread is paused by the gdb server, typically because another
    /// thread is stepping and client has asked for other threads not to run.
    Paused,
    /// The thread has died.
    Dead,
}

struct BinaryDependencyNode {
    name: String,
    dependencies: Vec<String>,
}

/// Topologically sorts the binary dylibs using Kahn's algorithm
/// and returns the sorted list of indices
fn generate_binary_load_order(graph: &[BinaryDependencyNode]) -> Result<Vec<usize>, String> {
    let node_to_index: HashMap<_, _> = graph
        .iter()
        .enumerate()
        .map(|(idx, node)| (node.name.as_str(), idx))
        .collect();

    let mut node_dependents = HashMap::new();
    let mut node_in_degrees: HashMap<_, _> = node_to_index.values().map(|&idx| (idx, 0)).collect();

    for node in graph {
        let &bin_index = node_to_index
            .get(node.name.as_str())
            .ok_or_else(|| format!("Failed to find {:?} name mapping", node.name))?;

        // Bin names dont include prefix while dynamic lib paths do
        for dependency in node
            .dependencies
            .iter()
            .map(|path| path.strip_prefix("/usr/lib/").unwrap_or(path.as_str()))
        {
            // Ignore dependencies that are not included in packaged dylibs
            let Some(&dylib_index) = node_to_index.get(dependency) else {
                continue;
            };
            node_dependents
                .entry(dylib_index)
                .or_insert_with(Vec::new)
                .push(bin_index);

            node_in_degrees
                .entry(bin_index)
                .and_modify(|in_degree| *in_degree += 1);
        }
    }

    let mut leaf_nodes: VecDeque<_> = node_in_degrees
        .iter()
        .filter(|(_, &in_degree)| in_degree == 0)
        .map(|(&node, _)| node)
        .collect();

    let mut sorted_indices = Vec::new();

    while let Some(node) = leaf_nodes.pop_front() {
        sorted_indices.push(node);

        let Some(dependents) = node_dependents.get(&node) else {
            continue;
        };

        for &dependant in dependents {
            let Some(in_degree) = node_in_degrees.get_mut(&dependant) else {
                continue;
            };
            *in_degree -= 1;

            if *in_degree == 0 {
                leaf_nodes.push_back(dependant);
            }
        }
    }

    if let Some((&index, _)) = node_in_degrees.iter().find(|(_, &in_degree)| in_degree > 0) {
        return Err(format!(
            "Failed to sort nodes, cycle with {:?}",
            graph.get(index).unwrap().name
        ));
    }
    log!(
        "Found sorted order {:?}",
        sorted_indices
            .iter()
            .map(|&index| graph.get(index).unwrap().name.as_str())
            .collect::<Vec<_>>()
    );

    Ok(sorted_indices)
}

/// Enforces the one (real) Environment limit. See
/// [Environment::with_yielder] for why this is needed.
static ENVIRONMENT_INSTANCE_EXISTS: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

fn nearest_guest_symbol(symbols: &HashMap<String, u32>, pc: u32) -> Option<(&str, u32)> {
    let pc = pc & !1;
    symbols
        .iter()
        .filter_map(|(name, symbol_pc)| {
            let symbol_pc = symbol_pc & !1;
            let distance = pc.checked_sub(symbol_pc)?;
            if distance > 0x100000
                || name.len() > 128
                || !name.is_ascii()
                || name.bytes().any(|byte| byte.is_ascii_control())
            {
                return None;
            }
            Some((name.as_str(), distance))
        })
        .min_by(|(name_a, distance_a), (name_b, distance_b)| {
            distance_a.cmp(distance_b).then_with(|| name_a.cmp(name_b))
        })
}

impl Environment {
    /// Loads the binary and sets up the emulator.
    pub fn new(
        bundle: bundle::Bundle,
        fs: fs::Fs,
        mut options: options::Options,
        app_args: Vec<String>,
    ) -> Result<Environment, String> {
        let startup_time = Instant::now();

        // Enforces the one (real) Environment limit. See `with_yielder` for
        // why this is needed.
        if ENVIRONMENT_INSTANCE_EXISTS.swap(true, std::sync::atomic::Ordering::SeqCst) {
            return Err("Only one (real) Environment can exist at a time!".to_string());
        }
        // If this fails, there's no Environment whose drop would clear it, and
        // the next app could never start.
        struct ClearOnFailure(bool);
        impl Drop for ClearOnFailure {
            fn drop(&mut self) {
                if self.0 {
                    ENVIRONMENT_INSTANCE_EXISTS.store(false, std::sync::atomic::Ordering::SeqCst);
                }
            }
        }
        let mut clear_on_failure = ClearOnFailure(true);

        // Certain apps need to launch in a non-portrait orientation, and this
        // should be handled before creating the window because handling of
        // window rotation after-the-fact is somewhat glitchy.
        // This also ensures the splash screen is correctly oriented.
        if options.initial_orientation == window::DeviceOrientation::Portrait {
            let orientations = bundle.supported_interface_orientations();
            // Many apps list other orientations as well as portrait, but
            // should still start in portrait.
            if !orientations.contains(&"UIInterfaceOrientationPortrait") {
                if let Some(&non_portrait_orientation) = orientations
                    .iter()
                    .find(|&&o| o != "UIInterfaceOrientationPortrait")
                {
                    // TODO: Overwriting the options might not be ideal;
                    //       do we need to distinguish this kind of
                    //       orientation change from others?
                    options.initial_orientation = match non_portrait_orientation {
                        // UIInterfaceOrientation values are flipped relative to
                        // (UI)DeviceOrientation values (content has to rotate
                        // in the opposite direction to how the device rotates).
                        "UIInterfaceOrientationPortraitUpsideDown" => {
                            window::DeviceOrientation::PortraitUpsideDown
                        }
                        "UIInterfaceOrientationLandscapeLeft" => {
                            window::DeviceOrientation::LandscapeRight
                        }
                        "UIInterfaceOrientationLandscapeRight" => {
                            window::DeviceOrientation::LandscapeLeft
                        }
                        // This appears to be an older way set the orientation.
                        // From testing, it seems to correspond to left.
                        "UIInterfaceOrientationLandscape" => {
                            window::DeviceOrientation::LandscapeLeft
                        }
                        other => unimplemented!("Unsupported startup orientation: {:?}", other),
                    };
                    log!("App needs non-portrait user interface orientation {:?}, applying device orientation {:?}.", non_portrait_orientation, options.initial_orientation);
                }
            }
        }

        let device_family_override = options.device_family;
        let device_family_array = bundle.device_family_array();
        let device_family = match device_family_array.len() {
            // iPhone only or iPad only
            1 => {
                let only_supported = device_family_array[0];
                if let Some(dfo) = device_family_override {
                    if dfo != only_supported {
                        log!("Warning: User-defined {:?} device family override is not supported by the app! ignoring", dfo);
                    }
                }
                only_supported
            }
            // iPhone and iPad
            2 => {
                if let Some(dfo) = device_family_override {
                    assert!(device_family_array.contains(&dfo));
                    dfo
                } else if window::host_is_ipad() {
                    // The iPad version makes better use of the screen.
                    DeviceFamily::iPad
                } else {
                    assert!(device_family_array.contains(&DeviceFamily::iPhone));
                    DeviceFamily::iPhone
                }
            }
            _ => unreachable!(),
        };
        log!("{:?} device family is chosen.", device_family);
        options.device_family = Some(device_family);

        let window = if options.headless {
            None
        } else {
            let icon = bundle.load_icon(&fs);
            if let Err(ref e) = icon {
                log!("Warning: {}", e);
            }

            let launch_image_path = bundle.launch_image_path();
            let launch_image = if fs.is_file(&launch_image_path) {
                let res = fs
                    .read(launch_image_path)
                    .map_err(|_| "Could not read launch image file".to_string())
                    .and_then(|bytes| {
                        image::Image::from_bytes(&bytes)
                            .map_err(|e| format!("Could not parse launch image: {e}"))
                    });
                if let Err(ref e) = res {
                    log!("Warning: {}", e);
                };
                res.ok()
            } else {
                None
            };

            Some(Box::new(window::Window::new(
                &format!(
                    "{} (iDared 32bit {}{}{})",
                    bundle.display_name(),
                    super::branding(),
                    if super::branding().is_empty() {
                        ""
                    } else {
                        " "
                    },
                    super::DISPLAY_VERSION
                ),
                icon.ok(),
                launch_image,
                &options,
            )))
        };

        let mut mem = mem::Mem::new();

        let is_spore = bundle.bundle_identifier().starts_with("com.ea.spore");
        let is_critter_crunch = bundle
            .bundle_identifier()
            .starts_with("com.capybaragames.CritterCrunch")
            || bundle
                .bundle_identifier()
                .starts_with("com.go.starwave.CritterCrunch");
        // We always reset this flag depending on which game is launched.
        mem.zero_memory_on_free = !is_spore && !is_critter_crunch;
        if is_spore {
            log!("Applying game-specific hack for Spore Origins: zeroing memory on alloc instead of free.");
        }
        if is_critter_crunch {
            // Without this hack, every time a critter 'explodes',
            // the game crashes with a null page access error.
            log!("Applying game-specific hack for Critter Crunch: zeroing memory on alloc instead of free.");
        }
        let executable = mach_o::MachO::load_from_file(
            bundle.executable_path(),
            &fs,
            &mut mem,
            /* slide: */ 0,
        )
        .map_err(|e| format!("Could not load executable: {e}"))?;

        // This needs the executable's code as it was before linking.
        airplay_sdk::fix_code_hash(&executable, &mut mem);

        let mut dylibs = Vec::new();
        for dylib in &executable.dynamic_libraries {
            // There are some Free Software libraries bundled with touchHLE and
            // exposed via the guest file system (see Fs::new()).
            let dylib_path = fs::GuestPath::new(dylib);
            if fs.is_file(dylib_path) {
                // We use hardcoded slide values for libgcc and libstdc++
                // based on base addresses of those dylibs prior to iOS 3.1
                // TODO: implement some kind of ASLR instead of hardcoding
                assert!(dylib_path.as_str().starts_with("/usr/lib/"));
                let name = dylib_path.file_name().unwrap();
                let dylib_slide = match name {
                    "libstdc++.6.dylib" | "libstdc++.6.0.9.dylib" => 0x3748a000,
                    "libgcc_s.1.dylib" => 0x30000000,
                    "libz.1.dylib" | "libz.1.2.3.dylib" | "libz.dylib" | "libz.1.1.3.dylib" => {
                        // We build `libz` from sources with our OSS toolchain,
                        // the base address is already set and sliding is not
                        // needed.
                        0
                    }
                    "libsqlite3.dylib" | "libsqlite3.0.dylib" => {
                        // We build `libsqlite3` from sources with our OSS
                        // toolchain, the base address is already set and
                        // sliding is not needed.
                        0
                    }
                    "libxml2.2.dylib" | "libxml2.dylib" | "libxml2.2.7.8.dylib" => {
                        // We build `libxml2` from sources with our OSS
                        // toolchain, the base address is already set and
                        // sliding is not needed.
                        0
                    }
                    _ => unimplemented!("Unknown binary slide for {}", name),
                };
                let dylib = mach_o::MachO::load_from_file(
                    fs::GuestPath::new(dylib),
                    &fs,
                    &mut mem,
                    dylib_slide,
                )
                .map_err(|e| format!("Could not load bundled dylib: {e}"))?;
                dylibs.push(dylib);
            // Otherwise, look for it in our host implementations.
            } else if !crate::dyld::DYLIB_LIST
                .iter()
                .any(|d| d.path == dylib || d.aliases.contains(&dylib.as_str()))
            {
                log!(
                    "Warning: app binary depends on unimplemented or missing dylib \"{}\"",
                    dylib
                );
            }
        }

        let entry_point_addr = executable
            .entry_point_pc
            .ok_or_else(|| {
                "Mach-O file does not specify an entry point PC, perhaps it is not an executable?"
                    .to_string()
            })
            .unwrap();
        let entry_point_addr = abi::GuestFunction::from_addr_with_thumb_bit(entry_point_addr);

        log_dbg!("Address of start function: {:?}", entry_point_addr);

        let mut bins = dylibs;
        bins.insert(0, executable);

        let mut objc = objc::ObjC::new();

        let mut dyld = dyld::Dyld::new();
        dyld.do_initial_linking(&bins, &mut mem, &mut objc)?;

        let cpu = cpu::Cpu::new(match options.direct_memory_access {
            true => Some(&mut mem),
            false => None,
        });

        let main_thread_init_routine = Coroutine::new(move |yielder, mut env: Environment| {
            let res = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                env.with_yielder(yielder, move |env| {
                    echo!("CPU emulation begins now.");
                    // Some apps use the stack inside the static initializer.
                    // While properly behaving apps should be fine, some app
                    // will try to poke the top of the stack, so we'll give
                    // it some room.
                    env.cpu.regs_mut()[Cpu::SP] = 0xFFFFF000;

                    // Call `+load` method on classes where it's defined.
                    // TODO: `+load` methods from our image should take priority
                    // over frameworks ones.
                    // TODO: a category `+load` method should be called after
                    // the class's own +load method.
                    // Note: `+load` is sent without triggering `+initialize`,
                    // matching the runtime's guarantee that `+load` runs first.
                    let mut to_be_loaded = Vec::new();
                    let mut processed = HashSet::new();
                    let load_sel: objc::SEL = env
                        .objc
                        .register_host_selector("load".to_string(), &mut env.mem);
                    for (class_name, &class) in env.objc.all_classes() {
                        if processed.contains(&class) {
                            continue;
                        }
                        if env.objc.is_unimplemented_class(class) || env.objc.is_fake_class(class) {
                            continue;
                        }
                        if env
                            .objc
                            .object_has_uninherited_method(&env.mem, class, load_sel)
                        {
                            log_dbg!("Calling +load on inheritance chain of {} class", class_name);
                            let mut inherited = Vec::new();
                            let mut curr_class = class;
                            while curr_class != objc::nil
                                && !env.objc.is_unimplemented_class(curr_class)
                                && !env.objc.is_fake_class(curr_class)
                            {
                                if !processed.contains(&curr_class)
                                    && env.objc.object_has_uninherited_method(
                                        &env.mem, curr_class, load_sel,
                                    )
                                {
                                    inherited.push(curr_class);
                                    processed.insert(curr_class);
                                }
                                curr_class = env.objc.get_superclass(curr_class);
                            }
                            to_be_loaded.extend(inherited.into_iter().rev());
                        }
                    }
                    for &class in &to_be_loaded {
                        () = objc::msg_send_no_initialize(env, (class, load_sel));
                    }

                    // Static initializers for libraries must be run before
                    // the initializer in the app binary.
                    for bin_idx in env.get_sorted_bin_indices().unwrap() {
                        let Some(bin) = env.bins.get(bin_idx) else {
                            continue;
                        };
                        let Some(section) =
                            bin.get_section(mach_o::SectionType::ModInitFuncPointers)
                        else {
                            continue;
                        };

                        log_dbg!("Calling static initializers for {:?}", bin.name);
                        assert!(section.size % 4 == 0);
                        let base: mem::ConstPtr<abi::GuestFunction> =
                            mem::Ptr::from_bits(section.addr);
                        let count = section.size / 4;
                        for i in 0..count {
                            let func = env.mem.read(base + i);
                            log_dbg!(
                                "Calling static initializer at {:?} from {:?}",
                                func,
                                (base + i)
                            );
                            () = func.call_from_host(env, ());
                        }
                        log_dbg!("Static initialization done");
                    }

                    {
                        let bin_path = env.bundle.executable_path();

                        let envp_list: Vec<String> = env
                            .env_vars
                            .clone()
                            .iter_mut()
                            .map(|tuple| {
                                [
                                    std::str::from_utf8(tuple.0).unwrap(),
                                    "=",
                                    env.mem.cstr_at_utf8(*tuple.1).unwrap(),
                                ]
                                .concat()
                            })
                            .collect();
                        let envp_ref_list: Vec<&str> =
                            envp_list.iter().map(|keyvalue| keyvalue.as_str()).collect();

                        let bin_path_apple_key = format!("executable_path={}", bin_path.as_str());

                        let argv = Vec::from_iter(
                            std::iter::once(bin_path.as_str())
                                .chain(app_args.iter().map(|s| s.as_str())),
                        );
                        let envp = envp_ref_list.as_slice();
                        let apple = &[bin_path_apple_key.as_str()];
                        stack::prep_stack_for_start(&mut env.mem, &mut env.cpu, &argv, envp, apple);
                    }

                    // Manually call here, since running call_from_host pushes
                    // a stack frame and disrupts abi for _start.
                    env.cpu
                        .branch_with_link(entry_point_addr, env.dyld.thread_exit_routine());
                    env.run_call();

                    if !env.return_to_app_picker {
                        panic!("Main function exited unexpectedly!");
                    }
                })
            }));
            if let Err(e) = res {
                let panic_cell = env.panic_cell.clone();
                panic_cell.set(Some(env));
                std::panic::resume_unwind(e);
            }
            env
        });
        let main_thread = Thread {
            state: ThreadState::Running,
            blocked_by: ThreadBlock::NotBlocked,
            return_value: None,
            guest_context: None,
            host_context: Some(main_thread_init_routine),
            stack: Some(mem::Mem::MAIN_THREAD_STACK_LOW_END..=0u32.wrapping_sub(1)),
            framework_state: Default::default(),
        };

        let audio_subsystem = build_audio_subsystem(window.as_deref());

        let mut env = Environment {
            startup_time,
            bundle: NullableBox::new(bundle),
            fs: NullableBox::new(fs),
            window,
            openal_manager: NullableBox::new(OpenALManager::new(audio_subsystem)?),
            mem: NullableBox::new(mem),
            bins,
            objc: NullableBox::new(objc),
            dyld: NullableBox::new(dyld),
            cpu: NullableBox::new(cpu),
            current_thread: 0,
            threads: vec![main_thread],
            libc_state: Default::default(),
            mutex_state: Default::default(),
            framework_state: Default::default(),
            options: NullableBox::new(options),
            gdb_server: None,
            env_vars: Default::default(),
            is_app_picker: false,
            return_to_app_picker: false,
            quit_deadline: None,
            dump_file: None,
            yielder: std::ptr::null(),
            remaining_ticks: None,
            panic_cell: Rc::new(Cell::new(None)),
        };

        // Apps get the on-screen Home button (where there is one), the app
        // picker doesn't.
        if let Some(window) = env.window.as_mut() {
            window.enable_on_screen_home_button();
        }

        if env.options.dumping_options.any() {
            env.dump_file =
            …9580 tokens truncated…se] if it should resume normal execution.
    fn debug_cpu_error(&mut self, error: cpu::CpuError) {
        if matches!(error, cpu::CpuError::UndefinedInstruction)
            || matches!(error, cpu::CpuError::Breakpoint)
        {
            // Rewind the PC so that it's at the instruction where the error
            // occurred, rather than the next instruction. This is necessary for
            // GDB to detect its software breakpoints. For some reason this
            // isn't correct for memory errors however.
            let instruction_len = if (self.cpu.cpsr() & cpu::Cpu::CPSR_THUMB) != 0 {
                2
            } else {
                4
            };
            self.cpu.regs_mut()[cpu::Cpu::PC] -= instruction_len;
        }

        if self.gdb_server.is_none() {
            panic!("Error during CPU execution: {error:?}");
        }

        echo!("Debuggable error during CPU execution: {:?}.", error);
        self.enter_debugger(Some(error))
    }

    /// Used to check whether a debugger is connected, and therefore whether
    /// [Environment::enter_debugger] will do something.
    pub fn is_debugging_enabled(&self) -> bool {
        self.gdb_server.is_some()
    }

    #[cfg(test)]
    pub fn new_fake_for_test() -> Self {
        let options = options::Options::default();
        let mut mem = mem::Mem::new();
        let mut objc = objc::ObjC::new();
        let mut dyld = dyld::Dyld::new();
        dyld.do_initial_linking_with_no_bins(&mut mem, &mut objc);
        let cpu = cpu::Cpu::new(None);

        Environment {
            startup_time: Instant::now(),
            bundle: NullableBox::new(bundle::Bundle::new_fake_bundle()),
            fs: NullableBox::new(fs::Fs::new_fake_fs()),
            window: None,
            openal_manager: unsafe { NullableBox::null() },
            mem: NullableBox::new(mem),
            bins: Vec::new(),
            objc: NullableBox::new(objc),
            dyld: NullableBox::new(dyld),
            cpu: NullableBox::new(cpu),
            current_thread: 0,
            threads: Vec::new(),
            libc_state: Default::default(),
            mutex_state: Default::default(),
            framework_state: Default::default(),
            options: NullableBox::new(options),
            gdb_server: None,
            env_vars: Default::default(),
            dump_file: None,
            is_app_picker: false,
            return_to_app_picker: false,
            quit_deadline: None,
            yielder: std::ptr::null(),
            remaining_ticks: None,
            panic_cell: Rc::new(Cell::new(None)),
        }
    }

    /// Suspend execution and hand control to the connected debugger.
    /// You should precede this call with a log message that explains why the
    /// debugger is being invoked. The return value is the same as
    /// [gdb::GdbServer::wait_for_debugger]'s.
    ///
    /// Note that this also yields the thread - take care!
    pub fn enter_debugger(&mut self, reason: Option<cpu::CpuError>) {
        // GDB doesn't seem to manage to produce a useful stack trace, so
        // let's print our own.
        self.stack_trace_current();

        self.yield_thread(ThreadBlock::WaitingForDebugger(reason));
    }

    #[inline(always)]
    /// Respond to the new CPU state (do nothing, execute an SVC or enter
    /// debugging) and decide what to do next.
    fn handle_cpu_state(&mut self, state: cpu::CpuState) -> ThreadNextAction {
        match state {
            cpu::CpuState::Normal => ThreadNextAction::Continue,
            cpu::CpuState::Svc(svc) => {
                // The program counter is pointing at the
                // instruction after the SVC, but we want the
                // address of the SVC itself.
                let svc_pc = self.cpu.regs()[cpu::Cpu::PC] - 4;
                match svc {
                    dyld::Dyld::SVC_RETURN_TO_HOST => {
                        assert!(
                            svc_pc == self.dyld.return_to_host_routine().addr_without_thumb_bit()
                        );
                        // Normal return from host-to-guest call.
                        ThreadNextAction::ReturnToHost
                    }
                    dyld::Dyld::SVC_LAZY_LINK
                    | dyld::Dyld::SVC_LAZY_LINK_RET_FLAG
                    | dyld::Dyld::SVC_LINKED_FUNCTIONS_BASE.. => {
                        if let Some(f) = self.dyld.get_svc_handler(
                            &self.bins,
                            &mut self.mem,
                            &mut self.cpu,
                            svc_pc,
                            svc,
                        ) {
                            f.call_from_guest(self);
                            if let Some(len) = self.options.zero_stack_after_guest_to_host_call {
                                log_once!("Applying zeroing of stack after guest to host call.");
                                let start = self.cpu.regs()[cpu::Cpu::SP] - len;
                                self.mem
                                    .bytes_at_mut(mem::Ptr::from_bits(start), len)
                                    .fill(0);
                            }

                            if self.return_to_app_picker {
                                return ThreadNextAction::ReturnToHost;
                            }

                            // On entry_size 4 return here since there's
                            // no space to add a ret after the svc call
                            if svc & dyld::Dyld::SVC_LAZY_LINK_RET_FLAG != 0 {
                                let lr = self.cpu.regs()[cpu::Cpu::LR];
                                self.cpu.branch(GuestFunction::from_addr_with_thumb_bit(lr));
                            }
                            ThreadNextAction::Continue
                        } else {
                            self.cpu.regs_mut()[cpu::Cpu::PC] = svc_pc;
                            ThreadNextAction::Continue
                        }
                    }
                    dyld::Dyld::SVC_THREAD_EXIT => {
                        unimplemented!("TODO: implement exit routines for threads!")
                    }
                }
            }
            cpu::CpuState::Error(e) => ThreadNextAction::DebugCpuError(e),
        }
    }

    fn run_inner(&mut self) {
        let initial_thread = self.current_thread;
        assert!(self.threads[initial_thread].is_running());
        assert!(self.threads[initial_thread].guest_context.is_none());

        loop {
            while self
                .remaining_ticks
                .is_none_or(|remaining_ticks| remaining_ticks > 0)
            {
                let state = self
                    .cpu
                    .run_or_step(&mut self.mem, self.remaining_ticks.as_mut());

                match self.handle_cpu_state(state) {
                    ThreadNextAction::Continue => {
                        if self.remaining_ticks.is_none() {
                            break;
                        }
                    }
                    ThreadNextAction::ReturnToHost => return,
                    ThreadNextAction::DebugCpuError(e) => {
                        self.debug_cpu_error(e);
                    }
                }
            }
            self.yield_thread(ThreadBlock::NotBlocked);
        }
    }

    /// Yield the current thread, suspending execution and handing control back
    /// to the executor ([Self::run]), waiting until the current `thread_block`
    /// condition is met.
    pub fn yield_thread(&mut self, thread_block: ThreadBlock) {
        assert!(!self.threads[self.current_thread].is_blocked());
        log_dbg!(
            "Thread {} yielding on {:?}",
            self.current_thread,
            thread_block
        );
        unsafe {
            self.threads[self.current_thread].blocked_by = thread_block;
            let yielder = self.yielder.as_ref().unwrap();
            self.yielder = std::ptr::null();
            let panic_cell = self.panic_cell.clone();
            let res = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let env = std::mem::replace(self, Self::new_fake());
                yielder.suspend(env)
            }));
            match res {
                Ok(env) => {
                    let _ = std::mem::replace(self, env);
                    self.yielder = yielder;
                }
                Err(payload) => {
                    let Some(env) = panic_cell.take() else {
                        log_no_panic!("Did not recieve env for coroutine unwind, must abort!");
                        std::process::exit(-1)
                    };
                    let _ = std::mem::replace(self, env);
                    self.yielder = yielder;
                    std::panic::resume_unwind(payload);
                }
            }
        }
        assert!(!self.threads[self.current_thread].is_blocked());
    }

    /// Find the next thread to execute, and set it up to be switched to.
    ///
    /// This also handles all the required bookkeeping (unlocking mutexes,
    /// decrementing semaphores, setting the thread to be unblocked, etc.).
    /// It is not required that the thread is switched to immediately.
    fn schedule_next_thread(&mut self) -> ThreadId {
        // GDB can allow schedule_next_thread to be called twice in a row -
        // we make sure that this works by immediately fufilling conditions
        // (relocking mutexes, decrementing semaphores, etc.)!
        loop {
            // Try to find a new thread to execute, starting with the thread
            // following the one currently executing.
            let mut next_awakening: Option<Instant> = None;
            for i in 0..self.threads.len() {
                let thread_id = (self.current_thread + 1 + i) % self.threads.len();
                let candidate = &mut self.threads[thread_id];

                if !candidate.is_running() {
                    continue;
                }
                match candidate.blocked_by {
                    ThreadBlock::Sleeping(sleeping_until) => {
                        if sleeping_until <= Instant::now() {
                            log_dbg!("Thread {} finished sleeping.", thread_id);
                            candidate.blocked_by = ThreadBlock::NotBlocked;
                            return thread_id;
                        } else {
                            next_awakening = match next_awakening {
                                None => Some(sleeping_until),
                                Some(other) => Some(other.min(sleeping_until)),
                            };
                        }
                    }
                    ThreadBlock::Mutex(mutex_id) => {
                        if !self.mutex_state.mutex_is_locked(mutex_id) {
                            log_dbg!("Thread {} was unblocked due to mutex #{} unlocking, relocking mutex.", thread_id, mutex_id);
                            self.threads[thread_id].blocked_by = ThreadBlock::NotBlocked;
                            self.relock_unblocked_mutex_for_thread(thread_id, mutex_id);
                            return thread_id;
                        }
                    }
                    ThreadBlock::Semaphore(sem) => {
                        let host_sem_rc: &mut _ = self
                            .libc_state
                            .semaphore
                            .open_semaphores
                            .get_mut(&sem)
                            .unwrap();
                        let mut host_sem = (*host_sem_rc).borrow_mut();

                        if host_sem.value > 0 {
                            log_dbg!(
                                "Thread {} has awaken on semaphore {:?} with value {}",
                                thread_id,
                                sem,
                                host_sem.value
                            );
                            host_sem.value -= 1;
                            host_sem.waiting.remove(&self.current_thread);
                            self.threads[thread_id].blocked_by = ThreadBlock::NotBlocked;
                            return thread_id;
                        }
                    }
                    ThreadBlock::Condition(cond, deadline) => {
                        let host_cond = self
                            .libc_state
                            .pthread
                            .cond
                            .condition_variables
                            .get_mut(&cond)
                            .unwrap();
                        let mutex = host_cond.curr_mutex.unwrap();
                        if host_cond
                            .waking
                            .front()
                            .is_some_and(|waking_thread| *waking_thread == thread_id)
                            && !self.mutex_state.mutex_is_locked(mutex)
                        {
                            log_dbg!("Thread {} is unblocking on cond var {:?}.", thread_id, cond);
                            host_cond.waking.pop_front();
                            self.threads[thread_id].blocked_by = ThreadBlock::NotBlocked;
                            self.relock_unblocked_mutex_for_thread(thread_id, mutex);
                            return thread_id;
                        } else if let Some(deadline) = deadline {
                            let time = SystemTime::now()
                                .duration_since(SystemTime::UNIX_EPOCH)
                                .unwrap();
                            if deadline > time {
                                // Make sure we wake up for the timeout if
                                // every thread is blocked.
                                let timeout_at = Instant::now() + (deadline - time);
                                next_awakening = Some(match next_awakening {
                                    None => timeout_at,
                                    Some(other) => other.min(timeout_at),
                                });
                            } else if !host_cond.waking.contains(&thread_id)
                                && !self.mutex_state.mutex_is_locked(mutex)
                            {
                                // A thread that has already been signalled
                                // will wake up normally, and a timed-out
                                // thread still has to wait to get the mutex
                                // back.
                                log_dbg!(
                                    "Thread {} is timed out on cond var {:?}.",
                                    thread_id,
                                    cond
                                );
                                assert!(!host_cond.timed_out.contains(&thread_id));
                                host_cond.timed_out.insert(thread_id);

                                host_cond.waiting.retain(|&t| t != thread_id);

                                self.threads[thread_id].blocked_by = ThreadBlock::NotBlocked;
                                self.relock_unblocked_mutex_for_thread(thread_id, mutex);
                                return thread_id;
                            }
                        }
                    }
                    ThreadBlock::Joining(joinee_thread, ptr) => {
                        if !self.threads[joinee_thread].is_alive() {
                            log_dbg!(
                                "Thread {} joining with now finished thread {}.",
                                self.current_thread,
                                joinee_thread
                            );
                            // Write the return value, unless the pointer to
                            // write to is null.
                            if !ptr.is_null() {
                                self.mem
                                    .write(ptr, self.threads[joinee_thread].return_value.unwrap());
                            }
                            self.threads[thread_id].blocked_by = ThreadBlock::NotBlocked;
                            return thread_id;
                        }
                    }
                    ThreadBlock::NotBlocked => {
                        return thread_id;
                    }
                    ThreadBlock::WaitingForDebugger(_) => unreachable!(),
                    ThreadBlock::Suspended(cnt, _) => {
                        assert!(cnt > 0);
                    }
                    ThreadBlock::FileObjectLock(file_ptr) => {
                        // TODO: fairness
                        let acquired = self.libc_state.stdio.try_acquire_file_object_lock(
                            &mut self.mem,
                            file_ptr,
                            thread_id,
                        );
                        if acquired {
                            self.threads[thread_id].blocked_by = ThreadBlock::NotBlocked;
                            return thread_id;
                        }
                    }
                }
            }

            // All suitable threads are blocked and at least one is asleep.
            // Sleep until one of them wakes up.
            if let Some(next_awakening) = next_awakening {
                let duration = next_awakening.duration_since(Instant::now());
                log_dbg!("All threads blocked/asleep, sleeping for {:?}.", duration);
                std::thread::sleep(duration);
                // Try again, there should be some thread awake now (or
                // there will be soon, since timing is approximate).
                continue;
            } else {
                // This should hopefully not happen, but if a thread is
                // blocked on another thread waiting for a deferred return,
                // it could.
                panic!("No active threads, program has deadlocked!");
            }
        }
    }

    fn set_up_initial_env_vars(&mut self) {
        // TODO: Provide all the system environment variables an app might
        // expect to find.

        // Initialize HOME envvar
        let home_value_cstr = self
            .mem
            .alloc_and_write_cstr(self.fs.home_directory().as_str().as_bytes());
        self.env_vars.insert(b"HOME".to_vec(), home_value_cstr);
    }

    fn get_sorted_bin_indices(&self) -> Result<Vec<usize>, String> {
        let dylib_graph: Vec<BinaryDependencyNode> = self
            .bins
            .iter()
            .map(|bin| BinaryDependencyNode {
                name: bin.name.clone(),
                dependencies: bin.dynamic_libraries.clone(),
            })
            .collect();

        generate_binary_load_order(&dylib_graph)
    }

    /// Run a function using window and options on the parent stack if we are
    /// inside a coroutine, or run it directly if we aren't. Some
    /// [window::Window] functions require to be called inside this function.
    ///
    /// Android's ABI seems to dislike if certain functions aren't called from
    /// the main stack. Since corosensei uses seperate stacks to run
    /// coroutines, Android doesn't recognize it as the main stack, so those
    /// functions need to be run on the main stack. Unfortunately, there's no
    /// documentation of which functions need to be called with this, so we
    /// have to check ourselves.
    pub fn on_parent_stack_in_coroutine<F, R>(&mut self, f: F) -> R
    where
        F: FnOnce(&mut window::Window, &mut options::Options) -> R + Send,
    {
        struct WindowWrapper<'a> {
            window: &'a mut window::Window,
        }
        // SAFETY: we're not sending across threads, we're only sending across
        // the coroutine boundary so it's ok.
        unsafe impl Send for WindowWrapper<'_> {}

        if !self.yielder.is_null() {
            unsafe {
                let yielder = self.yielder.as_ref().unwrap();
                let wrapped = WindowWrapper {
                    window: self.window.as_mut().unwrap(),
                };
                let res = yielder.on_parent_stack(|| {
                    let wrapped = wrapped;
                    wrapped.window.on_main_stack = true;
                    f(wrapped.window, self.options.as_mut())
                });
                self.window.as_mut().unwrap().on_main_stack = false;
                res
            }
        } else {
            if let Some(w) = self.window.as_mut() {
                w.on_main_stack = true;
            }
            f(self.window.as_mut().unwrap(), self.options.as_mut())
        }
    }
}

impl Drop for Environment {
    // Clean up all the remaining HostContexts. This isn't strictly required,
    // since this should only occur after a sucessful panic or the app ending,
    // but it is a bit cleaner and avoids confusion inside the logs.
    fn drop(&mut self) {
        if self.objc.is_null() {
            return;
        }
        if let Some(w) = self.window.as_mut() {
            w.on_main_stack = false;
        }
        if self.threads.is_empty()
            || self
                .threads
                .iter()
                .all(|thread| thread.host_context.is_none())
        {
            ENVIRONMENT_INSTANCE_EXISTS.store(false, std::sync::atomic::Ordering::SeqCst);
            return;
        }
        unsafe {
            let mut env = std::mem::replace(self, Environment::new_fake());
            let panic_cell = env.panic_cell.clone();
            let threads_len = env.threads.len();
            for i in 0..threads_len {
                let host_context = env.threads[i].host_context.take();
                panic_cell.set(Some(env));
                std::mem::drop(host_context);
                env = panic_cell.take().unwrap_or_else(|| {
                    log_no_panic!(
                        "Did not recieve env from coroutine unwind during drop, must abort!"
                    );
                    std::process::exit(-1)
                });
            }
            *self = env;
        }
        ENVIRONMENT_INSTANCE_EXISTS.store(false, std::sync::atomic::Ordering::Relaxed);
    }
}

#[cfg(test)]
mod dylib_sorting_tests {
    use std::collections::HashSet;

    use super::*;

    fn create_dylib_graph(bin_configs: &[(&str, &[&str])]) -> Vec<BinaryDependencyNode> {
        bin_configs
            .iter()
            .map(|(name, dependencies)| BinaryDependencyNode {
                name: name.to_string(),
                dependencies: dependencies.iter().map(|s| s.to_string()).collect(),
            })
            .collect()
    }

    /// Verify dylib sort by checking that no dependents are needed
    /// before their import
    fn verify_sort(graph: &[BinaryDependencyNode], sorted_indices: &[usize]) {
        assert_eq!(sorted_indices.len(), graph.len());

        let bin_to_index: HashMap<_, _> = graph
            .iter()
            .enumerate()
            .map(|(idx, node)| (node.name.as_str(), idx))
            .collect();

        let mut loaded_dylibs = HashSet::new();

        for &index in sorted_indices {
            let current_bin = graph.get(index).unwrap();

            for dependency in current_bin
                .dependencies
                .iter()
                .map(|path| path.strip_prefix("/usr/lib/").unwrap_or(path.as_str()))
            {
                // Ignore dependencies that are not included in packaged dylibs
                let Some(&dylib_index) = bin_to_index.get(dependency) else {
                    continue;
                };

                assert!(loaded_dylibs.contains(&dylib_index));
            }

            loaded_dylibs.insert(index);
        }
    }

    #[test]
    fn test_no_dependencies() {
        let dylib_graph = create_dylib_graph(&[]);
        let sorted_indices = generate_binary_load_order(&dylib_graph).unwrap();
        verify_sort(&dylib_graph, &sorted_indices);
    }

    #[test]
    fn test_single_bin() {
        let dylib_graph = create_dylib_graph(&[("A", &[])]);

        let sorted_indices = generate_binary_load_order(&dylib_graph).unwrap();

        verify_sort(&dylib_graph, &sorted_indices);
    }

    #[test]
    fn test_linear_dependencies() {
        // A -> B -> C -> D
        let dylib_graph = create_dylib_graph(&[
            ("A", &[]),
            ("B", &["/usr/lib/A"]),
            ("C", &["/usr/lib/B"]),
            ("D", &["/usr/lib/C"]),
        ]);

        let sorted_indices = generate_binary_load_order(&dylib_graph).unwrap();

        verify_sort(&dylib_graph, &sorted_indices);
    }

    #[test]
    fn test_diamond_dependencies() {
        // A -> B -> D
        //  \-> C -/
        let dylib_graph =
            create_dylib_graph(&[("A", &[]), ("B", &["A"]), ("C", &["A"]), ("D", &["B", "C"])]);

        let sorted_indices = generate_binary_load_order(&dylib_graph).unwrap();

        verify_sort(&dylib_graph, &sorted_indices);
    }

    #[test]
    fn test_with_isolated_nodes() {
        // A -> B
        // C
        // D
        let dylib_graph = create_dylib_graph(&[("A", &[]), ("B", &["A"]), ("C", &[]), ("D", &[])]);

        let sorted_indices = generate_binary_load_order(&dylib_graph).unwrap();

        verify_sort(&dylib_graph, &sorted_indices);
    }

    #[test]
    fn test_complex_dependency_graph() {
        // A -> B -> D
        // A -> C -> E
        // F -> G
        // H
        let dylib_graph = create_dylib_graph(&[
            ("A", &[]),
            ("B", &["A"]),
            ("C", &["A"]),
            ("D", &["B"]),
            ("E", &["C"]),
            ("F", &[]),
            ("G", &["F"]),
            ("H", &[]),
        ]);

        let sorted_indices = generate_binary_load_order(&dylib_graph).unwrap();

        verify_sort(&dylib_graph, &sorted_indices);
    }

    #[test]
    fn test_with_external_dependencies() {
        let dylib_graph = create_dylib_graph(&[
            ("A", &["external1"]),
            ("B", &["A", "external2"]),
            ("C", &["B"]),
        ]);

        let sorted_indices = generate_binary_load_order(&dylib_graph).unwrap();

        verify_sort(&dylib_graph, &sorted_indices);
    }

    #[test]
    fn test_cycle() {
        // A -> B -> C -> A
        let dylib_graph = create_dylib_graph(&[("A", &["C"]), ("B", &["A"]), ("C", &["B"])]);

        let result = generate_binary_load_order(&dylib_graph);

        assert!(
            result.is_err(),
            "Sort should detect cycle and return an error"
        );
    }

    #[test]
    fn test_self_dependency() {
        let dylib_graph = create_dylib_graph(&[("A", &["A"])]);

        let result = generate_binary_load_order(&dylib_graph);

        assert!(
            result.is_err(),
            "Sort should detect self-dependency as a cycle and return an error"
        );
    }

    /// Runs libstdc++'s static initializers, in batched (ticks-based) mode,
    /// on the real bundled libgcc and libstdc++: real guest code that
    /// exercises `svc` handling, stores through the memory callbacks, and
    /// CP15 barriers, all early enough to fail fast if any of them break.
    mod libstdcxx_static_initializers {
        use crate::cpu::{Cpu, CpuState};
        use crate::mach_o::{MachO, SectionType};
        use crate::mem::{ConstPtr, Mem, Ptr};
        use crate::objc::ObjC;

        #[test]
        fn trace_libstdcxx_static_initializers() {
            let mut mem = Mem::new();
            mem.set_null_segment_size(0x1000);

            let libgcc_bytes = std::fs::read("dylibs/libgcc_s.1.dylib").unwrap();
            let libgcc = MachO::load_from_bytes(
                &libgcc_bytes,
                &mut mem,
                "libgcc_s.1.dylib".to_string(),
                0x30000000,
            )
            .unwrap();

            let libstdcxx_bytes = std::fs::read("dylibs/libstdc++.6.0.9.dylib").unwrap();
            let libstdcxx = MachO::load_from_bytes(
                &libstdcxx_bytes,
                &mut mem,
                "libstdc++.6.0.9.dylib".to_string(),
                0x3748a000,
            )
            .unwrap();

            let mut objc = ObjC::new();
            let mut dyld = crate::dyld::Dyld::new();
            let bins = [libgcc, libstdcxx];
            dyld.do_initial_linking(&bins, &mut mem, &mut objc).unwrap();

            let mut cpu = Cpu::new(None);
            cpu.regs_mut()[Cpu::SP] = 0xFFFFF000;
            cpu.regs_mut()[Cpu::LR] = dyld.return_to_host_routine().addr_with_thumb_bit();

            let Some(section) = bins[1].get_section(SectionType::ModInitFuncPointers) else {
                panic!("no ModInitFuncPointers section");
            };
            assert!(section.size % 4 == 0);
            let base: crate::mem::ConstPtr<crate::abi::GuestFunction> =
                Ptr::from_bits(section.addr);
            let count = section.size / 4;
            eprintln!("{count} static initializer(s) in libstdc++");

            for idx in 0..count {
                let func = mem.read(base + idx);
                eprintln!("=== Calling static initializer {idx} at {:?}", func);
                cpu.branch(func);
                cpu.regs_mut()[Cpu::LR] = dyld.return_to_host_routine().addr_with_thumb_bit();

                for step in 0..500000u32 {
                    let pc = cpu.pc_with_thumb_bit();
                    let inst: u32 = mem.read(ConstPtr::from_bits(pc.addr_without_thumb_bit()));
                    let mut ticks = 100000u64;
                    let state = cpu.run_or_step(&mut mem, Some(&mut ticks));
                    match state {
                        CpuState::Normal => {}
                        CpuState::Svc(svc) => {
                            if svc == crate::dyld::Dyld::SVC_RETURN_TO_HOST {
                                eprintln!(
                                "initializer {idx} returned to host cleanly after {step} steps."
                            );
                                break;
                            }
                            eprintln!(
                                "initializer {idx} step {step}: SVC {svc:#x} at pc={:?}, skipping",
                                pc
                            );
                            break;
                        }
                        CpuState::Error(e) => {
                            eprintln!(
                                "initializer {idx} step {step}: CPU ERROR {:?} at pc={:?} \
                             inst={:#010x} sp={:#010x} lr={:#010x} r0={:#010x} r1={:#010x}",
                                e,
                                pc,
                                inst,
                                cpu.regs()[Cpu::SP],
                                cpu.regs()[Cpu::LR],
                                cpu.regs()[0],
                                cpu.regs()[1]
                            );
                            panic!("CPU error during trace: {:?}", e);
                        }
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod guest_symbol_trace_tests {
    use super::nearest_guest_symbol;
    use std::collections::HashMap;

    #[test]
    fn nearest_guest_symbol_handles_thumb_addresses_and_offsets() {
        let symbols = HashMap::from([
            ("_first".to_string(), 0x1001),
            ("_second".to_string(), 0x1100),
        ]);

        assert_eq!(nearest_guest_symbol(&symbols, 0x1109), Some(("_second", 8)));
    }

    #[test]
    fn nearest_guest_symbol_ignores_future_and_distant_symbols() {
        let symbols = HashMap::from([
            ("_future".to_string(), 0x3001),
            ("_distant".to_string(), 0x1000),
        ]);

        assert_eq!(nearest_guest_symbol(&symbols, 0x100), None);
        assert_eq!(
            nearest_guest_symbol(&symbols, 0x2000),
            Some(("_distant", 0x1000))
        );
    }
}
