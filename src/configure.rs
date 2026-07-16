use smithay_client_toolkit::shell::wlr_layer::{Anchor, KeyboardInteractivity, Layer};
use std::cell::Cell;

#[derive(Debug, Clone)]
pub struct WindowConf {
    pub width: u32,
    pub height: u32,
    pub anchor: [Option<Anchor>; 4],
    pub margin: (i32, i32, i32, i32),
    pub layer_type: Layer,
    pub board_interactivity: Cell<KeyboardInteractivity>,
    pub exclusive_zone: Option<i32>,
    pub monitor_name: Option<String>,
    pub natural_scroll: bool,
}

impl WindowConf {
    pub fn builder() -> WindowConfBuilder {
        WindowConfBuilder::default()
    }
}

#[derive(Default)]
pub struct WindowConfBuilder {
    max_width: u32,
    max_height: u32,
    anchor: [Option<Anchor>; 4],
    margin: (i32, i32, i32, i32),
    layer_type: Option<Layer>,
    board_interactivity: KeyboardInteractivity,
    exclusive_zone: Option<i32>,
    monitor_name: Option<String>,
    natural_scroll: bool,
}

impl WindowConfBuilder {
    pub fn width<I: Into<u32>>(&mut self, width: I) -> &mut Self {
        self.max_width = width.into();
        self
    }

    pub fn height<I: Into<u32>>(&mut self, height: I) -> &mut Self {
        self.max_height = height.into();
        self
    }

    pub fn anchor_1(&mut self, anchor: Anchor) -> &mut Self {
        self.anchor[0] = Some(anchor);
        self
    }

    pub fn anchor_2(&mut self, anchor: Anchor) -> &mut Self {
        self.anchor[1] = Some(anchor);
        self
    }

    pub fn anchor_3(&mut self, anchor: Anchor) -> &mut Self {
        self.anchor[2] = Some(anchor);
        self
    }

    pub fn anchor_4(&mut self, anchor: Anchor) -> &mut Self {
        self.anchor[3] = Some(anchor);
        self
    }

    pub fn margins(&mut self, top: i32, right: i32, bottom: i32, left: i32) -> &mut Self {
        self.margin = (top, right, bottom, left);
        self
    }

    pub fn layer_type(&mut self, layer: Layer) -> &mut Self {
        self.layer_type = Some(layer);
        self
    }

    pub fn board_interactivity(&mut self, board: KeyboardInteractivity) -> &mut Self {
        self.board_interactivity = board;
        self
    }

    pub fn exclusive_zone(&mut self, dimention: i32) -> &mut Self {
        self.exclusive_zone = Some(dimention);
        self
    }

    pub fn monitor(&mut self, name: String) -> &mut Self {
        self.monitor_name = Some(name);
        self
    }

    pub fn natural_scroll(&mut self, scroll: bool) -> &mut Self {
        self.natural_scroll = scroll;
        self
    }

    pub fn build(&self) -> Result<WindowConf, Box<dyn std::error::Error>> {
        Ok(WindowConf {
            width: if self.max_width != 0 {
                self.max_width
            } else {
                return Err("width is either not defined or set to zero".into());
            },
            height: if self.max_height != 0 {
                self.max_height
            } else {
                return Err("height is either not defined or set to zero".into());
            },
            anchor: self.anchor,
            margin: self.margin,
            layer_type: match self.layer_type {
                None => Layer::Top,
                Some(val) => val,
            },
            board_interactivity: Cell::new(self.board_interactivity),
            exclusive_zone: self.exclusive_zone,
            monitor_name: self.monitor_name.clone(),
            natural_scroll: self.natural_scroll,
        })
    }
}
