#[macro_export]
macro_rules! windows {
    ($($slint_win:ty),+) => {
        use $crate::wayland_adapter::{WinHandle, WaylandWindow};
        #[allow(unused_imports)]
        use std::io::Write;
        $crate::macro_internal::paste! {
            $(
                struct [<$slint_win Wl>] {
                    ui: $slint_win ,
                    way: WaylandWindow,
                }

                impl std::fmt::Debug for [<$slint_win Wl>] {
                    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                        f.debug_struct("WlWindow")
                        .field("wayland_side:", &self.way)
                        .finish()
                    }
                }

                impl [<$slint_win Wl>] {
                    pub fn spawn(name: &str, window_conf: $crate::configure::WindowConf) -> Self {
                        let way_win = WaylandWindow::spawn(name, window_conf);
                        [<$slint_win Wl>] {
                            ui: $slint_win::new().unwrap(),
                            way: way_win
                        }
                    }

                    pub fn hide(&self) {
                        self.way.hide();
                    }

                    pub fn show_again(&mut self) {
                        self.way.show_again();
                    }

                    pub fn toggle(&mut self) {
                        self.way.toggle();
                    }

                    pub fn grab_focus(&self) {
                        self.way.grab_focus();
                    }

                    pub fn remove_focus(&self) {
                        self.way.remove_focus();
                    }

                    pub fn add_input_region(&self, x: i32, y: i32, width: i32, height: i32) {
                        self.way.add_input_region(x, y, width, height);
                    }

                    pub fn subtract_input_region(&self, x: i32, y: i32, width: i32, height: i32) {
                        self.way.subtract_input_region(x, y, width, height);
                    }

                    pub fn add_opaque_region(&self, x: i32, y: i32, width: i32, height: i32) {
                        self.way.add_opaque_region(x, y, width, height);
                    }

                    pub fn subtract_opaque_region(&self, x: i32, y: i32, width: i32, height: i32) {
                        self.way.subtract_opaque_region(x, y, width, height);
                    }

                    pub fn set_exclusive_zone(&mut self, val: i32) {
                        self.way.set_exclusive_zone(val);
                    }

                    pub fn get_handler(&self) -> WinHandle {
                        WinHandle(self.way.loop_handle.clone())
                    }

                    pub fn parts(self) -> ($slint_win, WaylandWindow) {
                        let [<$slint_win Wl>] { ui, way } = self;
                        (ui, way)
                    }
                }

                impl $crate::WindowHandler for [<$slint_win Wl>] {
                    fn on_call(
                        &mut self,
                    ) -> Result<(), Box<dyn std::error::Error>> {
                        let event_loop = self.way.event_loop.clone();
                        event_loop
                            .borrow_mut()
                            .dispatch(Some(std::time::Duration::ZERO), &mut self.way)?;
                        Ok(())
                    }

                    fn get_span(&self) -> String {
                        self.way.span.clone()
                    }
                }

                impl std::ops::Deref for [<$slint_win Wl>] {
                    type Target = [<$slint_win>];
                    fn deref(&self) -> &Self::Target {
                        &self.ui
                    }
                }
            )+
        }
    };
}

#[macro_export]
macro_rules! run_windows {
    (
        windows: [ $($entry:tt),+ $(,)? ]
        $(,)?
    ) => {{
        let mut windows = Vec::new();
        $(
            windows.push(Box::new($entry) as Box<dyn $crate::WindowHandler>);
        )+
        $crate::run_event_loop(windows)
    }};
}
