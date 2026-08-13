use smithay::{
    backend::input::{
        AbsolutePositionEvent, Axis, AxisSource, ButtonState, Event, InputBackend, InputEvent, 
        KeyState, KeyboardKeyEvent, PointerAxisEvent, PointerButtonEvent,
    },
    input::{
        keyboard::{FilterResult, ModifiersState, Keysym},
        pointer::{AxisFrame, ButtonEvent, MotionEvent},
    },
    reexports::wayland_server::protocol::wl_surface::WlSurface,
    utils::SERIAL_COUNTER,
};
use std::{
    env::var, process::Command
};

use xkbcommon::xkb::{self, KEYSYM_NO_FLAGS};
use smithay::desktop::{Space, Window};

use crate::state::Thearf;

fn bind_to_cmd(func: String, thearf: &mut Thearf) {
    let window = thearf.space.elements().last();
    if window.is_none() {
        return;
    }

    match func.as_str() {
        "close_win" => {
            if thearf.win_order.len() == 0 {}
            else {
                window.unwrap().toplevel().unwrap().send_close(); 
                Thearf::retile(thearf);
            }
        },
        "refresh" => { Thearf::retile(thearf);
        },
        _ => {
            // This just handles the error so that if there is an invalid function it doesn't crash the WM
            if let Err(err) = std::process::Command::new(func.as_str()).spawn() {
                eprintln!("Command failed {func}: {err}");
            }
            Thearf::retile(thearf);
        },
    }
}

fn keysym_press_check(keysym: Keysym, check: &String) -> u8 {
    let target = xkb::keysym_from_name(check, xkb::KEYSYM_NO_FLAGS);
    if keysym == target {
        return 1;
    }
    0
}


fn name_to_press(key_check: &String, modifiers: &ModifiersState) -> u8 {
    let held = match key_check.as_str() {
        "ctrl" => modifiers.ctrl,
        "alt" => modifiers.alt,
        "shift" => modifiers.shift,
        "super" => modifiers.logo,
        "caps_lock" => modifiers.caps_lock,
        _ => false,
    };
    if held { 1 } else { 0 }
}

// keys.0 is the check if whether a key is pressed or not
// keys.1 are keys that need to be pressed for the action
// keys.2 are the modifiers that will be used
    /*
    Modifiers:
    1. ctrl
    2. alt
    3. meta
    4. shift
    5. caps_lock
    */
fn check_keys(keys: (KeyState, Vec<String>, ModifiersState), keysym: &Keysym) -> bool {
    let mut pressed: u8 = 0;

    if !(keys.0 == KeyState::Pressed) {
        return false
    }

    for key in &(keys.1) {
        pressed += name_to_press(key, &keys.2);
    }
    
    pressed += keysym_press_check(*keysym, &keys.1[keys.1.len()-1]);

    if &(pressed as usize) == &(keys.1).len() {
        return true;
    }
    false
}

fn get_keys_from_file(manager: &mut Thearf, keystate: KeyState, modifiers: &ModifiersState, keysym: &Keysym) -> u8 {
    let data = &manager.cfg_file;
    let mut successes: u8 = 0;
    for line in std::fs::read_to_string(data).unwrap().lines() {
        if !(line.chars().nth(0) == Some('#') || line.chars().nth(0) == Some('/')) && !(line.chars().collect::<Vec<char>>().len() == 0) {

            let words: Vec<&str> = line.split_whitespace().collect();
            let params: Vec<String> = words[1..]
                .iter()
                .filter(|s| **s != "=")
                .map(|string| string.to_string())
                .collect();
            let func_true = check_keys((keystate, params, *modifiers), keysym);

            if func_true == true {
                bind_to_cmd(words[0].to_string(), manager);
                successes += 1
            }
        }
    }
    successes
}


