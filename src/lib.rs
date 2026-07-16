pub mod configure;
mod event_macros;
mod skia_non_docs;
pub mod slint_adapter;
pub mod wayland_adapter;

pub mod layer_properties {
    pub use crate::configure::{WindowConf, WindowConfBuilder};
    pub use smithay_client_toolkit::shell::wlr_layer::Anchor as LayerAnchor;
    pub use smithay_client_toolkit::shell::wlr_layer::KeyboardInteractivity as BoardType;
    pub use smithay_client_toolkit::shell::wlr_layer::Layer as LayerType;
}

pub mod macro_internal {
    pub use log::{info, warn};
    pub use paste::paste;
    pub use smithay_client_toolkit::reexports::calloop::{
        Interest, Mode, PostAction, generic::Generic,
    };
}

use std::error::Error;
use std::time::Duration;

pub trait WindowHandler: std::fmt::Debug {
    fn on_call(&mut self) -> Result<(), Box<dyn Error>>;

    fn get_span(&self) -> String {
        String::from("unnamed-widget")
    }

    fn is_locked(&self) -> bool {
        true
    }
}

pub fn run_event_loop(mut windows: Vec<Box<dyn WindowHandler>>) -> Result<(), Box<dyn Error>> {
    loop {
        for win in windows.iter_mut() {
            let _span = win.get_span();
            win.on_call()?;
        }
        std::thread::sleep(Duration::from_millis(16));
    }
}
