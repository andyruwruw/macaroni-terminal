//! Dragging the dividers between panes.
//!
//! Pressing a divider's handle (drawn by `view/panes.rs`) stores a [`Drag`]; while it's set the
//! subscription turns cursor moves into [`Message::DragMove`], which sets the split's ratio,
//! and the release clears it.

use iced::{Point, Task};
use macaroni_core::{Axis, Command, Divider, WorkspaceId};

use crate::message::Message;
use crate::state::Drag;
use crate::{App, view};

impl App {
    /// A divider was pressed: follow the pointer until the button is released.
    pub(crate) fn divider_press(
        &mut self,
        workspace: WorkspaceId,
        divider: Divider,
    ) -> Task<Message> {
        self.drag = Some(Drag { workspace, divider });
        Task::none()
    }

    /// Moves the dragged divider to the pointer, as a ratio of its split's area. The model
    /// clamps the ratio, so the cursor can leave the area.
    pub(crate) fn divider_move(&mut self, position: Point) -> Task<Message> {
        let Some(drag) = &self.drag else {
            return Task::none();
        };
        let a = drag.divider.area;
        // The divider's area is relative to the workspace area; cursor positions are window
        // coordinates. Banners above the panes aren't counted, as in drop targeting.
        let origin = view::workspace_origin(self.chrome);
        let ratio = match drag.divider.axis {
            Axis::Row => (position.x - origin.x - a.x) / a.width,
            Axis::Column => (position.y - origin.y - a.y) / a.height,
        };
        let command = Command::SetSplitRatio {
            workspace: drag.workspace,
            path: drag.divider.path.clone(),
            ratio,
        };
        self.apply(command)
    }
}
