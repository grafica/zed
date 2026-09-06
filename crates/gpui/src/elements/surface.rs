use crate::{
    App, Bounds, DefiniteLength, Element, ElementId, GlobalElementId, InspectorElementId,
    IntoElement, LayoutId, Length, ObjectFit, Pixels, Style, StyleRefinement, Styled, Window, px,
};
#[cfg(any(target_os = "macos", target_os = "ios"))]
use core_video::pixel_buffer::CVPixelBuffer;
use refineable::Refineable;

/// A source of a surface's content.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SurfaceSource {
    /// A macOS image buffer from CoreVideo
    #[cfg(any(target_os = "macos", target_os = "ios"))]
    Surface(CVPixelBuffer),
}

#[cfg(any(target_os = "macos", target_os = "ios"))]
impl From<CVPixelBuffer> for SurfaceSource {
    fn from(value: CVPixelBuffer) -> Self {
        SurfaceSource::Surface(value)
    }
}

/// A surface element.
pub struct Surface {
    source: SurfaceSource,
    object_fit: ObjectFit,
    style: StyleRefinement,
}

/// Create a new surface element.
#[cfg(any(target_os = "macos", target_os = "ios"))]
pub fn surface(source: impl Into<SurfaceSource>) -> Surface {
    Surface {
        source: source.into(),
        object_fit: ObjectFit::Contain,
        style: Default::default(),
    }
}

impl Surface {
    /// Set the object fit for the image.
    pub fn object_fit(mut self, object_fit: ObjectFit) -> Self {
        self.object_fit = object_fit;
        self
    }
}

impl Element for Surface {
    type RequestLayoutState = ();
    type PrepaintState = ();

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _global_id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        let mut style = Style::default();
        style.refine(&self.style);

        // Give the surface the intrinsic size of its buffer, the way `img` does with the
        // intrinsic size of its image. Without this a surface laid out with only
        // `max_w`/`max_h` -- or with nothing at all -- resolves `Length::Auto` against no
        // content, lays out at 0x0, and paints NOTHING, silently. Every timer around it
        // still reports a plausible number, because what they measure is the handoff of
        // the buffer rather than the draw.
        #[cfg(any(target_os = "macos", target_os = "ios"))]
        {
            let SurfaceSource::Surface(buffer) = &self.source;
            let size = crate::size(
                px(buffer.get_width() as f32),
                px(buffer.get_height() as f32),
            );
            if size.width > px(0.) && size.height > px(0.) {
                if style.aspect_ratio.is_none() {
                    style.aspect_ratio = Some(size.width / size.height);
                }
                if let Length::Auto = style.size.width {
                    style.size.width = match style.size.height {
                        Length::Definite(DefiniteLength::Absolute(abs)) => {
                            let height = abs.to_pixels(window.rem_size());
                            Length::Definite(px(size.width.0 * height.0 / size.height.0).into())
                        }
                        _ => Length::Definite(size.width.into()),
                    };
                }
                if let Length::Auto = style.size.height {
                    style.size.height = match style.size.width {
                        Length::Definite(DefiniteLength::Absolute(abs)) => {
                            let width = abs.to_pixels(window.rem_size());
                            Length::Definite(px(size.height.0 * width.0 / size.width.0).into())
                        }
                        _ => Length::Definite(size.height.into()),
                    };
                }
            }
        }

        let layout_id = window.request_layout(style, [], cx);
        (layout_id, ())
    }

    fn prepaint(
        &mut self,
        _global_id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        _bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        _window: &mut Window,
        _cx: &mut App,
    ) -> Self::PrepaintState {
    }

    fn paint(
        &mut self,
        _global_id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        #[cfg_attr(not(target_os = "macos"), allow(unused_variables))] bounds: Bounds<Pixels>,
        _: &mut Self::RequestLayoutState,
        _: &mut Self::PrepaintState,
        #[cfg_attr(not(target_os = "macos"), allow(unused_variables))] window: &mut Window,
        _: &mut App,
    ) {
        match &self.source {
            #[cfg(any(target_os = "macos", target_os = "ios"))]
            SurfaceSource::Surface(surface) => {
                let size = crate::size(surface.get_width().into(), surface.get_height().into());
                let new_bounds = self.object_fit.get_bounds(bounds, size);
                // TODO: Add support for corner_radii
                window.paint_surface(new_bounds, surface.clone());
            }
            #[allow(unreachable_patterns)]
            _ => {}
        }
    }
}

impl IntoElement for Surface {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Styled for Surface {
    fn style(&mut self) -> &mut StyleRefinement {
        &mut self.style
    }
}
