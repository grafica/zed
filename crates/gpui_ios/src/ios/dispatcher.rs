//! iOS task dispatcher using Grand Central Dispatch (GCD).
//!
//! iOS shares the same GCD infrastructure as macOS, so this implementation
//! is nearly identical to the macOS dispatcher.

#![allow(non_upper_case_globals)]
#![allow(non_camel_case_types)]
#![allow(non_snake_case)]

use gpui::{PlatformDispatcher, Priority, RunnableVariant};
use std::thread;

use objc2::runtime::Bool;
use objc2::{class, msg_send};
use std::{ffi::c_void, ptr::NonNull, time::Duration};

// GCD types - these are the same on iOS and macOS
type dispatch_queue_t = *mut std::ffi::c_void;
type dispatch_time_t = u64;

const DISPATCH_TIME_NOW: dispatch_time_t = 0;
const DISPATCH_QUEUE_PRIORITY_HIGH: i64 = 2;
const DISPATCH_QUEUE_PRIORITY_DEFAULT: i64 = 0;
const DISPATCH_QUEUE_PRIORITY_LOW: i64 = -2;

// SAFETY: These are C functions from libdispatch
unsafe extern "C" {
    static _dispatch_main_q: std::ffi::c_void;
    fn dispatch_async_f(
        queue: dispatch_queue_t,
        context: *mut c_void,
        work: Option<unsafe extern "C" fn(*mut c_void)>,
    );
    fn dispatch_after_f(
        when: dispatch_time_t,
        queue: dispatch_queue_t,
        context: *mut c_void,
        work: Option<unsafe extern "C" fn(*mut c_void)>,
    );
    fn dispatch_get_global_queue(identifier: i64, flags: u64) -> dispatch_queue_t;
    fn dispatch_time(when: dispatch_time_t, delta: i64) -> dispatch_time_t;
}

pub(crate) fn dispatch_get_main_queue() -> dispatch_queue_t {
    std::ptr::addr_of!(_dispatch_main_q) as *const _ as dispatch_queue_t
}

fn priority_to_gcd(priority: Priority) -> i64 {
    match priority {
        Priority::High => DISPATCH_QUEUE_PRIORITY_HIGH,
        Priority::Low => DISPATCH_QUEUE_PRIORITY_LOW,
        _ => DISPATCH_QUEUE_PRIORITY_DEFAULT,
    }
}

pub(crate) struct IosDispatcher;

impl PlatformDispatcher for IosDispatcher {
    fn is_main_thread(&self) -> bool {
        unsafe {
            let is_main: Bool = msg_send![class!(NSThread), isMainThread];
            is_main.as_bool()
        }
    }

    fn dispatch(&self, runnable: RunnableVariant, priority: Priority) {
        let context = runnable.into_raw().as_ptr() as *mut c_void;
        unsafe {
            dispatch_async_f(
                dispatch_get_global_queue(priority_to_gcd(priority), 0),
                context,
                Some(trampoline),
            );
        }
    }

    fn dispatch_on_main_thread(&self, runnable: RunnableVariant, _priority: Priority) {
        let context = runnable.into_raw().as_ptr() as *mut c_void;
        unsafe {
            dispatch_async_f(dispatch_get_main_queue(), context, Some(trampoline));
        }
    }

    fn dispatch_after(&self, duration: Duration, runnable: RunnableVariant) {
        let context = runnable.into_raw().as_ptr() as *mut c_void;
        unsafe {
            let queue = dispatch_get_global_queue(DISPATCH_QUEUE_PRIORITY_HIGH, 0);
            let when = dispatch_time(DISPATCH_TIME_NOW, duration.as_nanos() as i64);
            dispatch_after_f(when, queue, context, Some(trampoline));
        }
    }

    fn spawn_realtime(&self, f: Box<dyn FnOnce() + Send>) {
        // On iOS, we don't have direct realtime thread control like macOS.
        // Use a high-priority GCD queue as an approximation.
        thread::Builder::new()
            .name("gpui-ios-realtime".into())
            .spawn(move || {
                f();
            })
            .ok();
    }
}

unsafe extern "C" fn trampoline(runnable: *mut c_void) {
    let task = unsafe { RunnableVariant::from_raw(NonNull::new_unchecked(runnable as *mut ())) };
    task.run();
}

/// Run `f` on the main thread, via the same main queue `dispatch_on_main_thread` uses.
///
/// `prevent_idle_sleep`'s `ActivityGuard` can be dropped on any thread — it is a plain
/// RAII guard handed to arbitrary callers — while `setIdleTimerDisabled:` is UIKit and
/// so is main-thread only. This is the hop between the two. It takes a boxed closure
/// rather than a `RunnableVariant` because there is no task to schedule, just a flag to
/// set, and going through the executor would need a `ForegroundExecutor` the guard
/// cannot hold.
pub(crate) fn run_on_main_thread(f: impl FnOnce() + Send + 'static) {
    let boxed: Box<Box<dyn FnOnce() + Send>> = Box::new(Box::new(f));
    // SAFETY: the context is a `Box` we leak here and reclaim exactly once in
    // `call_boxed_closure`, which libdispatch invokes exactly once on the main queue.
    unsafe {
        dispatch_async_f(
            dispatch_get_main_queue(),
            Box::into_raw(boxed) as *mut c_void,
            Some(call_boxed_closure),
        );
    }
}

unsafe extern "C" fn call_boxed_closure(context: *mut c_void) {
    let f = unsafe { Box::from_raw(context as *mut Box<dyn FnOnce() + Send>) };
    f();
}
