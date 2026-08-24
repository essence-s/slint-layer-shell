use crate::{
    WindowHandler,
    configure::WindowConf,
    skia_non_docs::SkiaWindowAdapter,
    slint_adapter::{ADAPTERS, SlintPlatform},
    wayland_adapter::{
        fractional_scaling::{FractionalScaleHandler, FractionalScaleState, delegate_fractional_scale},
        viewporter::{Viewport, ViewporterState, delegate_viewporter},
        way_helper::{PointerState, set_config, set_event_sources},
    },
};
use i_slint_core::items::MouseCursor;
use i_slint_core::timers::TimerList;
use i_slint_renderer_skia::SkiaSharedContext;
use slint::platform::{WindowAdapter, WindowEvent};
use smithay_client_toolkit::{
    compositor::{CompositorHandler, CompositorState, Region},
    delegate_compositor, delegate_keyboard, delegate_layer, delegate_output, delegate_pointer,
    delegate_registry, delegate_seat, delegate_shm, delegate_touch,
    output::{OutputHandler, OutputState},
    reexports::{
        calloop::{EventLoop, LoopHandle},
        calloop_wayland_source::WaylandSource,
        client::{
            Connection, QueueHandle,
            globals::registry_queue_init,
            protocol::{
                wl_keyboard::WlKeyboard,
                wl_output,
                wl_shm,
                wl_surface,
            },
        },
    },
    registry::{ProvidesRegistryState, RegistryState},
    seat::{SeatState, pointer::cursor_shape::CursorShapeManager},
    shell::{
        WaylandSurface,
        wlr_layer::{
            KeyboardInteractivity, LayerShell, LayerShellHandler, LayerSurface,
            LayerSurfaceConfigure,
        },
    },
    shm::{
        Shm, ShmHandler,
        slot::{Buffer, SlotPool},
    },
};
use std::{
    cell::{Cell, RefCell},
    collections::HashMap,
    error::Error,
    os::unix::io::RawFd,
    rc::Rc,
    sync::{Arc, Mutex, Once},
};

mod fractional_scaling;
mod slint_to_wl_cursor_mapping;
mod viewporter;
mod way_helper;
mod win_impl;

static SET_SLINT_PLATFORM: Once = Once::new();

#[allow(clippy::type_complexity)]
pub(crate) type SlintTaskQueue = Arc<Mutex<Vec<Box<dyn FnOnce() + Send>>>>;
pub(crate) type Qh = QueueHandle<AppData>;

thread_local! {
    static APP: RefCell<Option<App>> = const { RefCell::new(None) };
    /// Slint task queue + wakeup fd, deliberately stored OUTSIDE `App`:
    /// the slint platform creates its event proxy during `set_platform()`,
    /// which happens *inside* `ensure_app()` while `APP` is already
    /// mutably borrowed.
    static SLINT_EVENTS: SlintTaskQueue = Arc::new(Mutex::new(Vec::new()));
    static SLINT_EVENTFD: Cell<RawFd> = const { Cell::new(-1) };
}

pub(crate) fn slint_task_queue() -> SlintTaskQueue {
    SLINT_EVENTS.with(|q| q.clone())
}

pub(crate) fn slint_eventfd() -> RawFd {
    SLINT_EVENTFD.with(Cell::get)
}

struct App {
    event_loop: EventLoop<'static, AppData>,
    data: AppData,
    qh: Qh,
}

/// One shared Wayland application state for every window:
/// a single connection, a single event queue, a single calloop loop and one
/// set of bound globals. Each window only owns its surface-specific pieces.
pub(crate) struct AppData {
    pub(crate) shared: SharedStates,
    #[allow(clippy::type_complexity)]
    pub(crate) slint_events: SlintTaskQueue,
    pub(crate) windows: Vec<WaylandWindow>,
    /// Routes incoming events (keyed by wl_surface) to the owning window.
    pub(crate) surfaces: HashMap<wl_surface::WlSurface, usize>,
    pub(crate) keyboard_focus: Option<usize>,
}

