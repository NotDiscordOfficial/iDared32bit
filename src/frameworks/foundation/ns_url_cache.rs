/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/.
 */
//! `NSURLCache` and `NSCachedURLResponse`.
//!
//! The cache keeps responses in memory only, keyed by URL. Nothing is
//! written to disk, so the disk usage is always 0.

use super::{ns_string, NSUInteger};
use crate::objc::{
    id, msg, nil, objc_classes, release, retain, ClassExports, HostObject, NSZonePtr,
};
use crate::Environment;
use std::collections::HashMap;

type NSURLCacheStoragePolicy = NSUInteger;
const NSURLCacheStorageAllowed: NSURLCacheStoragePolicy = 0;

#[derive(Default)]
pub struct State {
    /// `NSURLCache*`
    shared_cache: Option<id>,
}
impl State {
    fn get(env: &mut Environment) -> &mut State {
        &mut env.framework_state.foundation.ns_url_cache
    }
}

#[derive(Default)]
struct NSURLCacheHostObject {
    memory_capacity: NSUInteger,
    disk_capacity: NSUInteger,
    /// `NSString*`
    disk_path: id,
    /// URL strings to `NSCachedURLResponse*`s, retained.
    responses: HashMap<String, id>,
}
impl HostObject for NSURLCacheHostObject {}

#[derive(Default)]
struct NSCachedURLResponseHostObject {
    /// `NSURLResponse*`
    response: id,
    /// `NSData*`
    data: id,
    /// `NSDictionary*`
    user_info: id,
    storage_policy: NSURLCacheStoragePolicy,
}
impl HostObject for NSCachedURLResponseHostObject {}

/// The cache key for an `NSURLRequest*`, or [None] if it has no URL.
fn request_key(env: &mut Environment, request: id) -> Option<String> {
    let url: id = msg![env; request URL];
    if url == nil {
        return None;
    }
    let string: id = msg![env; url absoluteString];
    Some(ns_string::to_rust_string(env, string).into_owned())
}

