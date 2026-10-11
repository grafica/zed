//! Native entry points used by an iOS application delegate.

use gpui::{App, AppLifecyclePhase, Application, ApplicationHandle};
use std::{cell::UnsafeCell, ffi::c_void, rc::Rc, sync::OnceLock};

type AppCallback = Box<dyn FnOnce(&mut App)>;

struct IosCallbacks {
    finish_launching: Option<Box<dyn FnOnce()>>,
    app: Option<AppCallback>,
    quit: Option<Box<dyn FnMut()>>,
    open_urls: Option<Box<dyn FnMut(Vec<String>)>>,
    app_lifecycle: Option<Box<dyn FnMut(AppLifecyclePhase)>>,
    memory_warning: Option<Box<dyn FnMut()>>,
}

struct IosAppState {
    callbacks: UnsafeCell<IosCallbacks>,
    application: UnsafeCell<Option<ApplicationHandle>>,
    window_scene: UnsafeCell<*mut c_void>,
}

unsafe impl Send for IosAppState {}
unsafe impl Sync for IosAppState {}

pub(crate) struct WindowListWrapper(pub(crate) UnsafeCell<Vec<*const super::window::IosWindow>>);

unsafe impl Send for WindowListWrapper {}
unsafe impl Sync for WindowListWrapper {}

static IOS_APP_STATE: OnceLock<IosAppState> = OnceLock::new();
pub(crate) static IOS_WINDOW_LIST: OnceLock<WindowListWrapper> = OnceLock::new();

fn app_state() -> &'static IosAppState {
    IOS_APP_STATE.get_or_init(|| IosAppState {
        callbacks: UnsafeCell::new(IosCallbacks {
            finish_launching: None,
            app: None,
            quit: None,
            open_urls: None,
            app_lifecycle: None,
            memory_warning: None,
        }),
        application: UnsafeCell::new(None),
        window_scene: UnsafeCell::new(std::ptr::null_mut()),
    })
}

fn window_list() -> &'static WindowListWrapper {
    IOS_WINDOW_LIST.get_or_init(|| WindowListWrapper(UnsafeCell::new(Vec::new())))
}

fn ios_window(window: *mut c_void) -> Option<&'static super::window::IosWindow> {
    unsafe { (window as *const super::window::IosWindow).as_ref() }
}

#[unsafe(no_mangle)]
pub extern "C" fn gpui_ios_initialize() -> *mut c_void {
    app_state();
    window_list();
    std::ptr::dangling_mut::<c_void>()
}

#[unsafe(no_mangle)]
pub extern "C" fn gpui_ios_set_window_scene(scene: *mut c_void) {
    unsafe {
        *app_state().window_scene.get() = scene;
    }
}

pub(crate) fn window_scene() -> *mut objc2::runtime::AnyObject {
    unsafe { (*app_state().window_scene.get()).cast() }
}

pub(crate) fn register_window(window: *const super::window::IosWindow) {
    unsafe {
        (*window_list().0.get()).push(window);
    }
}

