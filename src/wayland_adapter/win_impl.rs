use crate::wayland_adapter::{WaylandWindow, way_helper::get_string};
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

impl TouchHandler for WaylandWindow {
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
        _surface: wl_surface::WlSurface,
        _id: i32,
        _position: (f64, f64),
    ) {
        log::info!("Down event from touch");
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

impl PointerHandler for WaylandWindow {
    fn pointer_frame(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _pointer: &wl_pointer::WlPointer,
        events: &[smithay_client_toolkit::seat::pointer::PointerEvent],
    ) {
        for event in events {
            match event.kind {
                PointerEventKind::Enter { serial } => {
                    log::info!("Pointer entered the window");
                    self.states.pointer_state.last_cursor_enter_serial = Some(serial);
                    self.states.pointer_state.pointer = Some(_pointer.clone());
                }
                PointerEventKind::Leave { .. } => {
                    log::info!("Pointer left the window");
                    self.states.pointer_state.last_cursor_enter_serial = None;
                    self.states.pointer_state.pointer = None;
                }
                PointerEventKind::Motion { .. } => {
                    self.adapter
                        .try_dispatch_event(WindowEvent::PointerMoved {
                            position: slint::LogicalPosition::new(
                                event.position.0 as f32,
                                event.position.1 as f32,
                            ),
                        })
                        .unwrap();
                }
                PointerEventKind::Press { button, .. } => {
                    let btn = match button {
                        smithay_client_toolkit::seat::pointer::BTN_LEFT => {
                            PointerEventButton::Left
                        }
                        smithay_client_toolkit::seat::pointer::BTN_RIGHT => {
                            PointerEventButton::Right
                        }
                        smithay_client_toolkit::seat::pointer::BTN_MIDDLE => {
                            PointerEventButton::Other
                        }
                        _ => PointerEventButton::Other,
                    };
                    self.adapter
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
                    let btn = match button {
                        smithay_client_toolkit::seat::pointer::BTN_LEFT => {
                            PointerEventButton::Left
                        }
                        smithay_client_toolkit::seat::pointer::BTN_RIGHT => {
                            PointerEventButton::Right
                        }
                        smithay_client_toolkit::seat::pointer::BTN_MIDDLE => {
                            PointerEventButton::Other
                        }
                        _ => PointerEventButton::Other,
                    };
                    self.adapter
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
                    self.adapter
                        .try_dispatch_event(WindowEvent::PointerScrolled {
                            delta_x: (if self.natural_scroll { h } else { -h } * 10.0) as f32,
                            delta_y: (if self.natural_scroll { v } else { -v } * 10.0) as f32,
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

impl KeyboardHandler for WaylandWindow {
    fn enter(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _keyboard: &wl_keyboard::WlKeyboard,
        _surface: &wl_surface::WlSurface,
        _serial: u32,
        _raw: &[u32],
        _keysyms: &[smithay_client_toolkit::seat::keyboard::Keysym],
    ) {
        log::info!("Keyboard entered");
    }

    fn leave(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _keyboard: &wl_keyboard::WlKeyboard,
        _surface: &wl_surface::WlSurface,
        _serial: u32,
    ) {
        log::info!("Keyboard left");
    }

    fn press_key(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _keyboard: &wl_keyboard::WlKeyboard,
        _serial: u32,
        event: KeyEvent,
    ) {
        let text = get_string(event);
        self.adapter
            .try_dispatch_event(WindowEvent::KeyPressed { text })
            .unwrap();
    }

    fn repeat_key(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _keyboard: &wl_keyboard::WlKeyboard,
        _serial: u32,
        event: KeyEvent,
    ) {
        let text = get_string(event);
        self.adapter
            .try_dispatch_event(WindowEvent::KeyPressed { text })
            .unwrap();
    }

    fn release_key(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _keyboard: &wl_keyboard::WlKeyboard,
        _serial: u32,
        event: KeyEvent,
    ) {
        let text = get_string(event);
        self.adapter
            .try_dispatch_event(WindowEvent::KeyReleased { text })
            .unwrap();
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

impl SeatHandler for WaylandWindow {
    fn seat_state(&mut self) -> &mut smithay_client_toolkit::seat::SeatState {
        &mut self.states.seat_state
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
            if let Ok(keyboard) = self.states.seat_state.get_keyboard(qh, &_seat, None::<RMLVO>)
            {
                self.states.keyboard_state = Some(keyboard);
            }
        } else if capability == Capability::Pointer {
            if let Ok(pointer) = self.states.seat_state.get_pointer(qh, &_seat) {
                self.states.pointer_state.pointer = Some(pointer);
            }
        } else if capability == Capability::Touch {
            if let Ok(touch) = self.states.seat_state.get_touch(qh, &_seat) {
                self.states.touch_state = Some(touch);
            }
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
            self.states.keyboard_state = None;
        } else if capability == Capability::Pointer {
            self.states.pointer_state.pointer = None;
        } else if capability == Capability::Touch {
            self.states.touch_state = None;
        }
    }

    fn remove_seat(&mut self, _conn: &Connection, _qh: &QueueHandle<Self>, _seat: wl_seat::WlSeat) {
        log::trace!("Seat removed");
    }
}