pub const CLASSES: ClassExports = objc_classes! {

(env, this, _cmd);

@implementation NSURLCache: NSObject

+ (id)allocWithZone:(NSZonePtr)_zone {
    let host_object = Box::<NSURLCacheHostObject>::default();
    env.objc.alloc_object(this, host_object, &mut env.mem)
}

+ (id)sharedURLCache {
    if let Some(cache) = State::get(env).shared_cache {
        return cache;
    }
    // iPhone OS's default: 512 KB in memory, 10 MB on disk.
    let cache: id = msg![env; this alloc];
    let cache: id = msg![env; cache initWithMemoryCapacity:(512 * 1024u32)
                                              diskCapacity:(10 * 1024 * 1024u32)
                                                  diskPath:nil];
    State::get(env).shared_cache = Some(cache);
    cache
}
+ (())setSharedURLCache:(id)cache { // NSURLCache*
    retain(env, cache);
    if let Some(old) = State::get(env).shared_cache.replace(cache) {
        release(env, old);
    }
}

- (id)initWithMemoryCapacity:(NSUInteger)memory_capacity
                diskCapacity:(NSUInteger)disk_capacity
                    diskPath:(id)disk_path { // NSString*
    retain(env, disk_path);
    let host_obj = env.objc.borrow_mut::<NSURLCacheHostObject>(this);
    host_obj.memory_capacity = memory_capacity;
    host_obj.disk_capacity = disk_capacity;
    host_obj.disk_path = disk_path;
    this
}

- (())dealloc {
    let host_obj = std::mem::take(env.objc.borrow_mut::<NSURLCacheHostObject>(this));
    release(env, host_obj.disk_path);
    for response in host_obj.responses.into_values() {
        release(env, response);
    }
    env.objc.dealloc_object(this, &mut env.mem)
}

- (NSUInteger)memoryCapacity {
    env.objc.borrow::<NSURLCacheHostObject>(this).memory_capacity
}
- (())setMemoryCapacity:(NSUInteger)capacity {
    env.objc.borrow_mut::<NSURLCacheHostObject>(this).memory_capacity = capacity;
}
- (NSUInteger)diskCapacity {
    env.objc.borrow::<NSURLCacheHostObject>(this).disk_capacity
}
- (())setDiskCapacity:(NSUInteger)capacity {
    env.objc.borrow_mut::<NSURLCacheHostObject>(this).disk_capacity = capacity;
}

- (NSUInteger)currentMemoryUsage {
    let responses: Vec<id> = env
        .objc
        .borrow::<NSURLCacheHostObject>(this)
        .responses
        .values()
        .copied()
        .collect();
    let mut usage = 0;
    for response in responses {
        let data: id = msg![env; response data];
        let length: NSUInteger = msg![env; data length];
        usage += length;
    }
    usage
}
- (NSUInteger)currentDiskUsage {
    0
}

- (id)cachedResponseForRequest:(id)request { // NSURLRequest*
    let Some(key) = request_key(env, request) else {
        return nil;
    };
    let host_obj = env.objc.borrow::<NSURLCacheHostObject>(this);
    host_obj.responses.get(&key).copied().unwrap_or(nil)
}

- (())storeCachedResponse:(id)response // NSCachedURLResponse*
               forRequest:(id)request { // NSURLRequest*
    let Some(key) = request_key(env, request) else {
        return;
    };
    retain(env, response);
    let host_obj = env.objc.borrow_mut::<NSURLCacheHostObject>(this);
    if let Some(old) = host_obj.responses.insert(key, response) {
        release(env, old);
    }
}

- (())removeCachedResponseForRequest:(id)request { // NSURLRequest*
    let Some(key) = request_key(env, request) else {
        return;
    };
    let host_obj = env.objc.borrow_mut::<NSURLCacheHostObject>(this);
    if let Some(old) = host_obj.responses.remove(&key) {
        release(env, old);
    }
}

- (())removeAllCachedResponses {
    let host_obj = env.objc.borrow_mut::<NSURLCacheHostObject>(this);
    let responses = std::mem::take(&mut host_obj.responses);
    for response in responses.into_values() {
        release(env, response);
    }
}

@end

@implementation NSCachedURLResponse: NSObject

+ (id)allocWithZone:(NSZonePtr)_zone {
    let host_object = Box::<NSCachedURLResponseHostObject>::default();
    env.objc.alloc_object(this, host_object, &mut env.mem)
}

- (id)initWithResponse:(id)response // NSURLResponse*
                  data:(id)data { // NSData*
    msg![env; this initWithResponse:response
                               data:data
                           userInfo:nil
                      storagePolicy:NSURLCacheStorageAllowed]
}

- (id)initWithResponse:(id)response // NSURLResponse*
                  data:(id)data // NSData*
              userInfo:(id)user_info // NSDictionary*
         storagePolicy:(NSURLCacheStoragePolicy)storage_policy {
    retain(env, response);
    retain(env, data);
    retain(env, user_info);
    let host_obj = env.objc.borrow_mut::<NSCachedURLResponseHostObject>(this);
    host_obj.response = response;
    host_obj.data = data;
    host_obj.user_info = user_info;
    host_obj.storage_policy = storage_policy;
    this
}

- (())dealloc {
    let host_obj = std::mem::take(env.objc.borrow_mut::<NSCachedURLResponseHostObject>(this));
    release(env, host_obj.response);
    release(env, host_obj.data);
    release(env, host_obj.user_info);
    env.objc.dealloc_object(this, &mut env.mem)
}

- (id)response {
    env.objc.borrow::<NSCachedURLResponseHostObject>(this).response
}
- (id)data {
    env.objc.borrow::<NSCachedURLResponseHostObject>(this).data
}
- (id)userInfo {
    env.objc.borrow::<NSCachedURLResponseHostObject>(this).user_info
}
- (NSURLCacheStoragePolicy)storagePolicy {
    env.objc.borrow::<NSCachedURLResponseHostObject>(this).storage_policy
}

@end

};