pub(crate) struct SharedStates {
    pub(crate) registry_state: RegistryState,
    pub(crate) seat_state: SeatState,
    pub(crate) output_state: OutputState,
    pub(crate) compositor: CompositorState,
    pub(crate) layer_shell: LayerShell,
    pub(crate) shm: Shm,
    pub(crate) cursor_manager: CursorShapeManager,
    pub(crate) fractional_scale: FractionalScaleState,
    pub(crate) viewporter_state: ViewporterState,
    /// One Skia context shared by every window renderer.
    pub(crate) skia_ctx: SkiaSharedContext,
    pub(crate) monitors: HashMap<String, wl_output::WlOutput>,
    pub(crate) pointer_state: PointerState,
    pub(crate) keyboard_state: Option<WlKeyboard>,
}

#[derive(Clone)]
pub struct WaylandWindow(pub(crate) Rc<WaylandWindowInner>);

impl std::fmt::Debug for WaylandWindow {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WaylandWindow")
            .field("adapter", &self.0.adapter)
            .field("first_configure", &self.0.first_configure)
            .field("configured", &self.0.configured)
            .field("is_hidden", &self.0.is_hidden)
            .finish()
    }
}

pub(crate) struct WaylandWindowInner {
    pub(crate) id: usize,
    pub(crate) adapter: Rc<SkiaWindowAdapter>,
    pub(crate) buffer: RefCell<Buffer>,
    pub(crate) layer: LayerSurface,
    pub(crate) viewport: Viewport,
    pub(crate) config: RefCell<WindowConf>,
    pub(crate) input_region: Region,
    pub(crate) opaque_region: Region,
    pub(crate) first_configure: Cell<bool>,
    /// True once the layer surface received its first configure; buffers may
    /// only be attached afterwards (protocol requirement).
    pub(crate) configured: Cell<bool>,
    pub(crate) natural_scroll: bool,
    pub(crate) is_hidden: Cell<bool>,
    pub(crate) loop_handle: LoopHandle<'static, AppData>,
    pub(crate) span: String,
    pub(crate) layer_name: String,
}

fn ensure_app() {
    APP.with_borrow_mut(|slot| {
        if slot.is_some() {
            return;
        }
        let conn =
            Connection::connect_to_env().expect("Failed to connect to the Wayland compositor");
        let (globals, mut event_queue) =
            registry_queue_init::<AppData>(&conn).expect("Failed to init the registry queue");
        let qh: Qh = event_queue.handle();

        let compositor =
            CompositorState::bind(&globals, &qh).expect("wl_compositor is not available");
        let layer_shell = LayerShell::bind(&globals, &qh).expect("layer shell is not available");
        let shm = Shm::bind(&globals, &qh).expect("wl_shm is not available");
        let cursor_manager =
            CursorShapeManager::bind(&globals, &qh).expect("cursor shape is not available");
        let fractional_scale = FractionalScaleState::bind(&globals, &qh)
            .expect("Fractional Scale couldn't be set");
        let viewporter_state =
            ViewporterState::bind(&globals, &qh).expect("Couldn't set viewporter");

        let eventfd_fd = unsafe { libc::eventfd(0, libc::EFD_SEMAPHORE | libc::EFD_NONBLOCK) };
        if eventfd_fd == -1 {
            panic!("eventfd creation failed: {}", std::io::Error::last_os_error());
        }
        SLINT_EVENTFD.with(|c| c.set(eventfd_fd));

        let mut data = AppData {
            shared: SharedStates {
                registry_state: RegistryState::new(&globals),
                seat_state: SeatState::new(&globals, &qh),
                output_state: OutputState::new(&globals, &qh),
                compositor,
                layer_shell,
                shm,
                cursor_manager,
                fractional_scale,
                viewporter_state,
                skia_ctx: SkiaSharedContext::default(),
                monitors: HashMap::new(),
                pointer_state: PointerState {
                    pointer: None,
                    current_wayland_cursor: MouseCursor::Default,
                    last_cursor_enter_serial: None,
                },
                keyboard_state: None,
            },
            slint_events: slint_task_queue(),
            windows: Vec::new(),
            surfaces: HashMap::new(),
            keyboard_focus: None,
        };

        let event_loop: EventLoop<'static, AppData> =
            EventLoop::try_new().expect("Failed to initialize the event loop!");
        set_event_sources(event_loop.handle(), eventfd_fd, qh.clone());

        // Discover outputs once with a single roundtrip.
        if event_queue.roundtrip(&mut data).is_ok() {
            let outputs: HashMap<String, wl_output::WlOutput> = data
                .shared
                .output_state
                .outputs()
                .filter_map(|output| {
                    let info = data.shared.output_state.info(&output)?;
                    Some((info.name?, output))
                })
                .collect();
            data.shared.monitors = outputs;
        } else {
            log::warn!("Initial roundtrip failed; monitor discovery skipped");
        }

        WaylandSource::new(conn.clone(), event_queue)
            .insert(event_loop.handle())
            .expect("Failed to register the wayland source");

        SET_SLINT_PLATFORM.call_once(|| {
            log::trace!("Slint platform set");
            if let Err(err) = slint::platform::set_platform(Box::new(SlintPlatform)) {
                log::warn!("Error setting slint platform: {err}");
            }
        });

        *slot = Some(App { event_loop, data, qh });
    });
}

