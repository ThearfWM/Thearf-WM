#![allow(clippy::type_complexity)]

use std::{collections::HashMap, io, path::Path, time::Duration};

use smithay::{
    backend::{
        allocator::{Fourcc, Modifier, gbm::{GbmAllocator, GbmBufferFlags, GbmDevice}, format::FormatSet},
        drm::{CreateDrmNodeError, DrmDevice, DrmDeviceFd, DrmError, DrmEvent, DrmNode, NodeType,
            output::{DrmOutput, DrmOutputManager, DrmOutputRenderElements},
            exporter::gbm::GbmFramebufferExporter},
        egl::{self, EGLContext, EGLDevice, EGLDisplay, context::ContextPriority},
        input::InputEvent,
        libinput::{LibinputInputBackend, LibinputSessionInterface},
        renderer::{
            ImportAll, ImportDma, ImportEgl, ImportMemWl, Renderer,
            element::{AsRenderElements, RenderElementStates, surface::WaylandSurfaceRenderElement},
            gles::{Capability, GlesRenderer},
            multigpu::{GpuManager, MultiRenderer, gbm::GbmGlesBackend},
        },
        session::{Event as SessionEvent, Session, libseat::LibSeatSession},
        udev::{all_gpus, primary_gpu, UdevBackend, UdevEvent},
        SwapBuffersError,
    },
    desktop::{Space, Window},
    input::keyboard::LedState,
    output::{Mode as WlMode, Output, PhysicalProperties},
    reexports::{
        calloop::{EventLoop, LoopHandle, RegistrationToken, timer::{TimeoutAction, Timer}},
        drm::{Device as _, control::{Device, ModeTypeFlags, connector, crtc}},
        input::{DeviceCapability, Libinput},
        rustix::fs::OFlags,
        wayland_server::{DisplayHandle, backend::GlobalId, protocol::wl_surface::WlSurface},
    },
    utils::{DeviceFd, Logical, Point, Scale, Transform},
    wayland::{compositor, dmabuf::{DmabufFeedbackBuilder, DmabufGlobal, DmabufHandler, DmabufState, ImportNotifier}, drm_syncobj::{DrmSyncobjHandler, DrmSyncobjState, supports_syncobj_eventfd}},
};
use smithay_drm_extras::{display_info, drm_scanner::{DrmScanEvent, DrmScanner}};
use tracing::{error, info, warn};

pub type GpuBackend = GbmGlesBackend<GlesRenderer, DrmDeviceFd>;
pub type RendererType<'a> = MultiRenderer<'a, 'a, GpuBackend, GpuBackend>;
pub type Allocator = GbmAllocator<DrmDeviceFd>;
pub type Exporter = GbmFramebufferExporter<DrmDeviceFd>;
pub type OutputType = DrmOutput<Allocator, Exporter, (), DrmDeviceFd>;


smithay::backend::renderer::element::render_elements! {
    pub DrmRenderElements<R> where R: ImportAll;
    Window = WaylandSurfaceRenderElement<R>,
}

#[derive(Debug, PartialEq)]
struct OutputId { node: DrmNode, crtc: crtc::Handle }

struct SurfaceData {
    output: Output,
    global: GlobalId,
    render_node: DrmNode,
    drm_output: OutputType,
}

struct BackendData {
    manager: DrmOutputManager<Allocator, Exporter, (), DrmDeviceFd>,
    scanner: DrmScanner,
    render_node: Option<DrmNode>,
    token: RegistrationToken,
    surfaces: HashMap<crtc::Handle, SurfaceData>,
}

