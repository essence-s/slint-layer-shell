use crate::wayland_adapter::{AppData, way_helper::get_string};
use slint::platform::{PointerEventButton, WindowEvent};
use smithay_client_toolkit::{
    reexports::client::{
        Connection, QueueHandle,
        protocol::{wl_keyboard, wl_pointer, wl_seat, wl_surface, wl_touch},
    },
    seat::{
        Capability, SeatHandler,
        keyboard::{KeyEvent, KeyboardHandler, Modifiers, RMLVO, RawModifiers},
        pointer::{PointerEventKind, PointerHandler},
        touch::TouchHandler,
    },
};

impl TouchHandler for AppData {
    fn up(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _touch: &wl_touch::WlTouch,
        _serial: u32,
        _time: u32,
        _id: i32,
    ) {
        log::info!("Up event from touch");
    }

    fn down(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _touch: &wl_touch::WlTouch,
        _serial: u32,
        _time: u32,
        surface: wl_surface::WlSurface,
        _id: i32,
        _position: (f64, f64),
    ) {
        log::info!("Down event from touch ({})", self.surfaces.contains_key(&surface));
    }

    fn motion(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _touch: &wl_touch::WlTouch,
        _time: u32,
        _id: i32,
        _position: (f64, f64),
    ) {
    }

    fn shape(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _touch: &wl_touch::WlTouch,
        _id: i32,
        _major: f64,
        _minor: f64,
    ) {
    }

    fn orientation(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _touch: &wl_touch::WlTouch,
        _id: i32,
        _orientation: f64,
    ) {
    }

    fn cancel(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _touch: &wl_touch::WlTouch,
    ) {
    }
}

impl PointerHandler for AppData {
    fn pointer_frame(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        pointer: &wl_pointer::WlPointer,
        events: &[smithay_client_toolkit::seat::pointer::PointerEvent],
    ) {
        for event in events {
            let Some(win) = self.window_for(&event.surface) else { continue };
            let inner = &*win.0;
            match event.kind {
                PointerEventKind::Enter { serial } => {
                    log::info!("Pointer entered {}", inner.span);
                    self.shared.pointer_state.last_cursor_enter_serial = Some(serial);
                    self.shared.pointer_state.pointer = Some(pointer.clone());
                }
                PointerEventKind::Leave { .. } => {
                    log::info!("Pointer left {}", inner.span);
                    self.shared.pointer_state.last_cursor_enter_serial = None;
                    self.shared.pointer_state.pointer = None;
                }
                PointerEventKind::Motion { .. } => {
                    inner
                        .adapter
                        .try_dispatch_event(WindowEvent::PointerMoved {
                            position: slint::LogicalPosition::new(
                                event.position.0 as f32,
                                event.position.1 as f32,
                            ),
                        })
                        .unwrap();
                }
                PointerEventKind::Press { button, .. } => {
                    let btn = map_button(button);
                    inner
                        .adapter
                        .try_dispatch_event(WindowEvent::PointerPressed {
                            button: btn,
                            position: slint::LogicalPosition::new(
                                event.position.0 as f32,
                                event.position.1 as f32,
                            ),
                        })
                        .unwrap();
                }
                PointerEventKind::Release { button, .. } => {
                    let btn = map_button(button);
                    inner
                        .adapter
                        .try_dispatch_event(WindowEvent::PointerReleased {
                            button: btn,
                            position: slint::LogicalPosition::new(
                                event.position.0 as f32,
                                event.position.1 as f32,
                            ),
                        })
                        .unwrap();
                }
                PointerEventKind::Axis {
                    ref horizontal,
                    ref vertical,
                    ..
                } => {
                    let h = horizontal.absolute;
                    let v = vertical.absolute;
                    if h == 0.0 && v == 0.0 {
                        continue;
                    }
                    inner
                        .adapter
                        .try_dispatch_event(WindowEvent::PointerScrolled {
                            delta_x: (if inner.natural_scroll { h } else { -h } * 10.0) as f32,
                            delta_y: (if inner.natural_scroll { v } else { -v } * 10.0) as f32,
                            position: slint::LogicalPosition::new(
                                event.position.0 as f32,
                                event.position.1 as f32,
                            ),
                        })
                        .unwrap();
                }
            }
        }
    }
}

