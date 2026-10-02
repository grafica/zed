//! Per-draw counters and timers, for finding where a window's draw goes on a device.
//!
//! Off unless the `GPUI_FRAME_STATS` environment variable is set, at the cost of one
//! relaxed atomic add per counted event and one `Instant::now` pair per timed span. On,
//! every 120 draws one line goes to stderr:
//!
//! ```text
//! gpui-stats,draws=120,nodes/draw=14.0,computes/draw=1.0,measures/draw=13.0,quads/draw=133.7,...,us_draw/draw=538.3,...
//! ```
//!
//! Counts per draw: layout nodes requested, layouts computed, measure callbacks run,
//! primitives inserted by kind, bounds-tree inserts and the slow searches among them,
//! line-layout cache hits and misses, Metal batches and command encoders. Microseconds
//! per draw: the whole draw, its render (`draw_roots`), the prepaint and paint phases,
//! layout computation, layout-node creation, measure callbacks, `shape_text`,
//! `resolve_font`, `line_wrapper`, `paint_glyph`.
//!
//! Written for the DDCM app's live camera screen on an iPhone 16 Pro (2026-10-02), where
//! it found a marker overlay paying one bounds-tree search per cell on every draw.
use std::sync::atomic::{AtomicU64, Ordering::Relaxed};

/// Draws, the divisor of every other count.
pub const DRAWS: usize = 0;
/// Layout nodes requested.
pub const NODES: usize = 1;
/// Layouts computed.
pub const COMPUTES: usize = 2;
/// Measure callbacks run by layout.
pub const MEASURES: usize = 3;
/// Quads inserted into the scene.
pub const QUADS: usize = 4;
/// Surfaces inserted into the scene.
pub const SURFACES: usize = 5;
/// Monochrome and polychrome sprites inserted into the scene.
pub const SPRITES: usize = 6;
/// Paths inserted into the scene.
pub const PATHS: usize = 7;
/// Every other primitive inserted into the scene.
pub const OTHER_PRIMS: usize = 8;
/// Bounds-tree inserts (one per primitive outside a layer).
pub const BT_INSERTS: usize = 9;
/// Bounds-tree inserts that searched the tree.
pub const BT_SLOW: usize = 10;
/// Line-layout cache hits.
pub const TEXT_HIT: usize = 11;
/// Line-layout cache misses (a shape).
pub const TEXT_MISS: usize = 12;
/// Primitive batches drawn by the renderer.
pub const BATCHES: usize = 13;
/// Metal command encoders created by the renderer.
pub const ENCODERS: usize = 14;
/// Microseconds in `Window::draw`.
pub const T_DRAW: usize = 15;
/// Microseconds in `draw_roots` (layout, prepaint, paint).
pub const T_RENDER: usize = 16;
/// Microseconds in the prepaint phase, layout included.
pub const T_PREPAINT: usize = 17;
/// Microseconds in the paint phase.
pub const T_PAINT: usize = 18;
/// Microseconds computing layout.
pub const T_LAYOUT: usize = 19;
/// Microseconds creating layout nodes.
pub const T_REQUEST: usize = 20;
/// Microseconds in measure callbacks.
pub const T_MEASURE: usize = 21;
/// Microseconds in `shape_text`.
pub const T_SHAPE: usize = 22;
/// Microseconds in `resolve_font`.
pub const T_FONT: usize = 23;
/// Microseconds in `line_wrapper`.
pub const T_WRAPPER: usize = 24;
/// Microseconds in `paint_glyph`.
pub const T_GLYPH: usize = 25;
const N: usize = 26;
const NAMES: [&str; N] = [
    "draws",
    "nodes",
    "computes",
    "measures",
    "quads",
    "surfaces",
    "sprites",
    "paths",
    "other_prims",
    "bt_inserts",
    "bt_slow",
    "text_hit",
    "text_miss",
    "batches",
    "encoders",
    "us_draw",
    "us_render",
    "us_prepaint",
    "us_paint",
    "us_layout",
    "us_request",
    "us_measure",
    "us_shape",
    "us_font",
    "us_wrapper",
    "us_glyph",
];
const EVERY: u64 = 120;
static C: [AtomicU64; N] = [const { AtomicU64::new(0) }; N];

/// Whether `GPUI_FRAME_STATS` is set; read once.
pub fn on() -> bool {
    static R: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *R.get_or_init(|| std::env::var_os("GPUI_FRAME_STATS").is_some())
}

/// Count one event of kind `i` (one of the constants above).
#[inline]
pub fn bump(i: usize) {
    C[i].fetch_add(1, Relaxed);
}

/// Count one scene primitive by its variant name.
pub fn bump_primitive(kind: &str) {
    bump(match kind {
        "Quad" => QUADS,
        "Surface" => SURFACES,
        "MonochromeSprite" | "PolychromeSprite" => SPRITES,
        "Path" => PATHS,
        _ => OTHER_PRIMS,
    });
}

/// Adds the microseconds since `start` to timer `i` when dropped, when stats are on.
pub struct Timer(Option<(usize, std::time::Instant)>);

impl Timer {
    /// Start timing span `i` (one of the `T_` constants); a no-op when stats are off.
    #[inline]
    pub fn start(i: usize) -> Timer {
        Timer(if on() {
            Some((i, std::time::Instant::now()))
        } else {
            None
        })
    }
}

impl Drop for Timer {
    fn drop(&mut self) {
        if let Some((i, t)) = self.0 {
            C[i].fetch_add(t.elapsed().as_micros() as u64, Relaxed);
        }
    }
}

/// Called once per `Window::draw`: prints and resets every `EVERY` draws.
pub fn frame_done() {
    if !on() {
        return;
    }
    let d = C[DRAWS].fetch_add(1, Relaxed) + 1;
    if d % EVERY != 0 {
        return;
    }
    let mut line = format!("gpui-stats,draws={d}");
    for i in 1..N {
        let v = C[i].swap(0, Relaxed);
        line.push_str(&format!(
            ",{}/draw={:.1}",
            NAMES[i],
            v as f64 / EVERY as f64
        ));
    }
    C[DRAWS].store(0, Relaxed);
    eprintln!("{line}");
}