pub(crate) fn unregister_window(window: *const super::window::IosWindow) {
    unsafe {
        (*window_list().0.get()).retain(|registered_window| *registered_window != window);
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn gpui_ios_get_window() -> *mut c_void {
    unsafe {
        (*window_list().0.get())
            .last()
            .copied()
            .map_or(std::ptr::null_mut(), |window| window.cast_mut().cast())
    }
}

pub(crate) fn set_finish_launching_callback(callback: Box<dyn FnOnce()>) {
    unsafe {
        (*app_state().callbacks.get()).finish_launching = Some(callback);
    }
}

pub(crate) fn set_quit_callback(callback: Box<dyn FnMut()>) {
    unsafe {
        (*app_state().callbacks.get()).quit = Some(callback);
    }
}

pub(crate) fn set_open_urls_callback(callback: Box<dyn FnMut(Vec<String>)>) {
    unsafe {
        (*app_state().callbacks.get()).open_urls = Some(callback);
    }
}

pub(crate) fn set_app_lifecycle_callback(callback: Box<dyn FnMut(AppLifecyclePhase)>) {
    unsafe {
        (*app_state().callbacks.get()).app_lifecycle = Some(callback);
    }
}

pub(crate) fn set_memory_warning_callback(callback: Box<dyn FnMut()>) {
    unsafe {
        (*app_state().callbacks.get()).memory_warning = Some(callback);
    }
}

pub fn set_app_callback(callback: AppCallback) {
    unsafe {
        (*app_state().callbacks.get()).app = Some(callback);
    }
}

fn take_app_callback() -> Option<AppCallback> {
    unsafe { (*app_state().callbacks.get()).app.take() }
}

#[unsafe(no_mangle)]
pub extern "C" fn gpui_ios_did_finish_launching(_app: *mut c_void) {
    let callback = unsafe { (*app_state().callbacks.get()).finish_launching.take() };
    if let Some(callback) = callback {
        callback();
    } else {
        log::error!("GPUI iOS: Application launch callback was not registered");
    }
}

fn notify_windows_active(is_active: bool) {
    unsafe {
        for &window in &*window_list().0.get() {
            if let Some(window) = window.as_ref() {
                window.notify_active_status_change(is_active);
            }
        }
    }
}

fn notify_app_lifecycle(phase: AppLifecyclePhase) {
    let callback = unsafe { (*app_state().callbacks.get()).app_lifecycle.take() };
    if let Some(mut callback) = callback {
        callback(phase);
        let callbacks = unsafe { &mut *app_state().callbacks.get() };
        if callbacks.app_lifecycle.is_none() {
            callbacks.app_lifecycle = Some(callback);
        }
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn gpui_ios_will_enter_foreground(_app: *mut c_void) {
    notify_windows_active(true);
    notify_app_lifecycle(AppLifecyclePhase::Foreground);
}

#[unsafe(no_mangle)]
pub extern "C" fn gpui_ios_did_become_active(_app: *mut c_void) {
    notify_windows_active(true);
    notify_app_lifecycle(AppLifecyclePhase::Active);
}

#[unsafe(no_mangle)]
pub extern "C" fn gpui_ios_will_resign_active(_app: *mut c_void) {
    notify_windows_active(false);
    notify_app_lifecycle(AppLifecyclePhase::Inactive);
}

#[unsafe(no_mangle)]
pub extern "C" fn gpui_ios_did_enter_background(_app: *mut c_void) {
    notify_windows_active(false);
    notify_app_lifecycle(AppLifecyclePhase::Background);
}

#[unsafe(no_mangle)]
pub extern "C" fn gpui_ios_did_receive_memory_warning(_app: *mut c_void) {
    let callback = unsafe { (*app_state().callbacks.get()).memory_warning.take() };
    if let Some(mut callback) = callback {
        callback();
        let callbacks = unsafe { &mut *app_state().callbacks.get() };
        if callbacks.memory_warning.is_none() {
            callbacks.memory_warning = Some(callback);
        }
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn gpui_ios_will_terminate(_app: *mut c_void) {
    let callback = unsafe { (*app_state().callbacks.get()).quit.take() };
    if let Some(mut callback) = callback {
        callback();
        let callbacks = unsafe { &mut *app_state().callbacks.get() };
        if callbacks.quit.is_none() {
            callbacks.quit = Some(callback);
        }
    }
    unsafe {
        (*app_state().application.get()).take();
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn gpui_ios_handle_touch(
    window: *mut c_void,
    touch: *mut c_void,
    event: *mut c_void,
) {
    let Some(window) = ios_window(window) else {
        return;
    };
    if touch.is_null() {
        return;
    }

    window.handle_touch(
        touch.cast::<objc2::runtime::AnyObject>(),
        event.cast::<objc2::runtime::AnyObject>(),
    );
}

#[unsafe(no_mangle)]
pub extern "C" fn gpui_ios_request_frame(window: *mut c_void) {
    let Some(window) = ios_window(window) else {
        return;
    };
    window.request_frame();
}

#[unsafe(no_mangle)]
pub extern "C" fn gpui_ios_show_keyboard(window: *mut c_void) {
    if let Some(window) = ios_window(window) {
        window.show_keyboard();
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn gpui_ios_hide_keyboard(window: *mut c_void) {
    if let Some(window) = ios_window(window) {
        window.hide_keyboard();
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn gpui_ios_handle_text_input(window: *mut c_void, text: *mut c_void) {
    let Some(window) = ios_window(window) else {
        return;
    };
    if !text.is_null() {
        window.handle_text_input(text.cast::<objc2::runtime::AnyObject>());
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn gpui_ios_handle_key_event(
    window: *mut c_void,
    key_code: u32,
    modifiers: u32,
    is_key_down: bool,
) {
    if let Some(window) = ios_window(window) {
        window.handle_key_event(key_code, modifiers, is_key_down);
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn gpui_ios_handle_open_url(url: *mut c_void) {
    if url.is_null() {
        return;
    }

    let url = unsafe {
        let utf8: *const std::ffi::c_char =
            objc2::msg_send![url.cast::<objc2::runtime::AnyObject>(), UTF8String];
        if utf8.is_null() {
            return;
        }
        std::ffi::CStr::from_ptr(utf8)
            .to_string_lossy()
            .into_owned()
    };

    let callback = unsafe { (*app_state().callbacks.get()).open_urls.take() };
    if let Some(mut callback) = callback {
        callback(vec![url]);
        let callbacks = unsafe { &mut *app_state().callbacks.get() };
        if callbacks.open_urls.is_none() {
            callbacks.open_urls = Some(callback);
        }
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn gpui_ios_run() {
    run_app();
}

pub fn run_app() {
    run_app_with_assets(());
}

pub fn run_app_with_assets(asset_source: impl gpui::AssetSource) {
    gpui_ios_initialize();

    let platform = Rc::new(super::IosPlatform::new());
    let application = Application::with_platform(platform)
        .with_assets(asset_source)
        .run_embedded(|cx: &mut App| {
            if let Some(callback) = take_app_callback() {
                callback(cx);
            } else {
                log::error!("GPUI iOS: App callback was not registered");
            }
        });
    unsafe {
        *app_state().application.get() = Some(application);
    }

    gpui_ios_did_finish_launching(std::ptr::null_mut());
}

/// What a [`FrameHook`] is called with: twice per drawn frame to encode (before and after
/// gpui's render passes), and once more when that frame's command buffer completes on the
/// GPU. The encode phases are called on the thread gpui draws on (the main thread on iOS);
/// work encoded into `command_buffer` there must not commit it.
pub enum FramePhase<'a> {
    /// The frame's command buffer just after it is created, before gpui's first render
    /// encoder (the scene's instance buffers are already written): work encoded here runs
    /// on the GPU ahead of the draw.
    BeforeRender {
        /// Frames this hook has been offered, from 0; the frame's other phases repeat it.
        frame: u64,
        device: &'a metal::DeviceRef,
        command_buffer: &'a metal::CommandBufferRef,
    },
    /// The same command buffer after gpui has encoded every render pass, before it is
    /// presented and committed: work encoded here runs behind the draw.
    AfterRender {
        frame: u64,
        device: &'a metal::DeviceRef,
        command_buffer: &'a metal::CommandBufferRef,
    },
    /// The same frame's command buffer completed. Called on a Metal completion thread.
    Completed(FrameTiming),
}

/// One frame command buffer's timing. Every time is seconds on the host clock
/// (`mach_absolute_time`), the clock Metal's `GPUStartTime` / `GPUEndTime` use.
#[derive(Clone, Copy, Debug)]
pub struct FrameTiming {
    pub frame: u64,
    /// Sampled after the hook's [`FramePhase::AfterRender`] returns, immediately before gpui
    /// presents the drawable and commits the buffer.
    pub commit_host_s: f64,
    /// The command buffer's `GPUStartTime`: gpui's render passes and anything the hook
    /// encoded are one span. Timing the hook's encoder apart from the draw needs a counter
    /// sample buffer (`MTLCounterSampleBuffer`, sampled at the encoder's stage boundaries),
    /// which this measurement hook does not set up.
    pub gpu_start_s: f64,
    /// The command buffer's `GPUEndTime`.
    pub gpu_end_s: f64,
    /// Sampled when the completed handler ran.
    pub completed_host_s: f64,
    /// `MTLCommandBufferStatus` at completion (4 = completed, 5 = error).
    pub status: u64,
}

/// A per-frame callback; see [`set_frame_hook`].
pub type FrameHook = Box<dyn FnMut(FramePhase<'_>) + Send>;

static FRAME_HOOK: parking_lot::Mutex<Option<FrameHook>> = parking_lot::Mutex::new(None);
static FRAME_COUNT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Registers (`Some`) or removes (`None`) the frame hook, called for every frame gpui draws
/// in any window: [`FramePhase::BeforeRender`] and [`FramePhase::AfterRender`] with the
/// frame's own command buffer before it is committed, then [`FramePhase::Completed`] with
/// its GPU span. With no hook registered a frame pays two uncontended locks.
///
/// This is a MEASUREMENT hook: it lets an app put a compute pass into gpui's command buffer
/// (paying no second commit and no second GPU wake) and see how long gpui's own draw
/// occupies the GPU. It is one process-wide slot, not per window, and a hook lives until it
/// is replaced. A product-grade version would be per window (registered on the window or
/// its renderer and dropped with it), would give the hook the drawable's size and the
/// frame's presentation time, and would not hold a global lock across the app's encode.
/// The hook must not call [`set_frame_hook`] itself: that deadlocks.
pub fn set_frame_hook(hook: Option<FrameHook>) {
    *FRAME_HOOK.lock() = hook;
}

#[repr(C)]
struct MachTimebaseInfo {
    numer: u32,
    denom: u32,
}

unsafe extern "C" {
    fn mach_absolute_time() -> u64;
    fn mach_timebase_info(info: *mut MachTimebaseInfo) -> i32;
}

/// The host clock in seconds (`mach_absolute_time`, the clock of `GPUStartTime`).
pub fn host_seconds() -> f64 {
    static SCALE: OnceLock<f64> = OnceLock::new();
    let scale = *SCALE.get_or_init(|| {
        let mut info = MachTimebaseInfo { numer: 0, denom: 0 };
        // SAFETY: writes the two fields of a valid `mach_timebase_info_data_t`.
        let status = unsafe { mach_timebase_info(&mut info) };
        if status == 0 && info.denom != 0 {
            info.numer as f64 / info.denom as f64 * 1e-9
        } else {
            1e-9
        }
    });
    // SAFETY: no arguments, no preconditions.
    (unsafe { mach_absolute_time() }) as f64 * scale
}

/// Runs the registered frame hook on one frame's command buffer; the window's draw calls it
/// through [`gpui_apple::metal_renderer::MetalRenderer::draw_with_frame_hook`].
pub(crate) fn run_frame_hook(
    pass: gpui_apple::metal_renderer::FramePass,
    device: &metal::DeviceRef,
    command_buffer: &metal::CommandBufferRef,
) {
    use gpui_apple::metal_renderer::FramePass;
    use std::sync::atomic::Ordering;
    let mut slot = FRAME_HOOK.lock();
    let Some(hook) = slot.as_mut() else {
        return;
    };
    // gpui draws on one thread, so a frame's AfterRender follows its own BeforeRender.
    let frame = match pass {
        FramePass::BeforeRender => {
            let frame = FRAME_COUNT.fetch_add(1, Ordering::Relaxed);
            hook(FramePhase::BeforeRender {
                frame,
                device,
                command_buffer,
            });
            return;
        }
        FramePass::AfterRender => FRAME_COUNT.load(Ordering::Relaxed).saturating_sub(1),
    };
    hook(FramePhase::AfterRender {
        frame,
        device,
        command_buffer,
    });
    drop(slot);
    let commit_host_s = host_seconds();
    let block = block2::RcBlock::new(move |buffer: *mut objc2::runtime::AnyObject| {
        let completed_host_s = host_seconds();
        if buffer.is_null() {
            return;
        }
        // SAFETY: `buffer` is the completed `id<MTLCommandBuffer>` Metal hands its handler;
        // these are read-only properties.
        let (gpu_start_s, gpu_end_s, status): (f64, f64, u64) = unsafe {
            (
                objc2::msg_send![buffer, GPUStartTime],
                objc2::msg_send![buffer, GPUEndTime],
                objc2::msg_send![buffer, status],
            )
        };
        if let Some(hook) = FRAME_HOOK.lock().as_mut() {
            hook(FramePhase::Completed(FrameTiming {
                frame,
                commit_host_s,
                gpu_start_s,
                gpu_end_s,
                completed_host_s,
                status,
            }));
        }
    });
    let buffer = command_buffer as *const metal::CommandBufferRef as *mut objc2::runtime::AnyObject;
    // SAFETY: a `CommandBufferRef` is the `id<MTLCommandBuffer>` itself, and it is not yet
    // committed, so a completed handler may still be added. Metal copies the block.
    unsafe {
        let _: () = objc2::msg_send![buffer, addCompletedHandler: &*block];
    }
}
