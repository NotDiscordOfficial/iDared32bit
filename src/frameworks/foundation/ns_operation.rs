/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/.
 */
//! Foundation NSOperation and NSOperationQueue support.
//! Queued operations execute on emulated NSThreads, not synchronously on the
//! submitting thread. Operations are retained until they finish.

use super::ns_thread::detach_new_thread_inner;
use super::{NSInteger, NSUInteger};
use crate::objc::{
    autorelease, id, msg, msg_class, nil, objc_classes, release, retain, ClassExports, HostObject,
    NSZonePtr, SEL,
};
use crate::{Environment, ThreadId};
use std::collections::HashMap;
use std::time::Duration;

#[derive(Default)]
pub struct State {
    main_queue: Option<id>,
    current_queues: HashMap<ThreadId, id>,
}

struct OperationHostObject {
    cancelled: bool,
    executing: bool,
    finished: bool,
    dependencies: Vec<id>,
    /// Weak reference; the queue retains both itself and the operation.
    queue: id,
    priority: NSInteger,
    thread_priority: f64,
    name: id,
    work: OperationWork,
}
impl HostObject for OperationHostObject {}

#[derive(Clone, Copy)]
enum OperationWork {
    None,
    TargetSelector {
        target: id,
        selector: SEL,
        argument: id,
    },
    Invocation(id),
}

fn new_operation(env: &mut Environment, class: crate::objc::Class) -> id {
    env.objc.alloc_object(
        class,
        Box::new(OperationHostObject {
            cancelled: false,
            executing: false,
            finished: false,
            dependencies: Vec::new(),
            queue: nil,
            priority: 0,
            thread_priority: 0.5,
            name: nil,
            work: OperationWork::None,
        }),
        &mut env.mem,
    )
}

struct OperationQueueHostObject {
    operations: Vec<id>,
    running: Vec<id>,
    suspended: bool,
    maximum: NSInteger,
    name: id,
}
impl HostObject for OperationQueueHostObject {}

/// Launch only ready operations, honoring FIFO ordering and queue priority.
/// -1 uses a conservative default concurrency of four guest threads.
fn start_ready_operations(env: &mut Environment, queue: id) {
    loop {
        let (suspended, maximum, operations, running) = {
            let state = env.objc.borrow::<OperationQueueHostObject>(queue);
            (
                state.suspended,
                if state.maximum == -1 {
                    4
                } else {
                    state.maximum as usize
                },
                state.operations.clone(),
                state.running.clone(),
            )
        };
        if suspended || running.len() >= maximum {
            return;
        }
        let mut chosen: Option<(id, NSInteger)> = None;
        for operation in operations {
            if running.contains(&operation) {
                continue;
            }
            let ready: bool = msg![env; operation isReady];
            if !ready {
                continue;
            }
            let priority: NSInteger = msg![env; operation queuePriority];
            if chosen.is_none_or(|(_, best)| priority > best) {
                chosen = Some((operation, priority));
            }
        }
        let Some((operation, _)) = chosen else {
            return;
        };
        env.objc
            .borrow_mut::<OperationQueueHostObject>(queue)
            .running
            .push(operation);

        // NSThread retains the target until the guest thread has returned.
        // start takes no arguments; the NSThread target API passes a spare nil.
        let selector = env.objc.lookup_selector("start").unwrap();
        detach_new_thread_inner(env, selector, operation, nil, true);
    }
}

fn operation_finished(env: &mut Environment, operation: id, queue: id) {
    env.framework_state
        .foundation
        .ns_operation
        .current_queues
        .remove(&env.current_thread);
    {
        let state = env.objc.borrow_mut::<OperationQueueHostObject>(queue);
        state.running.retain(|&item| item != operation);
        state.operations.retain(|&item| item != operation);
    }
    env.objc.borrow_mut::<OperationHostObject>(operation).queue = nil;
    start_ready_operations(env, queue);
    release(env, operation);
    release(env, queue);
}

