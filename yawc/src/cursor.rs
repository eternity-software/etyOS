#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum CursorShape {
    #[default]
    Default,
    Move,
    ColResize,
    RowResize,
    NeswResize,
    NwseResize,
}

impl CursorShape {
    pub fn to_cursor_icon(self) -> smithay::input::pointer::CursorIcon {
        use smithay::input::pointer::CursorIcon;

        match self {
            CursorShape::Default => CursorIcon::Default,
            CursorShape::Move => CursorIcon::Move,
            CursorShape::ColResize => CursorIcon::ColResize,
            CursorShape::RowResize => CursorIcon::RowResize,
            CursorShape::NeswResize => CursorIcon::NeswResize,
            CursorShape::NwseResize => CursorIcon::NwseResize,
        }
    }
}

#[cfg(feature = "winit-backend")]
impl CursorShape {
    pub fn to_winit(self) -> smithay::reexports::winit::window::CursorIcon {
        use smithay::reexports::winit::window::CursorIcon;

        match self {
            CursorShape::Default => CursorIcon::Default,
            CursorShape::Move => CursorIcon::Move,
            CursorShape::ColResize => CursorIcon::ColResize,
            CursorShape::RowResize => CursorIcon::RowResize,
            CursorShape::NeswResize => CursorIcon::NeswResize,
            CursorShape::NwseResize => CursorIcon::NwseResize,
        }
    }
}
