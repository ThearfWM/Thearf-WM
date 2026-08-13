use std::{ffi::OsString, sync::Arc, env::var, path::PathBuf};

use smithay::{
    desktop::{PopupManager, Space, Window, WindowSurfaceType, find_popup_root_surface},
    input::{Seat, SeatState},
    reexports::{
        calloop::{EventLoop, Interest, LoopSignal, Mode, PostAction, generic::Generic},
        wayland_server::{
            Display, DisplayHandle,
            backend::{ClientData, ClientId, DisconnectReason},
            protocol::wl_surface::WlSurface,
        },
    },
    utils::{Logical, Point, Size},
    wayland::{
        compositor::{CompositorClientState, CompositorState},
        output::OutputManagerState,
        selection::data_device::DataDeviceState,
        shell::xdg::XdgShellState,
        shm::ShmState,
        socket::ListeningSocketSource,
    },
};


pub struct Drm {
    pub session: Session,
    pub primary_gpu: DrmNode,
    pub all_gpus: GpuManager,
}

impl Drm {
    pub fn device_added(node: DrmNode, path: PathBuf) {
        None;
    }
}

pub struct Thearf {

    // Things
    pub socket_name: OsString,

    // Looping and Window Management
    pub space: Space<Window>,
    pub loop_signal: LoopSignal,

    // Thearf State
    pub compositor_state: CompositorState,
    pub xdg_shell_state: XdgShellState,
    pub shm_state: ShmState,
    pub output_manager_state: OutputManagerState,
    pub seat_state: SeatState<Thearf>,
    pub data_device_state: DataDeviceState,
    pub popups: PopupManager,
    pub display: Display,

    // Seat
    pub seat: Seat<Self>,

    // Things I added
    pub win_order: Vec<Window>,
    pub cfg_file: String,

    // Drm
    pub drm: Drm
}

impl Thearf {
    pub fn new(event_loop: &mut EventLoop<Self>, display: Display<Self>) -> Self {
        let start_time = std::time::Instant::now();

        // Here we initialize implementations of some wayland protocols
        // Some of them require us to implement traits on the Thearf state,
        // you can find those implementations in the `crate::handlers` module

        // Initialize protocols needed for displaying windows
        let compositor_state = CompositorState::new::<Self>(&dh);
        let xdg_shell_state = XdgShellState::new::<Self>(&dh);
        let shm_state = ShmState::new::<Self>(&dh, vec![]);
        let popups = PopupManager::default();

        let output_manager_state = OutputManagerState::new_with_xdg_output::<Self>(&dh);

        // Data device is responsible for clipboard and drag-and-drop
        let data_device_state = DataDeviceState::new::<Self>(&dh);

        // A seat is a group of keyboards, pointer and touch devices.
        // A seat typically has a pointer and maintains a keyboard focus and a pointer focus.
        let mut seat_state = SeatState::new();
        let mut seat: Seat<Self> = seat_state.new_wl_seat(&dh, "winit");

        // Notify clients that we have a keyboard, for the sake of the example we assume that keyboard is always present.
        // You may want to track keyboard hot-plug in real compositor.
        seat.add_keyboard(Default::default(), 200, 25).unwrap();

        // Notify clients that we have a pointer (mouse)
        // Here we assume that there is always pointer plugged in
        seat.add_pointer();

        // A space represents a two-dimensional plane. Windows and Outputs can be mapped onto it.
        //
        // Windows get a position and stacking order through mapping.
        // Outputs become views of a part of the Space and can be rendered via Space::render_output.
        let space = Space::default();

        // Setup a wayland socket that will be used to accept clients
        let socket_name = Self::init_wayland_listener(display, event_loop);

        // Get the loop signal, used to stop the event loop
        let loop_signal = event_loop.get_signal();


        let win_order = Vec::new();

        let home = std::env::var("HOME").expect("HOME not set.");
        let cfg_file = format!("{home}/.config/thearf/config").to_string();

        let session = LibSeatSession::new();
        
        let drm = None;
        
        Self {
            socket_name,
            space,
            loop_signal,
            compositor_state,
            xdg_shell_state,
            shm_state,
            output_manager_state,
            seat_state,
            data_device_state,
            popups,
            display,
            seat,
            win_order,
            cfg_file,
            drm,
        }
    }

