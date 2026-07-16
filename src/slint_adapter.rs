use slint::platform::{EventLoopProxy, Platform, WindowAdapter};
use std::{
    cell::RefCell,
    os::unix::io::RawFd,
    rc::Rc,
    sync::{Arc, Mutex},
};

use crate::skia_non_docs::SkiaWindowAdapter;

thread_local! {
    pub(crate) static ADAPTERS: RefCell<Vec<Rc<SkiaWindowAdapter>>> = const { RefCell::new(Vec::new()) };
}

pub struct SlintPlatform;

impl Default for SlintPlatform {
    fn default() -> Self {
        SlintPlatform
    }
}

impl Platform for SlintPlatform {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, slint::PlatformError> {
        let adapter = ADAPTERS.with(|v| v.borrow().last().unwrap().clone());
        Ok(adapter)
    }

    fn debug_log(&self, arguments: core::fmt::Arguments) {
        if let Some(val) = arguments.as_str() {
            log::info!("{val}");
        } else {
            log::info!("{}", arguments.to_string());
        }
    }

    fn new_event_loop_proxy(&self) -> Option<Box<dyn EventLoopProxy>> {
        Some(Box::new(SlintEventProxy(
            ADAPTERS.with(|v| v.borrow().last().unwrap().slint_event_proxy.clone()),
            ADAPTERS.with(|v| v.borrow().last().unwrap().eventfd),
        )))
    }
}

#[allow(clippy::type_complexity)]
struct SlintEventProxy(Arc<Mutex<Vec<Box<dyn FnOnce() + Send>>>>, RawFd);

impl EventLoopProxy for SlintEventProxy {
    fn quit_event_loop(&self) -> Result<(), i_slint_core::api::EventLoopError> {
        Ok(())
    }

    fn invoke_from_event_loop(
        &self,
        event: Box<dyn FnOnce() + Send>,
    ) -> Result<(), i_slint_core::api::EventLoopError> {
        if let Ok(mut list_of_event) = self.0.try_lock() {
            (*list_of_event).push(event);
            drop(list_of_event);
            if self.1 != -1 {
                let val: u64 = 1;
                unsafe {
                    libc::write(self.1, &val as *const u64 as *const libc::c_void, 8);
                }
            }
        } else {
            log::warn!("Slint proxy event could not be processed");
        }
        Ok(())
    }
}