impl WaylandWindow {
    pub fn spawn(name: &str, window_conf: WindowConf) -> Self {
        ensure_app();
        APP.with_borrow_mut(|slot| {
            let app = slot.as_mut().expect("slint-layer-shell app must be initialized");
            Self::create_window(app, name.to_string(), window_conf)
        })
    }

    fn create_window(app: &mut App, layer_name: String, window_conf: WindowConf) -> Self {
        let qh = app.qh.clone();
        let width = window_conf.width;
        let height = window_conf.height;
        let natural_scroll = window_conf.natural_scroll;

        let surface = app.data.shared.compositor.create_surface(&qh);

        let target_output: Option<wl_output::WlOutput> =
            window_conf.monitor_name.as_ref().and_then(|name| {
                let output = app.data.shared.monitors.get(name).cloned();
                if output.is_none() {
                    log::warn!("Monitor '{name}' not found, using default monitor");
                }
                output
            });

        let layer = app.data.shared.layer_shell.create_layer_surface(
            &qh,
            surface.clone(),
            window_conf.layer_type,
            Some(layer_name.clone()),
            target_output.as_ref(),
        );

        let fractional_scale =
            app.data.shared.fractional_scale.get_scale(layer.wl_surface(), &qh);
        let viewport = app.data.shared.viewporter_state.get_viewport(
            layer.wl_surface(),
            &qh,
            fractional_scale,
        );

        let stride = width as i32 * 4;
        let mut pool =
            SlotPool::new((width * height * 4) as usize, &app.data.shared.shm)
                .expect("Failed to create pool");
        let (way_pri_buffer, _) = pool
            .create_buffer(width as i32, height as i32, stride, wl_shm::Format::Argb8888)
            .expect("Creating Buffer");
        let primary_slot = way_pri_buffer.slot();

        let input_region =
            Region::new(&app.data.shared.compositor).expect("Couldn't create region");
        let opaque_region =
            Region::new(&app.data.shared.compositor).expect("Couldn't create opaque region");
        input_region.add(0, 0, width as i32, height as i32);

        set_config(&window_conf, &layer, Some(input_region.wl_region()), None);
        layer.commit();

        let adapter_value = SkiaWindowAdapter::new(
            Rc::new(RefCell::new(pool)),
            RefCell::new(primary_slot),
            width,
            height,
            &app.data.shared.skia_ctx,
        );
        ADAPTERS.with_borrow_mut(|v| v.push(adapter_value.clone()));

        let id = app.data.windows.len();
        app.data.surfaces.insert(surface, id);

        let win = WaylandWindow(Rc::new(WaylandWindowInner {
            id,
            adapter: adapter_value,
            buffer: RefCell::new(way_pri_buffer),
            layer,
            viewport,
            config: RefCell::new(window_conf),
            input_region,
            opaque_region,
            first_configure: Cell::new(true),
            configured: Cell::new(false),
            natural_scroll,
            is_hidden: Cell::new(false),
            loop_handle: app.event_loop.handle(),
            span: layer_name.clone(),
            layer_name,
        }));
        app.data.windows.push(win.clone());

        log::info!("Win: {} layer created successfully.", win.0.span);
        win
    }