fn map_button(button: u32) -> PointerEventButton {
    match button {
        smithay_client_toolkit::seat::pointer::BTN_LEFT => PointerEventButton::Left,
        smithay_client_toolkit::seat::pointer::BTN_RIGHT => PointerEventButton::Right,
        smithay_client_toolkit::seat::pointer::BTN_MIDDLE => PointerEventButton::Middle,
        _ => PointerEventButton::Other,
    }
}

impl KeyboardHandler for AppData {
    fn enter(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _keyboard: &wl_keyboard::WlKeyboard,
        surface: &wl_surface::WlSurface,
        _serial: u32,
        _raw: &[u32],
        _keysyms: &[smithay_client_toolkit::seat::keyboard::Keysym],
    ) {
        self.keyboard_focus = self.surfaces.get(surface).copied();
        if let Some(idx) = self.keyboard_focus
            && let Some(win) = self.windows.get(idx)
        {
            log::info!("Keyboard entered {}", win.0.span);
        }
    }

    fn leave(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _keyboard: &wl_keyboard::WlKeyboard,
        _surface: &wl_surface::WlSurface,
        _serial: u32,
    ) {
        log::trace!("Keyboard left");
        self.keyboard_focus = None;
    }

    fn press_key(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _keyboard: &wl_keyboard::WlKeyboard,
        _serial: u32,
        event: KeyEvent,
    ) {
        dispatch_key(self, WindowEvent::KeyPressed { text: get_string(event) });
    }

    fn repeat_key(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _keyboard: &wl_keyboard::WlKeyboard,
        _serial: u32,
        event: KeyEvent,
    ) {
        dispatch_key(self, WindowEvent::KeyPressed { text: get_string(event) });
    }

    fn release_key(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _keyboard: &wl_keyboard::WlKeyboard,
        _serial: u32,
        event: KeyEvent,
    ) {
        dispatch_key(self, WindowEvent::KeyReleased { text: get_string(event) });
    }

    fn update_modifiers(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _keyboard: &wl_keyboard::WlKeyboard,
        _serial: u32,
        _modifiers: Modifiers,
        _raw_modifiers: RawModifiers,
        _layout: u32,
    ) {
        log::trace!("Modifiers changed");
    }
}

fn dispatch_key(data: &mut AppData, event: WindowEvent) {
    let Some(idx) = data.keyboard_focus else { return };
    if let Some(win) = data.windows.get(idx) {
        win.0.adapter.try_dispatch_event(event).unwrap();
    }
}

impl SeatHandler for AppData {
    fn seat_state(&mut self) -> &mut smithay_client_toolkit::seat::SeatState {
        &mut self.shared.seat_state
    }

    fn new_seat(&mut self, _conn: &Connection, _qh: &QueueHandle<Self>, _seat: wl_seat::WlSeat) {
        log::trace!("New seat");
    }

    fn new_capability(
        &mut self,
        _conn: &Connection,
        qh: &QueueHandle<Self>,
        _seat: wl_seat::WlSeat,
        capability: Capability,
    ) {
        if capability == Capability::Keyboard {
            if let Ok(keyboard) = self.shared.seat_state.get_keyboard(qh, &_seat, None::<RMLVO>) {
                self.shared.keyboard_state = Some(keyboard);
            }
        } else if capability == Capability::Pointer {
            if let Ok(pointer) = self.shared.seat_state.get_pointer(qh, &_seat) {
                self.shared.pointer_state.pointer = Some(pointer);
            }
        } else if capability == Capability::Touch {
            // Touch state is currently only tracked for completeness.
        }
    }

    fn remove_capability(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _seat: wl_seat::WlSeat,
        capability: Capability,
    ) {
        if capability == Capability::Keyboard {
            self.shared.keyboard_state = None;
        } else if capability == Capability::Pointer {
            self.shared.pointer_state.pointer = None;
        }
    }

    fn remove_seat(&mut self, _conn: &Connection, _qh: &QueueHandle<Self>, _seat: wl_seat::WlSeat) {
        log::trace!("Seat removed");
    }
}