#[derive(Debug, thiserror::Error)]
enum DeviceAddError {
    #[error("failed to open DRM device: {0}")]
    Open(#[source] smithay::backend::session::libseat::Error),
    #[error("failed to create DRM node: {0}")]
    Node(#[source] CreateDrmNodeError),
    #[error("failed to initialize DRM: {0}")]
    Drm(#[source] DrmError),
    #[error("failed to initialize GBM: {0}")]
    Gbm(#[source] std::io::Error),
    #[error("failed to initialize EGL/GPU: {0}")]
    Gpu(#[source] egl::Error),
    #[error("no usable primary GPU exists")]
    NoPrimaryGpu,
    #[error("failed to register DRM notifier: {0}")]
    Notifier(String),
}

pub struct Drm {
    pub session: LibSeatSession,
    pub primary_gpu: DrmNode,
    pub all_gpus: GpuManager<GpuBackend>,
    backends: HashMap<DrmNode, BackendData>,
    pub dmabuf_state: Option<(DmabufState, DmabufGlobal)>,
    pub syncobj_state: Option<DrmSyncobjState>,
}

impl Drm {
    pub fn new(session: LibSeatSession, primary_gpu: DrmNode, all_gpus: GpuManager<GpuBackend>) -> Self {
        Self { session, primary_gpu, all_gpus, backends: HashMap::new(), dmabuf_state: None, syncobj_state: None }
    }

    fn device_added(&mut self, node: DrmNode, path: &Path, handle: LoopHandle<'_, crate::Thearf>, state: &mut crate::Thearf) -> Result<(), DeviceAddError> {
        let fd = self.session.open(path, OFlags::RDWR | OFlags::CLOEXEC | OFlags::NOCTTY | OFlags::NONBLOCK).map_err(DeviceAddError::Open)?;
        let fd = DrmDeviceFd::new(DeviceFd::from(fd));
        let (drm, notifier) = DrmDevice::new(fd.clone(), true).map_err(DeviceAddError::Drm)?;
        let gbm = GbmDevice::new(fd).map_err(DeviceAddError::Gbm)?;

        let token = handle.insert_source(notifier, move |event, _, data| {
            match event {
                DrmEvent::VBlank(crtc) => {
                    let mut drm = data.drm.take().expect("DRM backend missing");
                    if let Some(backend) = drm.backends.get_mut(&node) {
                        if let Some(surface) = backend.surfaces.get_mut(&crtc) {
                            let _ = surface.drm_output.frame_submitted();
                        }
                    }
                    drm.render(node, crtc, data);
                    data.drm = Some(drm);
                },
                DrmEvent::Error(e) => error!(?e, "DRM event error"),
            }
        }).map_err(|e| DeviceAddError::Notifier(e.to_string()))?;

        let display = unsafe { EGLDisplay::new(gbm.clone()).map_err(DeviceAddError::Gpu)? };
        let egl_device = EGLDevice::device_for_display(&display).map_err(DeviceAddError::Gpu)?;
        let render_node = egl_device.try_get_render_node().ok().flatten().unwrap_or(node);
        self.all_gpus.as_mut().add_node(render_node, gbm.clone()).map_err(DeviceAddError::Gpu)?;

        let allocator = GbmAllocator::new(gbm.clone(), GbmBufferFlags::RENDERING | GbmBufferFlags::SCANOUT);
        let exporter = GbmFramebufferExporter::new(gbm.clone(), render_node.into());
        let mut renderer = self.all_gpus.single_renderer(&render_node).map_err(|_| DeviceAddError::NoPrimaryGpu)?;
        let render_formats = renderer.as_mut().egl_context().dmabuf_render_formats().iter().filter(|f| render_node != node || f.modifier == Modifier::Linear).copied().collect::<FormatSet>();
        let manager = DrmOutputManager::new(drm, allocator, exporter, Some(gbm), [Fourcc::Argb8888, Fourcc::Abgr8888], render_formats);

        self.backends.insert(node, BackendData { manager, scanner: DrmScanner::new(), render_node: Some(render_node), token, surfaces: HashMap::new() });
        self.device_changed(node, state);
        Ok(())
    }

    fn device_changed(&mut self, node: DrmNode, state: &mut crate::Thearf) {
        let Some(backend) = self.backends.get_mut(&node) else { return };
        let events = match backend.scanner.scan_connectors(backend.manager.device()) {
            Ok(events) => events,
            Err(e) => { warn!(?e, "DRM connector scan failed"); return; }
        };
        for event in events {
            match event {
                DrmScanEvent::Connected { connector, crtc: Some(crtc) } => self.connector_connected(node, connector, crtc, state),
                DrmScanEvent::Disconnected { connector, crtc: Some(crtc) } => self.connector_disconnected(node, connector, crtc, state),
                DrmScanEvent::Changed { .. } => {}
                _ => {}
            }
        }
    }

    fn connector_connected(&mut self, node: DrmNode, connector: connector::Info, crtc: crtc::Handle, state: &mut crate::Thearf) {
        let Some(backend) = self.backends.get_mut(&node) else { return };
        let render_node = backend.render_node.unwrap_or(self.primary_gpu);
        let Ok(mut renderer) = self.all_gpus.single_renderer(&render_node) else { return };
        if connector.modes().is_empty() { return; }
        let mode = *connector.modes().iter().find(|m| m.mode_type().contains(ModeTypeFlags::PREFERRED)).unwrap_or(&connector.modes()[0]);
        let wl_mode = WlMode::from(mode);
        let name = format!("{}-{}", connector.interface().as_str(), connector.interface_id());
        let info = display_info::for_connector(backend.manager.device(), connector.handle());
        let make = info.as_ref().and_then(|i| i.make()).unwrap_or_else(|| "Unknown".into());
        let model = info.as_ref().and_then(|i| i.model()).unwrap_or_else(|| "Unknown".into());
        let serial = info.as_ref().and_then(|i| i.serial()).unwrap_or_else(|| "Unknown".into());
        let (pw, ph) = connector.size().unwrap_or((0, 0));
        let output = Output::new(name, PhysicalProperties { size: (pw as i32, ph as i32).into(), subpixel: connector.subpixel().into(), make, model, serial_number: serial });
        let global = output.create_global::<crate::Thearf>(&state.display_handle);
        let x = state.space().outputs().map(|o| state.space().output_geometry(o).unwrap().size.w).sum();
        output.set_preferred(wl_mode);
        output.change_current_state(Some(wl_mode), None, None, Some((x, 0).into()));
        state.space_mut().map_output(&output, (x, 0));
        output.user_data().insert_if_missing(|| OutputId { node, crtc });

        let Ok(planes) = backend.manager.device().planes(&crtc) else { return };
        let Ok(drm_output) = backend.manager.lock().initialize_output::<_, DrmRenderElements<RendererType<'_>>>(crtc, mode, &[connector.handle()], &output, Some(planes), &mut renderer, &DrmOutputRenderElements::default()) else {
            warn!(?crtc, "failed to initialize DRM output"); return;
        };
        backend.surfaces.insert(crtc, SurfaceData { output, global, render_node, drm_output });
        self.render(node, crtc, state);
    }

    fn connector_disconnected(&mut self, node: DrmNode, _connector: connector::Info, crtc: crtc::Handle, state: &mut crate::Thearf) {
        if let Some(backend) = self.backends.get_mut(&node) {
            if let Some(surface) = backend.surfaces.remove(&crtc) {
                state.space_mut().unmap_output(&surface.output);
                state.space_mut().refresh();
            }
        }
    }

    pub fn render(&mut self, node: DrmNode, crtc: crtc::Handle, state: &mut crate::Thearf) {
        let Some(backend) = self.backends.get_mut(&node) else { return };
        let Some(surface) = backend.surfaces.get_mut(&crtc) else { return };
        let output = surface.output.clone();
        let render_node = surface.render_node;
        let Ok(mut renderer) = self.all_gpus.single_renderer(&render_node) else { return };
        let scale = Scale::from(output.current_scale().fractional_scale());
        let mut elements = Vec::<DrmRenderElements<RendererType<'_>>>::new();
        for window in state.space().elements() {
            let Some(loc) = state.space().element_location(window) else { continue };
            elements.extend(window.render_elements(&mut renderer, loc.to_physical_precise_round(scale), scale, 1.0));
        }
        let result = surface.drm_output.render_frame(&mut renderer, &elements, [0.1, 0.1, 0.1, 1.0], smithay::backend::drm::compositor::FrameFlags::DEFAULT);
        match result {
            Ok(frame) => {
                let rendered = !frame.is_empty;
                if rendered {
                    if surface.drm_output.queue_frame(()).is_ok() {
                        for window in state.space().elements() {
                            window.send_frame(&output, state.start_time.elapsed(), Some(Duration::ZERO), |_, _| Some(output.clone()));
                        }
                    }
                } else {
                    let _ = surface.drm_output.queue_frame(());
                }
            }
            Err(e) => warn!(?e, "DRM render failed"),
        }
    }

    pub fn init(&mut self, event_loop: &mut EventLoop<crate::Thearf>, state: &mut crate::Thearf) -> Result<(), Box<dyn std::error::Error>> {
        let udev = UdevBackend::new(&self.session.seat())?;
        let mut libinput = Libinput::new_with_udev::<LibinputSessionInterface<LibSeatSession>>(self.session.clone().into());
        libinput.udev_assign_seat(&self.session.seat()).map_err(|_| "failed to assign libinput seat")?;
        let libinput_backend = LibinputInputBackend::new(libinput.clone());
        event_loop.handle().insert_source(libinput_backend, move |mut event, _, state| {
            if let InputEvent::DeviceAdded { device } = &mut event {
                if device.has_capability(DeviceCapability::Keyboard) {
                    if let Some(led) = state.seat.get_keyboard().map(|k| k.led_state()) { device.led_update(led.into()); }
                }
            }
            state.process_input_event(event);
        })?;
        let loop_handle = event_loop.handle();
        for (id, path) in udev.device_list() {
            if let Ok(node) = DrmNode::from_dev_id(id) {
                if let Err(e) = self.device_added(node, &path, loop_handle.clone(), state) { warn!(?e, ?node, "skipping DRM device"); }
            }
        }
        if let Some(primary) = self.backends.get(&self.primary_gpu) {
            if let Ok(mut renderer) = self.all_gpus.single_renderer(&primary.render_node.unwrap_or(self.primary_gpu)) {
                let feedback = DmabufFeedbackBuilder::new(self.primary_gpu.dev_id(), renderer.dmabuf_formats()).build()?;
                let mut dmabuf = DmabufState::new();
                let global = dmabuf.create_global_with_default_feedback::<crate::Thearf>(&state.display_handle, &feedback);
                self.dmabuf_state = Some((dmabuf, global));
                state.shm_state.update_formats(renderer.shm_formats());
                if let Err(e) = renderer.bind_wl_display(&state.display_handle) {
                    warn!(?e, "failed to bind EGL display to Wayland display");
                }
            }
        }
        let udev_handle = loop_handle.clone();
        event_loop.handle().insert_source(udev, move |event, _, state| match event {
            UdevEvent::Added { device_id, path } => {
                if let Ok(node) = DrmNode::from_dev_id(device_id) {
                    let mut drm = state.drm.take().expect("DRM backend missing");
                    let _ = drm.device_added(node, &path, udev_handle.clone(), state);
                    state.drm = Some(drm);
                }
            }
            UdevEvent::Changed { device_id } => {
                if let Ok(node) = DrmNode::from_dev_id(device_id) {
                    let mut drm = state.drm.take().expect("DRM backend missing");
                    drm.device_changed(node, state);
                    state.drm = Some(drm);
                }
            }
            UdevEvent::Removed { device_id } => { if let Ok(node) = DrmNode::from_dev_id(device_id) { if let Some(backend) = state.drm.as_mut().unwrap().backends.remove(&node) { for (_, s) in backend.surfaces { state.space_mut().unmap_output(&s.output); } } } }
        })?;
        Ok(())
    }

    pub fn handle_session_event(&mut self, event: SessionEvent, state: &mut crate::Thearf, libinput: &mut Libinput) {
        match event {
            SessionEvent::PauseSession => { libinput.suspend(); for backend in self.backends.values_mut() { backend.manager.pause(); } }
            SessionEvent::ActivateSession => { if libinput.resume().is_ok() { for backend in self.backends.values_mut() { let _ = backend.manager.lock().activate(false); } } }
        }
    }
}

pub fn udev(state: &mut crate::Thearf, event_loop: &mut EventLoop<crate::Thearf>) -> Result<(), Box<dyn std::error::Error>> {
    let (session, _notifier) = LibSeatSession::new()?;
    let primary_gpu = primary_gpu(session.seat())
        .ok().flatten()
        .and_then(|p| DrmNode::from_path(p).ok())
        .and_then(|n| n.node_with_type(NodeType::Render).and_then(|r| r.ok()))
        .or_else(|| all_gpus(session.seat()).ok()?.into_iter().find_map(|p| DrmNode::from_path(p).ok()))
        .ok_or("No DRM GPU found")?;
    info!(%primary_gpu, "using primary DRM GPU");
    let gpus = GpuManager::new(GbmGlesBackend::with_factory(|display| {
        let context = EGLContext::new_with_priority(display, ContextPriority::High)?;
        let capabilities = unsafe { GlesRenderer::supported_capabilities(&context)? };
        Ok(unsafe { GlesRenderer::with_capabilities(context, capabilities)? })
    }))?;
    event_loop.handle().insert_source(_notifier, |event, _, state| {
        match event {
            SessionEvent::PauseSession => { for backend in state.drm.as_mut().map(|d| d.backends.values_mut()).into_iter().flatten() { backend.manager.pause(); } }
            SessionEvent::ActivateSession => { if let Some(drm) = state.drm.as_mut() { for backend in drm.backends.values_mut() { let _ = backend.manager.lock().activate(false); } } }
        }
    })?;
    let mut drm = Drm::new(session, primary_gpu, gpus);
    drm.init(event_loop, state)?;
    state.drm = Some(drm);
    Ok(())
}

impl DmabufHandler for crate::Thearf {
    fn dmabuf_state(&mut self) -> &mut DmabufState { &mut self.drm.as_mut().unwrap().dmabuf_state.as_mut().expect("DRM dmabuf state not initialized").0 }
    fn dmabuf_imported(&mut self, _global: &DmabufGlobal, dmabuf: smithay::backend::allocator::dmabuf::Dmabuf, notifier: ImportNotifier) {
        let drm = self.drm.as_mut().expect("DRM backend not initialized");
        let primary = drm.primary_gpu;
        if drm.all_gpus.single_renderer(&primary).and_then(|mut r| r.import_dmabuf(&dmabuf, None)).is_ok() {
            if dmabuf.node().is_none() { dmabuf.set_node(primary); }
            let _ = notifier.successful::<crate::Thearf>();
        } else { notifier.failed(); }
    }
}

impl DrmSyncobjHandler for crate::Thearf {
    fn drm_syncobj_state(&mut self) -> Option<&mut DrmSyncobjState> { self.drm.as_mut().unwrap().syncobj_state.as_mut() }
}