    pub fn hide(&self) {
        let inner = &*self.0;
        if !inner.is_hidden.replace(true) {
            log::info!("Win: Hiding window {}", inner.span);
            let layer = &inner.layer;
            layer.wl_surface().attach(None, 0, 0);
            layer.commit();
        }
    }

    pub fn show_again(&self) {
        let inner = &*self.0;
        if inner.is_hidden.replace(false) {
            log::info!("Win: Showing window again {}", inner.span);
            {
                let config = inner.config.borrow();
                set_config(
                    &config,
                    &inner.layer,
                    Some(inner.input_region.wl_region()),
                    Some(inner.opaque_region.wl_region()),
                );
            }
            // Re-applying layer state requires waiting for a fresh configure
            // round: attaching a buffer before it is a protocol error.
            inner.first_configure.set(true);
            inner.configured.set(false);
            inner.layer.commit();
        }
    }

    pub fn toggle(&self) {
        log::info!("Win: view toggled {}", self.0.span);
        if self.0.is_hidden.get() {
            self.show_again();
        } else {
            self.hide();
        }
    }

    pub fn add_input_region(&self, x: i32, y: i32, width: i32, height: i32) {
        let inner = &*self.0;
        log::info!(
            "Win: {} input region added: [x: {}, y: {}, width: {}, height: {}]",
            inner.span, x, y, width, height
        );
        inner.input_region.add(x, y, width, height);
        self.apply_regions_and_commit();
    }

    pub fn subtract_input_region(&self, x: i32, y: i32, width: i32, height: i32) {
        let inner = &*self.0;
        log::info!(
            "Win: {} input region removed: [x: {}, y: {}, width: {}, height: {}]",
            inner.span, x, y, width, height
        );
        inner.input_region.subtract(x, y, width, height);
        self.apply_regions_and_commit();
    }

    pub fn add_opaque_region(&self, x: i32, y: i32, width: i32, height: i32) {
        let inner = &*self.0;
        log::info!(
            "Win: {} opaque region added: [x: {}, y: {}, width: {}, height: {}]",
            inner.span, x, y, width, height
        );
        inner.opaque_region.add(x, y, width, height);
        self.apply_regions_and_commit();
    }

    pub fn subtract_opaque_region(&self, x: i32, y: i32, width: i32, height: i32) {
        let inner = &*self.0;
        log::info!(
            "Win: {} opaque region removed: [x: {}, y: {}, width: {}, height: {}]",
            inner.span, x, y, width, height
        );
        inner.opaque_region.subtract(x, y, width, height);
        self.apply_regions_and_commit();
    }

    fn apply_regions_and_commit(&self) {
        let inner = &*self.0;
        {
            let config = inner.config.borrow();
            set_config(
                &config,
                &inner.layer,
                Some(inner.input_region.wl_region()),
                Some(inner.opaque_region.wl_region()),
            );
        }
        inner.layer.commit();
    }

    pub fn grab_focus(&self) {
        let inner = &*self.0;
        if !inner.is_hidden.get()
            && inner.config.borrow().board_interactivity.get() != KeyboardInteractivity::Exclusive
        {
            inner.config.borrow().board_interactivity.set(KeyboardInteractivity::Exclusive);
            inner.layer.set_keyboard_interactivity(KeyboardInteractivity::Exclusive);
            // Keyboard interactivity is layer state -> wait for re-configure.
            inner.configured.set(false);
            inner.layer.commit();
        }
    }

    pub fn remove_focus(&self) {
        let inner = &*self.0;
        if !inner.is_hidden.get()
            && inner.config.borrow().board_interactivity.get() != KeyboardInteractivity::None
        {
            inner.config.borrow().board_interactivity.set(KeyboardInteractivity::None);
            inner.layer.set_keyboard_interactivity(KeyboardInteractivity::None);
            inner.configured.set(false);
            inner.layer.commit();
        }
    }

