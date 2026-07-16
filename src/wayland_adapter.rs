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
use slint::platform::WindowAdapter;
use smithay_client_toolkit::{
    compositor::{CompositorHandler, CompositorState, Region},
    delegate_compositor, delegate_keyboard, delegate_layer, delegate_output, delegate_pointer,
    delegate_registry, delegate_seat, delegate_shm, delegate_touch,
    output::{OutputHandler, OutputState},
    reexports::{
        calloop::{EventLoop, LoopHandle},
        calloop_wayland_source::WaylandSource,
        client::{
            Connection, EventQueue, QueueHandle,
            globals::registry_queue_init,
            protocol::{wl_keyboard::WlKeyboard, wl_output, wl_shm, wl_surface, wl_touch::WlTouch},
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
    rc::Rc,
    sync::{Arc, Mutex, Once, OnceLock, RwLock},
};

mod fractional_scaling;
mod slint_to_wl_cursor_mapping;
mod viewporter;
mod way_helper;
mod win_impl;

static AVAILABLE_MONITORS: OnceLock<RwLock<HashMap<String, wl_output::WlOutput>>> = OnceLock::new();
static SET_SLINT_PLATFORM: Once = Once::new();

#[derive(Debug)]
pub(crate) struct States {
    pub(crate) registry_state: RegistryState,
    pub(crate) seat_state: SeatState,
    pub(crate) output_state: OutputState,
    pub(crate) pointer_state: PointerState,
    pub(crate) keyboard_state: Option<WlKeyboard>,
    pub(crate) touch_state: Option<WlTouch>,
    pub(crate) shm: Shm,
    pub(crate) viewporter: Option<Viewport>,
}

pub struct WaylandWindow {
    pub(crate) adapter: Rc<SkiaWindowAdapter>,
    pub loop_handle: LoopHandle<'static, WaylandWindow>,
    pub(crate) buffer: Buffer,
    pub(crate) states: States,
    pub(crate) layer: Option<LayerSurface>,
    pub(crate) first_configure: Cell<bool>,
    pub(crate) natural_scroll: bool,
    pub(crate) is_hidden: Cell<bool>,
    pub layer_name: String,
    pub(crate) config: WindowConf,
    pub(crate) input_region: Region,
    pub(crate) opaque_region: Region,
    pub event_loop: Rc<RefCell<EventLoop<'static, WaylandWindow>>>,
    pub span: String,
}

impl std::fmt::Debug for WaylandWindow {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WaylandWindow")
            .field("adapter", &self.adapter)
            .field("first_configure", &self.first_configure)
            .field("is_hidden", &self.is_hidden)
            .field("config", &self.config)
            .finish()
    }
}

impl WaylandWindow {
    pub(crate) fn create_window(
        conn: &Connection,
        window_conf: WindowConf,
        layer_name: String,
    ) -> Self {
        let (globals, mut event_queue) = registry_queue_init(conn).unwrap();
        let qh: QueueHandle<WaylandWindow> = event_queue.handle();

        let compositor =
            CompositorState::bind(&globals, &qh).expect("wl_compositor is not available");
        let event_loop: EventLoop<'static, WaylandWindow> =
            EventLoop::try_new().expect("Failed to initialize the event loop!");
        let layer_shell = LayerShell::bind(&globals, &qh).expect("layer shell is not available");
        let shm = Shm::bind(&globals, &qh).expect("wl_shm is not available");
        let mut pool = SlotPool::new((window_conf.width * window_conf.height * 4) as usize, &shm)
            .expect("Failed to create pool");
        let input_region = Region::new(&compositor).expect("Couldn't create region");
        let opaque_region = Region::new(&compositor).expect("Couldn't create opaque region");
        input_region.add(0, 0, window_conf.width as i32, window_conf.height as i32);
        let cursor_manager =
            CursorShapeManager::bind(&globals, &qh).expect("cursor shape is not available");
        let fractional_scale_state: FractionalScaleState =
            FractionalScaleState::bind(&globals, &qh).expect("Fractional Scale couldn't be set");
        let stride = window_conf.width as i32 * 4;

        let surface = compositor.create_surface(&qh);
        let viewporter_state =
            ViewporterState::bind(&globals, &qh).expect("Couldn't set viewporter");

        let (way_pri_buffer, _) = pool
            .create_buffer(
                window_conf.width as i32,
                window_conf.height as i32,
                stride,
                wl_shm::Format::Argb8888,
            )
            .expect("Creating Buffer");

        let primary_slot = way_pri_buffer.slot();

        let pointer_state = PointerState {
            pointer: None,
            cursor_shape: cursor_manager,
            current_wayland_cursor: MouseCursor::Default,
            last_cursor_enter_serial: None,
        };

        let eventfd_fd = unsafe {
            libc::eventfd(0, libc::EFD_SEMAPHORE | libc::EFD_NONBLOCK)
        };
        if eventfd_fd == -1 {
            panic!("eventfd creation failed: {}", std::io::Error::last_os_error());
        }

        #[allow(clippy::type_complexity)]
        let slint_proxy: Arc<Mutex<Vec<Box<dyn FnOnce() + Send>>>> =
            Arc::new(Mutex::new(Vec::new()));
        let adapter_value: Rc<SkiaWindowAdapter> = SkiaWindowAdapter::new(
            Rc::new(RefCell::new(pool)),
            RefCell::new(primary_slot),
            window_conf.width,
            window_conf.height,
            slint_proxy.clone(),
            eventfd_fd,
        );

        ADAPTERS.with_borrow_mut(|v| v.push(adapter_value.clone()));
        SET_SLINT_PLATFORM.call_once(|| {
            log::trace!("Slint platform set");
            if let Err(err) = slint::platform::set_platform(Box::new(SlintPlatform::default())) {
                log::warn!("Error setting slint platform: {err}");
            }
        });
        set_event_sources(&event_loop, eventfd_fd);

        let mut win = WaylandWindow {
            adapter: adapter_value,
            loop_handle: event_loop.handle(),
            buffer: way_pri_buffer,
            states: States {
                registry_state: RegistryState::new(&globals),
                seat_state: SeatState::new(&globals, &qh),
                output_state: OutputState::new(&globals, &qh),
                pointer_state,
                keyboard_state: None,
                touch_state: None,
                shm,
                viewporter: None,
            },
            layer: None,
            first_configure: Cell::new(true),
            natural_scroll: window_conf.natural_scroll,
            is_hidden: Cell::new(false),
            layer_name: layer_name.clone(),
            config: window_conf.clone(),
            input_region,
            opaque_region,
            event_loop: Rc::new(RefCell::new(event_loop)),
            span: layer_name.clone(),
        };

        if AVAILABLE_MONITORS.get().is_none() {
            match WaylandWindow::get_available_monitors(&mut event_queue, &mut win) {
                Some(monitors) => {
                    let _ = AVAILABLE_MONITORS.get_or_init(|| RwLock::new(monitors));
                }
                None => log::warn!("Failed to get available monitors"),
            }
        }

        let target_output: Option<wl_output::WlOutput> =
            if let Some(name) = &window_conf.monitor_name {
                let output = AVAILABLE_MONITORS
                    .get()
                    .and_then(|monitors| monitors.read().ok())
                    .and_then(|monitors| monitors.get(name).cloned());
                if output.is_none() {
                    log::warn!("Monitor '{}' not found, using default monitor", name);
                }
                output
            } else {
                None
            };

        let layer = layer_shell.create_layer_surface(
            &qh,
            surface,
            window_conf.layer_type,
            Some(layer_name.clone()),
            target_output.as_ref(),
        );
        let fractional_scale = fractional_scale_state.get_scale(layer.wl_surface(), &qh);

        let viewporter = viewporter_state.get_viewport(layer.wl_surface(), &qh, fractional_scale);

        set_config(
            &win.config,
            &layer,
            Some(win.input_region.wl_region()),
            None,
        );
        layer.commit();

        win.layer = Some(layer);
        win.states.viewporter = Some(viewporter);

        log::info!("Win: {} layer created successfully.", layer_name);

        WaylandSource::new(conn.clone(), event_queue)
            .insert(win.loop_handle.clone())
            .unwrap();
        win
    }

    fn get_available_monitors(
        event_queue: &mut EventQueue<WaylandWindow>,
        win: &mut WaylandWindow,
    ) -> Option<HashMap<String, wl_output::WlOutput>> {
        event_queue.roundtrip(win).ok()?;

        Some(
            win.states
                .output_state
                .outputs()
                .filter_map(|output| {
                    let info = win.states.output_state.info(&output)?;
                    Some((info.name?, output))
                })
                .collect(),
        )
    }

    pub fn get_handler(&self) -> WinHandle {
        log::info!("Win: Handle provided.");
        WinHandle(self.loop_handle.clone())
    }

    pub fn spawn(name: &str, window_conf: WindowConf) -> Self {
        let conn = Connection::connect_to_env().unwrap();
        WaylandWindow::create_window(&conn, window_conf.clone(), name.to_string())
    }

    pub fn hide(&self) {
        if !self.is_hidden.replace(true) {
            log::info!("Win: Hiding window");
            self.layer.as_ref().unwrap().wl_surface().attach(None, 0, 0);
            self.layer.as_ref().unwrap().commit();
        }
    }

    pub fn show_again(&self) {
        if self.is_hidden.replace(false) {
            log::info!("Win: Showing window again");
            self.set_config_internal();
            self.first_configure.set(true);
            self.layer.as_ref().unwrap().commit();
        }
    }

    pub fn toggle(&self) {
        log::info!("Win: view toggled");
        if self.is_hidden.get() {
            self.show_again();
        } else {
            self.hide();
        }
    }

    pub fn add_input_region(&self, x: i32, y: i32, width: i32, height: i32) {
        log::info!(
            "Win: input region added: [x: {}, y: {}, width: {}, height: {}]",
            x, y, width, height
        );
        self.input_region.add(x, y, width, height);
        self.set_config_internal();
        self.layer.as_ref().unwrap().commit();
    }

    pub fn subtract_input_region(&self, x: i32, y: i32, width: i32, height: i32) {
        log::info!(
            "Win: input region removed: [x: {}, y: {}, width: {}, height: {}]",
            x, y, width, height
        );
        self.input_region.subtract(x, y, width, height);
        self.set_config_internal();
        self.layer.as_ref().unwrap().commit();
    }

    pub fn add_opaque_region(&self, x: i32, y: i32, width: i32, height: i32) {
        log::info!(
            "Win: opaque region added: [x: {}, y: {}, width: {}, height: {}]",
            x, y, width, height
        );
        self.opaque_region.add(x, y, width, height);
        self.set_config_internal();
        self.layer.as_ref().unwrap().commit();
    }

    pub fn subtract_opaque_region(&self, x: i32, y: i32, width: i32, height: i32) {
        log::info!(
            "Win: opaque region removed: [x: {}, y: {}, width: {}, height: {}]",
            x, y, width, height
        );
        self.opaque_region.subtract(x, y, width, height);
        self.set_config_internal();
        self.layer.as_ref().unwrap().commit();
    }

    fn set_config_internal(&self) {
        set_config(
            &self.config,
            self.layer.as_ref().unwrap(),
            Some(self.input_region.wl_region()),
            Some(self.opaque_region.wl_region()),
        );
    }

    fn converter(&mut self, qh: &QueueHandle<Self>) {
        {
            let proxy = self.adapter.slint_event_proxy.clone();
            if let Ok(mut list) = proxy.try_lock()
                && !(*list).is_empty()
            {
                let events: Vec<_> = (*list).drain(..).collect();
                drop(list);
                for event in events {
                    event();
                }
            }
        }

        slint::platform::update_timers_and_animations();
        let width: u32 = self.adapter.size.get().width;
        let height: u32 = self.adapter.size.get().height;
        let window_adapter = self.adapter.clone();

        if !self.is_hidden.get() {
            let redraw_val: bool = window_adapter.draw_if_needed();

            log::trace!("[conv] {} hidden={} fc={} redraw={} size={}x{}",
                self.span, self.is_hidden.get(), self.first_configure.get(), redraw_val, width, height);

            self.states
                .pointer_state
                .update_cursor(self.adapter.current_cursor.get(), &qh);

            let buffer = &self.buffer;
            if self.first_configure.get() || redraw_val {
                self.first_configure.set(false);
                log::info!("[conv] {} attaching buffer", self.span);
                let surface = self.layer.as_ref().unwrap().wl_surface();
                let dirty = self.adapter.buffer_slint.last_dirty_region.borrow();
                if let Some(ref dirty) = *dirty {
                    let scale = self.adapter.scale_factor.get();
                    for box2d in dirty.iter() {
                        let phys = (box2d.to_rect() * scale).round_out();
                        surface.damage_buffer(
                            phys.min_x() as i32,
                            phys.min_y() as i32,
                            phys.width() as i32,
                            phys.height() as i32,
                        );
                    }
                } else {
                    surface.damage_buffer(0, 0, width as i32, height as i32);
                }
                surface.attach(Some(buffer.wl_buffer()), 0, 0);
            }

            self.layer
                .as_ref()
                .unwrap()
                .wl_surface()
                .frame(qh, self.layer.as_ref().unwrap().wl_surface().clone());
            self.layer.as_ref().unwrap().commit();
        }
    }

    pub fn grab_focus(&self) {
        if !self.is_hidden.get()
            && self.config.board_interactivity.get() != KeyboardInteractivity::Exclusive
        {
            self.config
                .board_interactivity
                .set(KeyboardInteractivity::Exclusive);
            self.layer
                .as_ref()
                .unwrap()
                .set_keyboard_interactivity(KeyboardInteractivity::Exclusive);
            self.layer.as_ref().unwrap().commit();
        }
    }

    pub fn remove_focus(&self) {
        if !self.is_hidden.get()
            && self.config.board_interactivity.get() != KeyboardInteractivity::None
        {
            self.config
                .board_interactivity
                .set(KeyboardInteractivity::None);
            self.layer
                .as_ref()
                .unwrap()
                .set_keyboard_interactivity(KeyboardInteractivity::None);
            self.layer.as_ref().unwrap().commit();
        }
    }

    pub fn set_exclusive_zone(&mut self, val: i32) {
        self.config.exclusive_zone = Some(val);
        self.layer.as_ref().unwrap().set_exclusive_zone(val);
        self.layer.as_ref().unwrap().commit();
    }
}

impl WindowHandler for WaylandWindow {
    fn on_call(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        let event_loop = self.event_loop.clone();
        event_loop
            .borrow_mut()
            .dispatch(None::<std::time::Duration>, self)?;
        Ok(())
    }

    fn get_span(&self) -> String {
        self.span.clone()
    }
}

impl ProvidesRegistryState for WaylandWindow {
    fn registry(&mut self) -> &mut RegistryState {
        &mut self.states.registry_state
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

delegate_compositor!(WaylandWindow);
delegate_registry!(WaylandWindow);
delegate_output!(WaylandWindow);
delegate_shm!(WaylandWindow);
delegate_seat!(WaylandWindow);
delegate_keyboard!(WaylandWindow);
delegate_pointer!(WaylandWindow);
delegate_touch!(WaylandWindow);
delegate_layer!(WaylandWindow);
delegate_fractional_scale!(WaylandWindow);
delegate_viewporter!(WaylandWindow);

impl ShmHandler for WaylandWindow {
    fn shm_state(&mut self) -> &mut Shm {
        &mut self.states.shm
    }
}

impl OutputHandler for WaylandWindow {
    fn output_state(&mut self) -> &mut OutputState {
        &mut self.states.output_state
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

impl CompositorHandler for WaylandWindow {
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
        self.converter(qh);
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

impl FractionalScaleHandler for WaylandWindow {
    fn preferred_scale(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_surface::WlSurface,
        scale: u32,
    ) {
        log::info!("Scale factor changed, invoked from custom trait. {}", scale);
        let width_old = self.adapter.size_original.get().width;
        let height_old = self.adapter.size_original.get().height;
        self.layer.as_ref().unwrap().wl_surface().damage_buffer(
            0,
            0,
            self.adapter.size.get().width as i32,
            self.adapter.size.get().height as i32,
        );
        let (buffer, width, height, scale_factor) = self.adapter.changed_scale_factor(scale);
        self.config.width = width;
        self.config.height = height;
        self.buffer = buffer;
        self.adapter
            .try_dispatch_event(slint::platform::WindowEvent::ScaleFactorChanged { scale_factor })
            .unwrap();
        self.states.viewporter.as_ref().unwrap().set_source(
            0.,
            0.,
            self.adapter.size.get().width.into(),
            self.adapter.size.get().height.into(),
        );

        self.states
            .viewporter
            .as_ref()
            .unwrap()
            .set_destination(width_old as i32, height_old as i32);
        self.adapter.request_redraw();
        self.layer.as_ref().unwrap().commit();
    }
}

impl LayerShellHandler for WaylandWindow {
    fn closed(&mut self, _conn: &Connection, _qh: &QueueHandle<Self>, _layer: &LayerSurface) {
        log::trace!("Closure of layer called");
    }

    fn configure(
        &mut self,
        _conn: &Connection,
        qh: &QueueHandle<Self>,
        _layer: &LayerSurface,
        configure: LayerSurfaceConfigure,
        serial: u32,
    ) {
        log::info!("[conf] {} serial={} size={}x{} hidden={}", self.span, serial, configure.new_size.0, configure.new_size.1, self.is_hidden.get());
        self.converter(qh);
    }
}

#[derive(Clone, Debug)]
pub struct WinHandle(pub LoopHandle<'static, WaylandWindow>);

impl WinHandle {
    pub fn hide(&self) {
        self.0.insert_idle(|win| win.hide());
    }

    pub fn show_again(&self) {
        self.0.insert_idle(|win| win.show_again());
    }

    pub fn toggle(&self) {
        self.0.insert_idle(|win| win.toggle());
    }

    pub fn grab_focus(&self) {
        self.0.insert_idle(|win| win.grab_focus());
    }

    pub fn remove_focus(&self) {
        self.0.insert_idle(|win| win.remove_focus());
    }

    pub fn add_input_region(&self, x: i32, y: i32, width: i32, height: i32) {
        self.0
            .insert_idle(move |win| win.add_input_region(x, y, width, height));
    }

    pub fn subtract_input_region(&self, x: i32, y: i32, width: i32, height: i32) {
        self.0
            .insert_idle(move |win| win.subtract_input_region(x, y, width, height));
    }

    pub fn add_opaque_region(&self, x: i32, y: i32, width: i32, height: i32) {
        self.0
            .insert_idle(move |win| win.add_opaque_region(x, y, width, height));
    }

    pub fn subtract_opaque_region(&self, x: i32, y: i32, width: i32, height: i32) {
        self.0
            .insert_idle(move |win| win.subtract_opaque_region(x, y, width, height));
    }

    pub fn set_exclusive_zone(&self, val: i32) {
        self.0.insert_idle(move |win| win.set_exclusive_zone(val));
    }
}
