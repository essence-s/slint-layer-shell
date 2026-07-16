use crate::{
    layer_properties::WindowConf,
    wayland_adapter::{WaylandWindow, slint_to_wl_cursor_mapping},
};
use i_slint_core::items::MouseCursor;
use slint::{SharedString, platform::Key};
use smithay_client_toolkit::{
    reexports::{
        calloop::{
            EventLoop, Interest, Mode, PostAction,
            generic::Generic,
        },
        client::{
            QueueHandle,
            protocol::{wl_pointer, wl_region::WlRegion},
        },
    },
    seat::{
        keyboard::{KeyEvent, Keysym},
        pointer::cursor_shape::CursorShapeManager,
    },
    shell::{wlr_layer::LayerSurface, WaylandSurface},
};
use std::{
    os::unix::io::{BorrowedFd, RawFd},
};

pub(super) fn set_config(
    window_conf: &WindowConf,
    layer: &LayerSurface,
    input_region: Option<&WlRegion>,
    opaque_region: Option<&WlRegion>,
) {
    layer.set_size(window_conf.width, window_conf.height);
    layer.set_margin(
        window_conf.margin.0,
        window_conf.margin.1,
        window_conf.margin.2,
        window_conf.margin.3,
    );
    layer.set_keyboard_interactivity(window_conf.board_interactivity.get());
    if let Some(in_region) = input_region {
        layer.set_input_region(Some(in_region));
    }
    if let Some(op_region) = opaque_region {
        layer.set_opaque_region(Some(op_region));
    }
    layer.set_layer(window_conf.layer_type);
    set_anchor(window_conf, layer);
}

fn set_anchor(window_conf: &WindowConf, layer: &LayerSurface) {
    let mut anchors = window_conf.anchor.into_iter().flatten();
    if let Some(mut combined) = anchors.next() {
        for a in anchors {
            combined.insert(a);
        }
        layer.set_anchor(combined);
    }
    if let Some(val) = window_conf.exclusive_zone {
        layer.set_exclusive_zone(val);
    }
}

pub(super) fn get_string(event: KeyEvent) -> SharedString {
    let mut key: Option<Key> = None;
    match event.keysym {
        Keysym::BackSpace => key = Some(Key::Backspace),
        Keysym::Tab => key = Some(Key::Tab),
        Keysym::Return => key = Some(Key::Return),
        Keysym::Escape => key = Some(Key::Escape),
        Keysym::BackTab => key = Some(Key::Backtab),
        Keysym::Delete => key = Some(Key::Delete),
        Keysym::Shift_L => key = Some(Key::Shift),
        Keysym::Shift_R => key = Some(Key::ShiftR),
        Keysym::Control_L => key = Some(Key::Control),
        Keysym::Control_R => key = Some(Key::ControlR),
        Keysym::Alt_L => key = Some(Key::Alt),
        Keysym::Alt_R => key = Some(Key::AltGr),
        Keysym::Caps_Lock => key = Some(Key::CapsLock),
        Keysym::Meta_L => key = Some(Key::Meta),
        Keysym::Meta_R => key = Some(Key::MetaR),
        Keysym::space => key = Some(Key::Space),
        Keysym::Up | Keysym::uparrow => key = Some(Key::UpArrow),
        Keysym::Down | Keysym::downarrow => key = Some(Key::DownArrow),
        Keysym::Left | Keysym::leftarrow => key = Some(Key::LeftArrow),
        Keysym::Right | Keysym::rightarrow => key = Some(Key::RightArrow),
        Keysym::F1 => key = Some(Key::F1),
        Keysym::F2 => key = Some(Key::F2),
        Keysym::F3 => key = Some(Key::F3),
        Keysym::F4 => key = Some(Key::F4),
        Keysym::F5 => key = Some(Key::F5),
        Keysym::F6 => key = Some(Key::F6),
        Keysym::F7 => key = Some(Key::F7),
        Keysym::F8 => key = Some(Key::F8),
        Keysym::F9 => key = Some(Key::F9),
        Keysym::F10 => key = Some(Key::F10),
        Keysym::F11 => key = Some(Key::F11),
        Keysym::F12 => key = Some(Key::F12),
        Keysym::F13 => key = Some(Key::F13),
        Keysym::F14 => key = Some(Key::F14),
        Keysym::F15 => key = Some(Key::F15),
        Keysym::F16 => key = Some(Key::F16),
        Keysym::F17 => key = Some(Key::F17),
        Keysym::F18 => key = Some(Key::F18),
        Keysym::F19 => key = Some(Key::F19),
        Keysym::F20 => key = Some(Key::F20),
        Keysym::F21 => key = Some(Key::F21),
        Keysym::F22 => key = Some(Key::F22),
        Keysym::F23 => key = Some(Key::F23),
        Keysym::F24 => key = Some(Key::F24),
        Keysym::Insert => key = Some(Key::Insert),
        Keysym::Home => key = Some(Key::Home),
        Keysym::End => key = Some(Key::End),
        Keysym::Page_Up => key = Some(Key::PageUp),
        Keysym::Page_Down => key = Some(Key::PageDown),
        Keysym::Scroll_Lock => key = Some(Key::ScrollLock),
        Keysym::Pause => key = Some(Key::Pause),
        Keysym::Sys_Req => key = Some(Key::SysReq),
        Keysym::XF86_Stop => key = Some(Key::Stop),
        Keysym::Menu => key = Some(Key::Menu),
        _ => {}
    }

    if let Some(key) = key {
        key.into()
    } else {
        SharedString::from(event.utf8.unwrap_or_default())
    }
}

#[derive(Debug)]
pub(crate) struct PointerState {
    pub pointer: Option<wl_pointer::WlPointer>,
    pub cursor_shape: CursorShapeManager,
    pub current_wayland_cursor: MouseCursor,
    pub last_cursor_enter_serial: Option<u32>,
}

impl PointerState {
    pub fn update_cursor(&mut self, mouse_cursor: MouseCursor, queue: &QueueHandle<WaylandWindow>) {
        if self.last_cursor_enter_serial.is_some()
            && self.pointer.is_some()
            && mouse_cursor != self.current_wayland_cursor
        {
            let pointer = self.pointer.as_ref().unwrap();
            let serial = self.last_cursor_enter_serial.unwrap();

            if mouse_cursor == MouseCursor::None {
                pointer.set_cursor(serial, None, 0, 0);
            } else {
                self.cursor_shape
                    .get_shape_device(pointer, queue)
                    .set_shape(
                        serial,
                        slint_to_wl_cursor_mapping::mouse_cursor_to_shape(mouse_cursor),
                    );
            }
            self.current_wayland_cursor = mouse_cursor;
        }
    }
}

fn drain_slint_events(data: &mut WaylandWindow) {
    let proxy = data.adapter.slint_event_proxy.clone();
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

pub(crate) fn set_event_sources(
    event_loop: &EventLoop<'static, WaylandWindow>,
    eventfd_fd: RawFd,
) {
    event_loop
        .handle()
        .insert_source(
            Generic::new(unsafe { BorrowedFd::borrow_raw(eventfd_fd) }, Interest::READ, Mode::Level),
            move |_, _, data| {
                let mut buf = [0u8; 8];
                unsafe {
                    libc::read(eventfd_fd, buf.as_mut_ptr() as *mut libc::c_void, 8);
                }
                drain_slint_events(data);
                Ok(PostAction::Continue)
            },
        )
        .unwrap();
}