impl Thearf {
    pub fn process_input_event<I: InputBackend>(&mut self, event: InputEvent<I>) {
        match event {
            InputEvent::Keyboard { event, .. } => {
                let serial = SERIAL_COUNTER.next_serial();
                let time = Event::time_msec(&event);
                let keystate = event.state();

                self.seat.get_keyboard().unwrap().input::<(), _>(
                    self,
                    event.key_code(),
                    keystate,
                    serial,
                    time,
                    |data, modifiers, keysym| {
                        if get_keys_from_file(data, keystate, modifiers, &keysym.raw_latin_sym_or_raw_current_sym().unwrap()) > 0 {
                            return FilterResult::Intercept(())
                        }
                        FilterResult::Forward
                    }
                );
            }
            InputEvent::PointerMotion { .. } => {}
            InputEvent::PointerMotionAbsolute { event, .. } => {
                let output = self.space.outputs().next().unwrap();

                let output_geo = self.space.output_geometry(output).unwrap();

                let pos = event.position_transformed(output_geo.size) + output_geo.loc.to_f64();

                let serial = SERIAL_COUNTER.next_serial();

                let pointer = self.seat.get_pointer().unwrap();

                let under = self.surface_under(pos);

                pointer.motion(
                    self,
                    under,
                    &MotionEvent {
                        location: pos,
                        serial,
                        time: event.time_msec(),
                    },
                );
                pointer.frame(self);
            }
            InputEvent::PointerButton { event, .. } => {
                let pointer = self.seat.get_pointer().unwrap();
                let keyboard = self.seat.get_keyboard().unwrap();

                let serial = SERIAL_COUNTER.next_serial();

                let button = event.button_code();

                let button_state = event.state();

                if ButtonState::Pressed == button_state && !pointer.is_grabbed() {
                    if let Some((window, _loc)) = self
                        .space
                        .element_under(pointer.current_location())
                        .map(|(w, l)| (w.clone(), l))
                    {
                        self.space.raise_element(&window, true);
                        keyboard.set_focus(
                            self,
                            Some(window.toplevel().unwrap().wl_surface().clone()),
                            serial,
                        );
                        self.space.elements().for_each(|window| {
                            window.toplevel().unwrap().send_pending_configure();
                        });
                    } else {
                        self.space.elements().for_each(|window| {
                            window.set_activated(false);
                            window.toplevel().unwrap().send_pending_configure();
                        });
                        keyboard.set_focus(self, Option::<WlSurface>::None, serial);
                    }
                };

                pointer.button(
                    self,
                    &ButtonEvent {
                        button,
                        state: button_state,
                        serial,
                        time: event.time_msec(),
                    },
                );
                pointer.frame(self);
            }
            InputEvent::PointerAxis { event, .. } => {
                let source = event.source();

                let horizontal_amount = event
                    .amount(Axis::Horizontal)
                    .unwrap_or_else(|| event.amount_v120(Axis::Horizontal).unwrap_or(0.0) * 15.0 / 120.);
                let vertical_amount = event
                    .amount(Axis::Vertical)
                    .unwrap_or_else(|| event.amount_v120(Axis::Vertical).unwrap_or(0.0) * 15.0 / 120.);
                let horizontal_amount_discrete = event.amount_v120(Axis::Horizontal);
                let vertical_amount_discrete = event.amount_v120(Axis::Vertical);

                let mut frame = AxisFrame::new(event.time_msec()).source(source);
                if horizontal_amount != 0.0 {
                    frame = frame.value(Axis::Horizontal, horizontal_amount);
                    if let Some(discrete) = horizontal_amount_discrete {
                        frame = frame.v120(Axis::Horizontal, discrete as i32);
                    }
                }
                if vertical_amount != 0.0 {
                    frame = frame.value(Axis::Vertical, vertical_amount);
                    if let Some(discrete) = vertical_amount_discrete {
                        frame = frame.v120(Axis::Vertical, discrete as i32);
                    }
                }

                if source == AxisSource::Finger {
                    if event.amount(Axis::Horizontal) == Some(0.0) {
                        frame = frame.stop(Axis::Horizontal);
                    }
                    if event.amount(Axis::Vertical) == Some(0.0) {
                        frame = frame.stop(Axis::Vertical);
                    }
                }

                let pointer = self.seat.get_pointer().unwrap();
                pointer.axis(self, frame);
                pointer.frame(self);
            }
            _ => {}
        }
    }
}
