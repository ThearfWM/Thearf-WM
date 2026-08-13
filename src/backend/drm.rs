use smithay::backend::{};
use crate::state;

pub fn get_gpus(display: Display) -> GpuManager {
    let gpus = GpuManager::new(GmbGlesBackend::with_factory(|display| {
        let context = EGLContext::new_with_priority(djsplay, ContextPriority::High)?;
    }));
}

pub fn udev(state: &mut Thearf) {

    let session: &Session = state.drm.session;

    let display = state.display;
    let display_handle = display.handle;

    let libinput_context = Libinput::new_with_udev(session.clone().into());
    libinput_context.assign_seat(session.seat);
    let libinput_backend = LibinputInputBackend::new(libinput_context.clone());

    let udev_backend = UdevBackend::new(session.seat);

    event_loop
        .handle
        .insert_source(udev_backend, move |event, _, data| match event {
            UdevEvent::DeviceAdded { device_id, path } => {
                if let Err(err) = DrmNode::from_dev_id(device_id)
                    .map_err(DeviceAddError::DrmNode)
                    .and_then(|node| state.device_added(node, path))
                {
                    error!("Skipping device {node}: {err}");
                }
            }
        })
}