    pub fn set_exclusive_zone(&self, val: i32) {
        let inner = &*self.0;
        inner.config.borrow_mut().exclusive_zone = Some(val);
        inner.layer.set_exclusive_zone(val);
        // Exclusive zone is layer state -> wait for re-configure.
        inner.configured.set(false);
        inner.layer.commit();
    }

    pub fn get_handler(&self) -> WinHandle {
        log::info!("Win: {} handle provided.", self.0.span);
        WinHandle { handle: self.0.loop_handle.clone(), idx: self.0.id }
    }

    pub fn layer_name(&self) -> &str {
        &self.0.layer_name
    }

    pub fn span(&self) -> &str {
        &self.0.span
    }

    pub fn is_visible(&self) -> bool {
        self.0.is_hidden.get()
    }
}

impl WindowHandler for WaylandWindow {
    fn on_call(&mut self) -> Result<(), Box<dyn Error>> {
        Ok(())
    }

    fn get_span(&self) -> String {
        self.0.span.clone()
    }
}

impl AppData {
    fn drain_slint_events(&mut self) {
        if let Ok(mut list) = self.slint_events.try_lock()
            && !list.is_empty()
        {
            let events: Vec<_> = list.drain(..).collect();
            drop(list);
            for event in events {
                event();
            }
        }
    }

    /// Process all pending work: queued slint closures, timer/animation ticks
    /// and rendering of every window that actually needs a new frame.
    pub(crate) fn sweep(&mut self, qh: &Qh) {
        self.drain_slint_events();
        slint::platform::update_timers_and_animations();
        for idx in 0..self.windows.len() {
            self.render_window(idx, qh);
        }
    }

    /// Draw + commit only when the window has pending work. The wayland
    /// frame-callback chain is only kept alive while animations/timers are
    /// active, so an idle desktop shell costs zero wakeups.
    fn render_window(&mut self, idx: usize, qh: &Qh) {
        let Some(win) = self.windows.get(idx) else { return };
        let inner = &*win.0;

        if inner.is_hidden.get() || !inner.configured.get() {
            return;
        }
        // Query the flags WITHOUT consuming them: draw_if_needed() consumes
        // needs_redraw itself and only paints when it was set.
        let first_configure = inner.first_configure.get();
        if !first_configure && !inner.adapter.needs_redraw.get() {
            return;
        }

        inner.adapter.draw_if_needed();
        inner.first_configure.set(false);

        let size = inner.adapter.size.get();
        let surface = inner.layer.wl_surface();

        {
            let dirty = inner.adapter.buffer_slint.last_dirty_region.borrow();
            if let Some(ref region) = *dirty {
                let scale = inner.adapter.scale_factor.get();
                for box2d in region.iter() {
                    let phys = (box2d.to_rect() * scale).round_out();
                    surface.damage_buffer(
                        phys.min_x() as i32,
                        phys.min_y() as i32,
                        phys.width() as i32,
                        phys.height() as i32,
                    );
                }
            } else {
                surface.damage_buffer(0, 0, size.width as i32, size.height as i32);
            }
        }

        surface.attach(Some(inner.buffer.borrow().wl_buffer()), 0, 0);

        // Only keep requesting frame callbacks while there is queued work;
        // otherwise let the whole process sleep until something happens.
        let animating =
            TimerList::next_timeout().is_some() || inner.adapter.needs_redraw.get();
        if animating {
            surface.frame(qh, surface.clone());
        }
        surface.commit();

        self.shared.pointer_state.update_cursor(
            &self.shared.cursor_manager,
            inner.adapter.current_cursor.get(),
            qh,
        );
    }

    fn window_for(&self, surface: &wl_surface::WlSurface) -> Option<&WaylandWindow> {
        self.surfaces.get(surface).and_then(|idx| self.windows.get(*idx))
    }
}

pub(crate) fn start_event_loop(
    handlers: &mut [Box<dyn WindowHandler>],
) -> Result<(), Box<dyn Error>> {
    ensure_app();
    APP.with_borrow_mut(|slot| {
        let mut app = slot.take().expect("slint-layer-shell app must be initialized");
        let result = {
            let App { event_loop, data, qh } = &mut app;
            run_inner(event_loop, data, qh, handlers)
        };
        *slot = Some(app);
        result
    })
}