    pub fn retile(&mut self) {

        let mut line_buffer: i32 = 6;
        for line in std::fs::read_to_string(&self.cfg_file).unwrap().lines() {
            if line.chars().nth(0) == Some('#') ||  line.chars().nth(0) == Some('/')  {}
            else {
                let words = line.split_whitespace().collect::<Vec<&str>>();
                if words.len() == 0 {}
                else {
                    if words[0] == "window_buffer" {
                        line_buffer = line.split_whitespace().collect::<Vec<&str>>()[2].to_string().parse().unwrap();
                    }
                }
            }
        }

        let output = self.space.outputs().next().unwrap();
        let screen_geo = self.space.output_geometry(output).unwrap();

        let win_count = self.win_order.len();

        for win_num in 0..win_count {
            let win = self.win_order.get(win_num as usize).unwrap();
            let mut win_geo = self.space.element_geometry(&win).unwrap();

            let mut posx = screen_geo.loc.x;
            let mut posy = screen_geo.loc.y;

            if win_count == 1 {
                win_geo.size.w = screen_geo.size.w - line_buffer;     
                win_geo.size.h = screen_geo.size.h - line_buffer;
                posy += line_buffer/2;
                posx += line_buffer/2;
            } else if win_num == 0 {
                win_geo.size.w = (screen_geo.size.w / 2) - (line_buffer as f32*1.5) as i32;
                win_geo.size.h = screen_geo.size.h - line_buffer;
                posy += line_buffer/2;
                posx += line_buffer/2;
            } else {
                win_geo.size.w = (screen_geo.size.w / 2) - line_buffer/2;
                win_geo.size.h = (screen_geo.size.h / (win_count as i32 - 1)) - line_buffer;
                posx += screen_geo.size.w/2;
                posy += (screen_geo.size.h/(win_count as i32 - 1))*((win_num-1) as i32) + line_buffer/2;
            }

            win.toplevel().unwrap().with_pending_state(|state| {
                state.size = Some(Size::new(win_geo.size.w, win_geo.size.h));
            });
            win.toplevel().unwrap().send_configure();

            self.space.map_element(win.clone(), (posx, posy), false);
        }
    }

    fn init_wayland_listener(display: Display<Thearf>, event_loop: &mut EventLoop<Self>) -> OsString {
        // Creates a new listening socket, automatically choosing the next available `wayland` socket name.
        let listening_socket = ListeningSocketSource::new_auto().unwrap();

        // Get the name of the listening socket.
        // Clients will connect to this socket.
        let socket_name = listening_socket.socket_name().to_os_string();

        let loop_handle = event_loop.handle();

        loop_handle
            .insert_source(listening_socket, move |client_stream, _, state| {
                // Inside the callback, you should insert the client into the display.
                //
                // You may also associate some data with the client when inserting the client.
                state
                    .display_handle
                    .insert_client(client_stream, Arc::new(ClientState::default()))
                    .unwrap();
            })
            .expect("Failed to init the wayland event source.");

        // You also need to add the display itself to the event loop, so that client events will be processed by wayland-server.
        loop_handle
            .insert_source(
                Generic::new(display, Interest::READ, Mode::Level),
                |_, display, state| {
                    // Safety: we don't drop the display
                    unsafe {
                        display.get_mut().dispatch_clients(state).unwrap();
                    }
                    Ok(PostAction::Continue)
                },
            )
            .unwrap();

        socket_name
    }

    pub fn surface_under(&self, pos: Point<f64, Logical>) -> Option<(WlSurface, Point<f64, Logical>)> {
        self.space.element_under(pos).and_then(|(window, location)| {
            window
                .surface_under(pos - location.to_f64(), WindowSurfaceType::ALL)
                .map(|(s, p)| (s, (p + location).to_f64()))
        })
    }
}

/// Data associated with a wayland client that connects to Thearf.
/// One instance of this type per client.
#[derive(Default)]
pub struct ClientState {
    pub compositor_state: CompositorClientState,
}

impl ClientData for ClientState {
    fn initialized(&self, _client_id: ClientId) {}
    fn disconnected(&self, _client_id: ClientId, _reason: DisconnectReason) {}
}