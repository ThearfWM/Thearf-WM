//! Move grab is the state of a composer during which the client window is being dragged around.
//!
//! eg. Usually whenever a user clicks on the app's titlebar and starts dragging, the compositors
//! enters a MoveSurfaceGrab state.

use crate::Thearf;
use smithay::{
    desktop::Window,
    input::pointer::{
        AxisFrame, ButtonEvent, GestureHoldBeginEvent, GestureHoldEndEvent, GesturePinchBeginEvent,
        GesturePinchEndEvent, GesturePinchUpdateEvent, GestureSwipeBeginEvent, GestureSwipeEndEvent,
        GestureSwipeUpdateEvent, GrabStartData as PointerGrabStartData, MotionEvent, PointerGrab,
        PointerInnerHandle, RelativeMotionEvent,
    },
    reexports::wayland_server::protocol::wl_surface::WlSurface,
    utils::{Logical, Point},
};

fn get_win_num(y: i32, windows: &Vec<Window>, screen_res_y: i32) -> usize {
    let stack_count = windows.len() - 1;

    let win_height = screen_res_y as f32 / stack_count as f32;

    let slot = ((y as f32) / win_height)
        .floor()
        .clamp(0.0, (stack_count) as f32) as usize;

    (slot + 1) as usize
}
pub struct MoveSurfaceGrab {
    pub start_data: PointerGrabStartData<Thearf>,
    pub window: Window,
    pub initial_window_location: Point<i32, Logical>,
}

impl PointerGrab<Thearf> for MoveSurfaceGrab {
    fn motion(
        &mut self,
        data: &mut Thearf,
        handle: &mut PointerInnerHandle<'_, Thearf>,
        _focus: Option<(WlSurface, Point<f64, Logical>)>,
        event: &MotionEvent,
    ) {
        // While the grab is active, no client has pointer focus
        handle.motion(data, None, event);

        let delta = event.location - self.start_data.location;
        let new_location = self.initial_window_location.to_f64() + delta;
        data.space
            .map_element(self.window.clone(), new_location.to_i32_round(), true);
    }

    fn relative_motion(
        &mut self,
        data: &mut Thearf,
        handle: &mut PointerInnerHandle<'_, Thearf>,
        focus: Option<(WlSurface, Point<f64, Logical>)>,
        event: &RelativeMotionEvent,
    ) {
        handle.relative_motion(data, focus, event);
    }

    fn button(
        &mut self,
        data: &mut Thearf,
        handle: &mut PointerInnerHandle<'_, Thearf>,
        event: &ButtonEvent,
    ) {
        handle.button(data, event);

        // The button is a button code as defined in the
        // Linux kernel's linux/input-event-codes.h header file, e.g. BTN_LEFT.
        const BTN_LEFT: u32 = 0x110;

        if !handle.current_pressed().contains(&BTN_LEFT) {
            // No more buttons are pressed, release the grab.
            handle.unset_grab(self, data, event.serial, event.time, true);
            let final_loc: Point<i32, Logical> = data.space.element_location(&self.window).unwrap();
            let output = data.space.outputs().next().unwrap();
            let screen_geo = data.space.output_geometry(output).unwrap();


            if final_loc.x < screen_geo.loc.x+(screen_geo.size.w/4) {
                let index = data.win_order.iter().position(|val| *val == self.window);

                data.win_order[index.unwrap()] = data.win_order[0].clone();
                data.win_order[0] = self.window.clone();
                Thearf::retile(data);
            } else {
                let swap_idx = get_win_num(final_loc.y, &data.win_order, screen_geo.size.h);
                let index = data.win_order.iter().position(|val| *val == self.window).unwrap();

                data.win_order[index] = data.win_order[swap_idx].clone();
                data.win_order[swap_idx] = self.window.clone();
                Thearf::retile(data);
            }
        }
    }

    fn axis(
        &mut self,
        data: &mut Thearf,
        handle: &mut PointerInnerHandle<'_, Thearf>,
        details: AxisFrame,
    ) {
        handle.axis(data, details)
    }

    fn frame(&mut self, data: &mut Thearf, handle: &mut PointerInnerHandle<'_, Thearf>) {
        handle.frame(data);
    }

    fn gesture_swipe_begin(
        &mut self,
        data: &mut Thearf,
        handle: &mut PointerInnerHandle<'_, Thearf>,
        event: &GestureSwipeBeginEvent,
    ) {
        handle.gesture_swipe_begin(data, event)
    }

    fn gesture_swipe_update(
        &mut self,
        data: &mut Thearf,
        handle: &mut PointerInnerHandle<'_, Thearf>,
        event: &GestureSwipeUpdateEvent,
    ) {
        handle.gesture_swipe_update(data, event)
    }

    fn gesture_swipe_end(
        &mut self,
        data: &mut Thearf,
        handle: &mut PointerInnerHandle<'_, Thearf>,
        event: &GestureSwipeEndEvent,
    ) {
        handle.gesture_swipe_end(data, event)
    }

    fn gesture_pinch_begin(
        &mut self,
        data: &mut Thearf,
        handle: &mut PointerInnerHandle<'_, Thearf>,
        event: &GesturePinchBeginEvent,
    ) {
        handle.gesture_pinch_begin(data, event)
    }

    fn gesture_pinch_update(
        &mut self,
        data: &mut Thearf,
        handle: &mut PointerInnerHandle<'_, Thearf>,
        event: &GesturePinchUpdateEvent,
    ) {
        handle.gesture_pinch_update(data, event)
    }

    fn gesture_pinch_end(
        &mut self,
        data: &mut Thearf,
        handle: &mut PointerInnerHandle<'_, Thearf>,
        event: &GesturePinchEndEvent,
    ) {
        handle.gesture_pinch_end(data, event)
    }

    fn gesture_hold_begin(
        &mut self,
        data: &mut Thearf,
        handle: &mut PointerInnerHandle<'_, Thearf>,
        event: &GestureHoldBeginEvent,
    ) {
        handle.gesture_hold_begin(data, event)
    }

    fn gesture_hold_end(
        &mut self,
        data: &mut Thearf,
        handle: &mut PointerInnerHandle<'_, Thearf>,
        event: &GestureHoldEndEvent,
    ) {
        handle.gesture_hold_end(data, event)
    }

    fn start_data(&self) -> &PointerGrabStartData<Thearf> {
        &self.start_data
    }

    fn unset(&mut self, _data: &mut Thearf) {}
}