fn run_inner(
    event_loop: &mut EventLoop<'static, AppData>,
    data: &mut AppData,
    qh: &Qh,
    handlers: &mut [Box<dyn WindowHandler>],
) -> Result<(), Box<dyn Error>> {
    loop {
        // Sleep until the next timer, animation tick or wayland event:
        // no polling, no fixed 16ms spin.
        let timeout = TimerList::next_timeout().map(|deadline| {
            let now = i_slint_core::animations::Instant::now();
            if deadline > now {
                std::time::Duration::from_millis(deadline.as_millis() - now.as_millis())
            } else {
                std::time::Duration::ZERO
            }
        });
        event_loop.dispatch(timeout, data)?;
        data.sweep(qh);
        // Sweep callbacks (slint timers, queued tasks, WinHandle actions)
        // may have queued calloop idle callbacks; flush them now instead of
        // letting them wait for the next real wakeup.
        event_loop.dispatch(Some(std::time::Duration::ZERO), data)?;
        // The flush above can deliver more wayland events -> render again
        // (cheap no-op when nothing is dirty).
        data.sweep(qh);
        for handler in handlers.iter_mut() {
            handler.on_call()?;
        }
    }
}

#[derive(Clone, Debug)]
pub struct WinHandle {
    pub(crate) handle: LoopHandle<'static, AppData>,
    pub(crate) idx: usize,
}

impl WinHandle {
    #[allow(clippy::redundant_closure_for_method_calls)]
    fn with_window<F>(&self, f: F)
    where
        F: FnOnce(&WaylandWindow) + 'static,
    {
        let idx = self.idx;
        self.handle.insert_idle(move |data: &mut AppData| {
            if let Some(win) = data.windows.get(idx) {
                f(win);
            }
        });
    }

    pub fn hide(&self) {
        self.with_window(WaylandWindow::hide);
    }

    pub fn show_again(&self) {
        self.with_window(WaylandWindow::show_again);
    }

    pub fn toggle(&self) {
        self.with_window(WaylandWindow::toggle);
    }

    pub fn grab_focus(&self) {
        self.with_window(WaylandWindow::grab_focus);
    }

    pub fn remove_focus(&self) {
        self.with_window(WaylandWindow::remove_focus);
    }

    pub fn add_input_region(&self, x: i32, y: i32, width: i32, height: i32) {
        self.with_window(move |win| win.add_input_region(x, y, width, height));
    }

    pub fn subtract_input_region(&self, x: i32, y: i32, width: i32, height: i32) {
        self.with_window(move |win| win.subtract_input_region(x, y, width, height));
    }

    pub fn add_opaque_region(&self, x: i32, y: i32, width: i32, height: i32) {
        self.with_window(move |win| win.add_opaque_region(x, y, width, height));
    }

    pub fn subtract_opaque_region(&self, x: i32, y: i32, width: i32, height: i32) {
        self.with_window(move |win| win.subtract_opaque_region(x, y, width, height));
    }

    pub fn set_exclusive_zone(&self, val: i32) {
        self.with_window(move |win| win.set_exclusive_zone(val));
    }
}

// ---------------------------------------------------------------------------
// Delegates: implemented once on the shared AppData, routed by wl_surface.
// ---------------------------------------------------------------------------

impl ProvidesRegistryState for AppData {
    fn registry(&mut self) -> &mut RegistryState {
        &mut self.shared.registry_state
    }
    fn runtime_add_global(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _name: u32,
        _interface: &str,
        _version: u32,
    ) {
    }
    fn runtime_remove_global(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _name: u32,
        _interface: &str,
    ) {
    }
}

impl ShmHandler for AppData {
    fn shm_state(&mut self) -> &mut Shm {
        &mut self.shared.shm
    }
}

impl OutputHandler for AppData {
    fn output_state(&mut self) -> &mut OutputState {
        &mut self.shared.output_state
    }

    fn new_output(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _output: wl_output::WlOutput,
    ) {
        log::trace!("New output Source Added");
    }

    fn update_output(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _output: wl_output::WlOutput,
    ) {
        log::trace!("Existing output is updated");
    }

