use i_slint_core::window::WindowAdapterInternal;
use i_slint_core::{items::MouseCursor, partial_renderer::DirtyRegion, platform::WindowAdapter};
use i_slint_renderer_skia::{
    skia_safe::{self, ColorType},
    software_surface::RenderBuffer,
};
use i_slint_renderer_skia::{SkiaRenderer, SkiaSharedContext, software_surface::SoftwareSurface};
use slint::{PhysicalSize, Window};
use smithay_client_toolkit::{
    reexports::client::protocol::wl_shm,
    shm::slot::{Buffer, Slot, SlotPool},
};
use std::{
    cell::{Cell, RefCell},
    fmt::Debug,
    rc::{Rc, Weak},
};

pub struct SkiaSoftwareBufferReal {
    pub primary_slot: RefCell<Slot>,
    pub pool: Rc<RefCell<SlotPool>>,
    pub last_dirty_region: RefCell<Option<DirtyRegion>>,
    pub is_buffer_fresh: Cell<bool>,
}

impl SkiaSoftwareBufferReal {
    fn refresh_buffer(&self, width: i32, height: i32) -> Buffer {
        let stride = width * 4;
        let (buffer, _raw_canvas) = self
            .pool
            .borrow_mut()
            .create_buffer(width, height, stride, wl_shm::Format::Argb8888)
            .unwrap();
        *self.primary_slot.borrow_mut() = buffer.slot();
        self.is_buffer_fresh.set(true);
        buffer
    }
}

impl RenderBuffer for SkiaSoftwareBufferReal {
    fn with_buffer(
        &self,
        _: &Window,
        size: PhysicalSize,
        render_callback: &mut dyn for<'a> FnMut(
            std::num::NonZero<u32>,
            std::num::NonZero<u32>,
            ColorType,
            u8,
            &'a mut [u8],
        ) -> Result<Option<DirtyRegion>, slint::PlatformError>,
    ) -> std::result::Result<(), slint::PlatformError> {
        let Some((width, height)): Option<(std::num::NonZeroU32, std::num::NonZeroU32)> =
            size.width.try_into().ok().zip(size.height.try_into().ok())
        else {
            return Ok(());
        };

        let age = if self.is_buffer_fresh.replace(false) { 0 } else { 1 };
        let pool = &mut self.pool.borrow_mut();
        *self.last_dirty_region.borrow_mut() = render_callback(
            width,
            height,
            skia_safe::ColorType::BGRA8888,
            age,
            self.primary_slot.borrow_mut().canvas(pool).unwrap(),
        )
        .unwrap();
        Ok(())
    }
}

pub struct SkiaWindowAdapter {
    pub(crate) window: Window,
    pub(crate) size: Cell<PhysicalSize>,
    pub(crate) size_original: Cell<PhysicalSize>,
    pub(crate) renderer: SkiaRenderer,
    #[allow(dead_code)]
    pub(crate) buffer_slint: Rc<SkiaSoftwareBufferReal>,
    pub(crate) needs_redraw: Cell<bool>,
    pub(crate) scale_factor: Cell<f32>,
    pub(crate) current_cursor: Cell<MouseCursor>,
}

impl Debug for SkiaWindowAdapter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SkiaWindowAdapter")
            .field("size", &self.size)
            .field("redraw", &self.needs_redraw)
            .finish()
    }
}

impl WindowAdapterInternal for SkiaWindowAdapter {
    fn set_mouse_cursor(&self, cursor: MouseCursor) {
        self.current_cursor.set(cursor);
    }
}

impl WindowAdapter for SkiaWindowAdapter {
    fn window(&self) -> &slint::Window {
        &self.window
    }

    fn size(&self) -> PhysicalSize {
        self.size.get()
    }

    fn renderer(&self) -> &dyn slint::platform::Renderer {
        &self.renderer
    }

    fn set_size(&self, size: slint::WindowSize) {
        log::info!("Set_size is called");
        self.size.set(size.to_physical(self.scale_factor.get()));
        self.window
            .dispatch_event(slint::platform::WindowEvent::Resized {
                size: size.to_logical(self.scale_factor.get()),
            })
    }

    fn request_redraw(&self) {
        self.needs_redraw.set(true);
    }

    fn internal(&self, _: i_slint_core::InternalToken) -> Option<&dyn WindowAdapterInternal> {
        Some(self)
    }
}

impl SkiaWindowAdapter {
    pub fn new(
        pool: Rc<RefCell<SlotPool>>,
        primary_slot: RefCell<Slot>,
        width: u32,
        height: u32,
        skia_ctx: &SkiaSharedContext,
    ) -> Rc<Self> {
        let buffer = Rc::new(SkiaSoftwareBufferReal {
            primary_slot,
            pool,
            last_dirty_region: Default::default(),
            is_buffer_fresh: Cell::new(true),
        });
        let renderer =
            SkiaRenderer::new_with_surface(skia_ctx, Box::new(SoftwareSurface::from(buffer.clone())));
        Rc::new_cyclic(|w: &Weak<Self>| Self {
            window: slint::Window::new(w.clone()),
            size: Cell::new(PhysicalSize { width, height }),
            size_original: Cell::new(PhysicalSize { width, height }),
            renderer,
            buffer_slint: buffer,
            scale_factor: Cell::new(1.),
            needs_redraw: Cell::new(true),
            current_cursor: Cell::new(MouseCursor::Default),
        })
    }

    pub fn draw(&self) -> bool {
        if self.needs_redraw.replace(false) {
            let _ = self.renderer.render().unwrap_or_else(|err| {
                log::warn!("Panicking because of error: {}", err);
                panic!("Seems like you have initialised slint before WaylandWindow");
            });
            true
        } else {
            false
        }
    }

    pub(crate) fn draw_if_needed(&self) -> bool {
        self.draw()
    }

    pub(crate) fn try_dispatch_event(
        &self,
        event: slint::platform::WindowEvent,
    ) -> Result<(), slint::PlatformError> {
        // El reloj de animación de slint solo avanza en sweep(); tras un idle
        // largo queda stale y una propiedad con `animate` calcularía
        // (now - base_stale) >= duración, saltando al valor final. Refrescarlo
        // aquí, justo antes de procesar el evento, es lo que exige el
        // contrato del backend ("before rendering or processing input event").
        slint::platform::update_timers_and_animations();
        self.window.try_dispatch_event(event)
    }

    pub(crate) fn changed_scale_factor(&self, scale: u32) -> (Buffer, u32, u32, f32) {
        let width: u32 = (self.size.get().width * scale + 60) / 120;
        let height: u32 = (self.size.get().height * scale + 60) / 120;
        let scale_factor: f32 = scale as f32 / 120.0;
        self.scale_factor.set(scale_factor);
        self.size.set(PhysicalSize { width, height });
        log::info!("Physical Size: width: {}, height: {}", width, height);
        (
            self.buffer_slint
                .refresh_buffer(width as i32, height as i32),
            width,
            height,
            scale_factor,
        )
    }
}