pub const CLASSES: ClassExports = objc_classes! {

(env, this, _cmd);

@implementation NSOperation: NSObject

+ (id)allocWithZone:(NSZonePtr)_zone {
    env.objc.alloc_object(this, Box::new(OperationHostObject {
        cancelled: false,
        executing: false,
        finished: false,
        dependencies: Vec::new(),
        queue: nil,
        priority: 0,
        thread_priority: 0.5,
        name: nil,
        work: OperationWork::None,
    }), &mut env.mem)
}
- (())main {
    // NSOperation is abstract: subclasses provide the work in -main.
}
- (())start {
    let (finished, executing, cancelled, queue) = {
        let op = env.objc.borrow::<OperationHostObject>(this);
        (op.finished, op.executing, op.cancelled, op.queue)
    };
    assert!(!finished && !executing, "NSOperation can only be started once");
    env.objc.borrow_mut::<OperationHostObject>(this).executing = true;
    if queue != nil {
        env.framework_state
            .foundation
            .ns_operation
            .current_queues
            .insert(env.current_thread, queue);
    }
    if !cancelled {
        () = msg![env; this main];
    }
    {
        let op = env.objc.borrow_mut::<OperationHostObject>(this);
        op.executing = false;
        op.finished = true;
    }
    if queue != nil {
        operation_finished(env, this, queue);
    }
}
- (())cancel {
    env.objc.borrow_mut::<OperationHostObject>(this).cancelled = true;
    let queue = env.objc.borrow::<OperationHostObject>(this).queue;
    if queue != nil {
        start_ready_operations(env, queue);
    }
}
- (bool)isCancelled {
    env.objc.borrow::<OperationHostObject>(this).cancelled
}
- (bool)isExecuting {
    env.objc.borrow::<OperationHostObject>(this).executing
}
- (bool)isFinished {
    env.objc.borrow::<OperationHostObject>(this).finished
}
- (bool)isConcurrent {
    false
}
- (bool)isAsynchronous {
    false
}
- (bool)isReady {
    let (cancelled, dependencies) = {
        let op = env.objc.borrow::<OperationHostObject>(this);
        (op.cancelled, op.dependencies.clone())
    };
    cancelled || dependencies.into_iter().all(|dep| {
        let finished: bool = msg![env; dep isFinished];
        finished
    })
}
- (())addDependency:(id)dependency {
    assert!(dependency != this);
    if !env.objc.borrow::<OperationHostObject>(this).dependencies.contains(&dependency) {
        retain(env, dependency);
        env.objc.borrow_mut::<OperationHostObject>(this).dependencies.push(dependency);
    }
}
- (())removeDependency:(id)dependency {
    let removed = {
        let op = env.objc.borrow_mut::<OperationHostObject>(this);
        if let Some(i) = op.dependencies.iter().position(|&dep| dep == dependency) {
            op.dependencies.remove(i);
            true
        } else {
            false
        }
    };
    if removed {
        release(env, dependency);
    }
    let queue = env.objc.borrow::<OperationHostObject>(this).queue;
    if queue != nil {
        start_ready_operations(env, queue);
    }
}
- (id)dependencies {
    let dependencies = env.objc.borrow::<OperationHostObject>(this).dependencies.clone();
    let array: id = msg_class![env; NSMutableArray new];
    for dependency in dependencies {
        () = msg![env; array addObject:dependency];
    }
    autorelease(env, array)
}
- (NSInteger)queuePriority {
    env.objc.borrow::<OperationHostObject>(this).priority
}
- (())setQueuePriority:(NSInteger)priority {
    env.objc.borrow_mut::<OperationHostObject>(this).priority = priority;
}
- (f64)threadPriority {
    env.objc.borrow::<OperationHostObject>(this).thread_priority
}
- (())setThreadPriority:(f64)priority {
    env.objc.borrow_mut::<OperationHostObject>(this).thread_priority = priority;
}
- (id)name {
    env.objc.borrow::<OperationHostObject>(this).name
}
- (())setName:(id)name {
    let copied: id = msg![env; name copy];
    let old = std::mem::replace(
        &mut env.objc.borrow_mut::<OperationHostObject>(this).name, copied
    );
    release(env, old);
}
- (())waitUntilFinished {
    while !env.objc.borrow::<OperationHostObject>(this).finished {
        env.sleep(Duration::from_millis(1));
    }
}
- (())dealloc {
    let op = env.objc.borrow_mut::<OperationHostObject>(this);
    let dependencies = std::mem::take(&mut op.dependencies);
    let name = std::mem::replace(&mut op.name, nil);
    let work = std::mem::replace(&mut op.work, OperationWork::None);
    for dependency in dependencies {
        release(env, dependency);
    }
    release(env, name);
    match work {
        OperationWork::None => {}
        OperationWork::TargetSelector { target, argument, .. } => {
            release(env, target);
            release(env, argument);
        }
        OperationWork::Invocation(invocation) => release(env, invocation),
    }
    env.objc.dealloc_object(this, &mut env.mem);
}
@end

@implementation NSInvocationOperation: NSOperation

+ (id)allocWithZone:(NSZonePtr)_zone {
    new_operation(env, this)
}

+ (id)invocationOperationWithTarget:(id)target selector:(SEL)selector object:(id)object {
    let operation: id = msg![env; this alloc];
    let operation: id = msg![env; operation initWithTarget:target
                                                  selector:selector
                                                    object:object];
    autorelease(env, operation)
}

- (id)initWithTarget:(id)target selector:(SEL)selector object:(id)object {
    retain(env, target);
    retain(env, object);
    env.objc.borrow_mut::<OperationHostObject>(this).work =
        OperationWork::TargetSelector { target, selector, argument: object };
    this
}

- (id)initWithInvocation:(id)invocation { // NSInvocation *
    retain(env, invocation);
    env.objc.borrow_mut::<OperationHostObject>(this).work =
        OperationWork::Invocation(invocation);
    this
}

- (())main {
    match env.objc.borrow::<OperationHostObject>(this).work {
        OperationWork::None => {}
        OperationWork::TargetSelector {
            target,
            selector,
            argument,
        } => {
            let _: () = crate::objc::msg_send(env, (target, selector, argument));
        }
        OperationWork::Invocation(invocation) => {
            () = msg![env; invocation invoke];
        }
    }
}

@end

@implementation NSOperationQueue: NSObject

+ (id)allocWithZone:(NSZonePtr)_zone {
    env.objc.alloc_object(this, Box::new(OperationQueueHostObject {
        operations: Vec::new(),
        running: Vec::new(),
        suspended: false,
        maximum: -1,
        name: nil,
    }), &mut env.mem)
}
+ (id)mainQueue {
    if let Some(queue) = env.framework_state.foundation.ns_operation.main_queue {
        queue
    } else {
        let queue: id = msg![env; this new];
        env.framework_state.foundation.ns_operation.main_queue = Some(queue);
        queue
    }
}
+ (id)currentQueue {
    env.framework_state.foundation.ns_operation
        .current_queues
        .get(&env.current_thread)
        .copied()
        .unwrap_or(nil)
}
- (())addOperation:(id)operation {
    assert!(operation != nil, "Cannot enqueue a nil operation");
    let finished: bool = msg![env; operation isFinished];
    let executing: bool = msg![env; operation isExecuting];
    assert!(!finished && !executing, "Cannot enqueue a started operation");
    assert!(
        env.objc.borrow::<OperationHostObject>(operation).queue == nil,
        "An operation cannot belong to two queues"
    );
    retain(env, operation);
    retain(env, this);
    env.objc.borrow_mut::<OperationHostObject>(operation).queue = this;
    env.objc.borrow_mut::<OperationQueueHostObject>(this).operations.push(operation);
    start_ready_operations(env, this);
}
- (())addOperations:(id)operations waitUntilFinished:(bool)wait {
    let count: NSUInteger = msg![env; operations count];
    let mut added = Vec::with_capacity(count as usize);
    for i in 0..count {
        let operation: id = msg![env; operations objectAtIndex:i];
        retain(env, operation);
        added.push(operation);
        () = msg![env; this addOperation:operation];
    }
    if wait {
        for &operation in &added {
            () = msg![env; operation waitUntilFinished];
        }
    }
    for operation in added {
        release(env, operation);
    }
}
- (())cancelAllOperations {
    let operations = env.objc.borrow::<OperationQueueHostObject>(this).operations.clone();
    for operation in operations {
        () = msg![env; operation cancel];
    }
}
- (())waitUntilAllOperationsAreFinished {
    while !env.objc.borrow::<OperationQueueHostObject>(this).operations.is_empty() {
        env.sleep(Duration::from_millis(1));
    }
}
- (NSUInteger)operationCount {
    env.objc.borrow::<OperationQueueHostObject>(this).operations.len() as NSUInteger
}
- (id)operations {
    let values = env.objc.borrow::<OperationQueueHostObject>(this).operations.clone();
    let array: id = msg_class![env; NSMutableArray new];
    for operation in values {
        () = msg![env; array addObject:operation];
    }
    autorelease(env, array)
}
- (bool)isSuspended {
    env.objc.borrow::<OperationQueueHostObject>(this).suspended
}
- (())setSuspended:(bool)value {
    env.objc.borrow_mut::<OperationQueueHostObject>(this).suspended = value;
    if !value {
        start_ready_operations(env, this);
    }
}
- (NSInteger)maxConcurrentOperationCount {
    env.objc.borrow::<OperationQueueHostObject>(this).maximum
}
- (())setMaxConcurrentOperationCount:(NSInteger)maximum {
    assert!(maximum == -1 || maximum > 0);
    env.objc.borrow_mut::<OperationQueueHostObject>(this).maximum = maximum;
    start_ready_operations(env, this);
}
- (id)name {
    env.objc.borrow::<OperationQueueHostObject>(this).name
}
- (())setName:(id)name {
    let copied: id = msg![env; name copy];
    let old = std::mem::replace(
        &mut env.objc.borrow_mut::<OperationQueueHostObject>(this).name, copied
    );
    release(env, old);
}
- (())dealloc {
    let queue = env.objc.borrow_mut::<OperationQueueHostObject>(this);
    let name = std::mem::replace(&mut queue.name, nil);
    let operations = std::mem::take(&mut queue.operations);
    for operation in operations {
        release(env, operation);
    }
    release(env, name);
    env.objc.dealloc_object(this, &mut env.mem);
}
@end

};