    fn output_destroyed(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _output: wl_output::WlOutput,
    ) {
        log::trace!("Output is destroyed");
    }
}

impl CompositorHandler for AppData {
    fn scale_factor_changed(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_surface::WlSurface,
        _: i32,
    ) {
        log::info!("Scale factor changed, compositor msg");
    }

    fn transform_changed(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _surface: &wl_surface::WlSurface,
        _new_transform: wl_output::Transform,
    ) {
        log::trace!("Compositor transformation changed");
    }

    fn frame(
        &mut self,
        _conn: &Connection,
        qh: &QueueHandle<Self>,
        _surface: &wl_surface::WlSurface,
        _time: u32,
    ) {
        self.sweep(qh);
    }

    fn surface_enter(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _surface: &wl_surface::WlSurface,
        _output: &wl_output::WlOutput,
    ) {
        log::trace!("Surface entered");
    }

    fn surface_leave(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _surface: &wl_surface::WlSurface,
        _output: &wl_output::WlOutput,
    ) {
        log::trace!("Surface left");
    }
}

impl FractionalScaleHandler for AppData {
    fn preferred_scale(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        surface: &wl_surface::WlSurface,
        scale: u32,
    ) {
        let Some(win) = self.window_for(surface) else { return };
        let inner = &*win.0;
        log::info!("[scale] {} preferred scale: {scale}", inner.span);

        let size_old = inner.adapter.size_original.get();
        inner.layer.wl_surface().damage_buffer(
            0,
            0,
            inner.adapter.size.get().width as i32,
            inner.adapter.size.get().height as i32,
        );

        let (buffer, width, height, scale_factor) = inner.adapter.changed_scale_factor(scale);
        {
            let mut config = inner.config.borrow_mut();
            config.width = width;
            config.height = height;
        }
        *inner.buffer.borrow_mut() = buffer;

        inner
            .adapter
            .try_dispatch_event(WindowEvent::ScaleFactorChanged { scale_factor })
            .unwrap();

        let size = inner.adapter.size.get();
        inner.viewport.set_source(0., 0., size.width.into(), size.height.into());
        inner.viewport.set_destination(size_old.width as i32, size_old.height as i32);

        inner.adapter.request_redraw();
        inner.layer.commit();
    }
}

impl LayerShellHandler for AppData {
    fn closed(&mut self, _conn: &Connection, _qh: &QueueHandle<Self>, layer: &LayerSurface) {
        match self.window_for(layer.wl_surface()) {
            Some(win) => log::info!("Win: {} layer closed", win.0.span),
            None => log::trace!("Closure of unknown layer called"),
        }
    }

    fn configure(
        &mut self,
        _conn: &Connection,
        qh: &QueueHandle<Self>,
        layer: &LayerSurface,
        configure: LayerSurfaceConfigure,
        serial: u32,
    ) {
        let Some(win) = self.window_for(layer.wl_surface()) else { return };
        let inner = &*win.0;
        inner.configured.set(true);
        log::info!(
            "[conf] {} serial={} size={}x{} hidden={}",
            inner.span,
            serial,
            configure.new_size.0,
            configure.new_size.1,
            inner.is_hidden.get()
        );
        self.sweep(qh);
    }
}

delegate_compositor!(AppData);
delegate_registry!(AppData);
delegate_output!(AppData);
delegate_shm!(AppData);
delegate_seat!(AppData);
delegate_keyboard!(AppData);
delegate_pointer!(AppData);
delegate_touch!(AppData);
delegate_layer!(AppData);
delegate_fractional_scale!(AppData);
delegate_viewporter!(AppData);

// ---------------------------------------------------------------------------
// Helpers used by slint_adapter before the loop starts running.
// ---------------------------------------------------------------------------

/// Handles for the global slint task queue + wakeup fd.
///
/// Reads independent thread-locals so it is safe to call from inside
/// `set_platform()` (i.e. while `APP` is borrowed by `ensure_app`).
pub(crate) fn proxy_handles() -> (SlintTaskQueue, RawFd) {
    (slint_task_queue(), slint_eventfd())
}
