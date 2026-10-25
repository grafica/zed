//! A measurement frame hook into each frame's own Metal command buffer (see [`set_frame_hook`]).

use std::sync::OnceLock;

/// What a [`FrameHook`] is called with: twice per drawn frame to encode (before and after
/// gpui's render passes), and once more when that frame's command buffer completes on the
/// GPU. The encode phases are called on the thread gpui draws on (the main thread on iOS);
/// work encoded into `command_buffer` there must not commit it.
pub enum FramePhase<'a> {
    /// The frame's command buffer just after it is created, before gpui's first render
    /// encoder (the scene's instance buffers are already written). Work encoded here is
    /// unordered against the draw and may overlap it, unless the hook sets `fence`.
    BeforeRender {
        /// Frames this hook has been offered, from 0; the frame's other phases repeat it.
        frame: u64,
        device: &'a metal::DeviceRef,
        command_buffer: &'a metal::CommandBufferRef,
        /// `None` on entry. Set it to order the hook's work ahead of the draw: gpui's first
        /// render encoder (the clear pass on the drawable) then waits for this fence before
        /// its vertex stage, so the hook's encoder must `update_fence` it.
        fence: &'a mut Option<metal::Fence>,
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
) -> Option<metal::Fence> {
    use gpui_apple::metal_renderer::FramePass;
    use std::sync::atomic::Ordering;
    let mut slot = FRAME_HOOK.lock();
    let hook = slot.as_mut()?;
    // gpui draws on one thread, so a frame's AfterRender follows its own BeforeRender.
    let frame = match pass {
        FramePass::BeforeRender => {
            let frame = FRAME_COUNT.fetch_add(1, Ordering::Relaxed);
            let mut fence = None;
            hook(FramePhase::BeforeRender {
                frame,
                device,
                command_buffer,
                fence: &mut fence,
            });
            return fence;
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
    None
}
