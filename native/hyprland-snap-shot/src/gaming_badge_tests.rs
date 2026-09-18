//! Exercise the real badge over a private Wayland connection; never touches the user's desktop.
use std::{
    fs::File,
    io::Write,
    os::unix::{fs::FileExt, net::UnixStream},
    sync::{Arc, mpsc},
    thread,
    time::Duration,
};
use wayland_protocols_wlr::layer_shell::v1::server::{
    zwlr_layer_shell_v1::{self as shell, ZwlrLayerShellV1 as Shell},
    zwlr_layer_surface_v1::{self as layer, ZwlrLayerSurfaceV1 as Layer},
};
use wayland_server::{
    Client, DataInit, Dispatch, Display, DisplayHandle, GlobalDispatch, New, Resource, WEnum,
    backend::{ClientData, ClientId, DisconnectReason},
    protocol::{
        wl_buffer::{self, WlBuffer},
        wl_callback::{self, WlCallback},
        wl_compositor::{self, WlCompositor},
        wl_region::{self, WlRegion},
        wl_shm::{self, WlShm},
        wl_shm_pool::{self, WlShmPool},
        wl_surface::{self, WlSurface},
    },
};
#[derive(Debug)]
struct ClientState;
impl ClientData for ClientState {
    fn initialized(&self, _: ClientId) {}
    fn disconnected(&self, _: ClientId, _: DisconnectReason) {}
}
struct State {
    layer: Option<Layer>,
    configured: bool,
    frame: Option<WlCallback>,
    buffer: Option<WlBuffer>,
    anchor: u32,
    input_area: i32,
    reports: mpsc::Sender<(u32, i32, u32)>,
}
impl GlobalDispatch<WlCompositor, ()> for State {
    fn bind(
        _: &mut Self,
        _: &DisplayHandle,
        _: &Client,
        resource: New<WlCompositor>,
        _: &(),
        init: &mut DataInit<'_, Self>,
    ) {
        init.init(resource, ());
    }
}
impl GlobalDispatch<WlShm, ()> for State {
    fn bind(
        _: &mut Self,
        _: &DisplayHandle,
        _: &Client,
        resource: New<WlShm>,
        _: &(),
        init: &mut DataInit<'_, Self>,
    ) {
        init.init(resource, ()).format(wl_shm::Format::Argb8888);
    }
}
impl GlobalDispatch<Shell, ()> for State {
    fn bind(
        _: &mut Self,
        _: &DisplayHandle,
        _: &Client,
        resource: New<Shell>,
        _: &(),
        init: &mut DataInit<'_, Self>,
    ) {
        init.init(resource, ());
    }
}
impl Dispatch<WlCompositor, ()> for State {
    fn request(
        _: &mut Self,
        _: &Client,
        _: &WlCompositor,
        request: wl_compositor::Request,
        _: &(),
        _: &DisplayHandle,
        init: &mut DataInit<'_, Self>,
    ) {
        match request {
            wl_compositor::Request::CreateSurface { id } => {
                init.init(id, ());
            }
            wl_compositor::Request::CreateRegion { id } => {
                init.init(id, ());
            }
            _ => {}
        }
    }
}
impl Dispatch<Shell, ()> for State {
    fn request(
        state: &mut Self,
        _: &Client,
        _: &Shell,
        request: shell::Request,
        _: &(),
        _: &DisplayHandle,
        init: &mut DataInit<'_, Self>,
    ) {
        if let shell::Request::GetLayerSurface {
            id,
            layer,
            namespace,
            ..
        } = request
        {
            assert_eq!(layer, WEnum::Value(shell::Layer::Overlay));
            assert_eq!(namespace, "t3-gaming-badge");
            state.layer = Some(init.init(id, ()));
        }
    }
}
impl Dispatch<Layer, ()> for State {
    fn request(
        state: &mut Self,
        _: &Client,
        _: &Layer,
        request: layer::Request,
        _: &(),
        _: &DisplayHandle,
        _: &mut DataInit<'_, Self>,
    ) {
        match request {
            layer::Request::SetKeyboardInteractivity {
                keyboard_interactivity,
            } => assert_eq!(
                keyboard_interactivity,
                WEnum::Value(layer::KeyboardInteractivity::None)
            ),
            layer::Request::SetExclusiveZone { zone } => assert_eq!(zone, -1),
            layer::Request::SetSize { width, height } => assert_eq!((width, height), (56, 56)),
            layer::Request::SetAnchor { anchor } => {
                state.anchor = match anchor {
                    WEnum::Value(anchor) => anchor.bits(),
                    WEnum::Unknown(value) => value,
                };
            }
            _ => {}
        }
    }
}
impl Dispatch<WlRegion, ()> for State {
    fn request(
        state: &mut Self,
        _: &Client,
        _: &WlRegion,
        request: wl_region::Request,
        _: &(),
        _: &DisplayHandle,
        _: &mut DataInit<'_, Self>,
    ) {
        if let wl_region::Request::Add { width, height, .. } = request {
            state.input_area += width * height;
        }
    }
}
impl Dispatch<WlSurface, ()> for State {
    fn request(
        state: &mut Self,
        _: &Client,
        _: &WlSurface,
        request: wl_surface::Request,
        _: &(),
        _: &DisplayHandle,
        init: &mut DataInit<'_, Self>,
    ) {
        match request {
            wl_surface::Request::Frame { callback } => state.frame = Some(init.init(callback, ())),
            wl_surface::Request::Attach { buffer, .. } => state.buffer = buffer,
            wl_surface::Request::Commit => {
                if !state.configured {
                    state.layer.as_ref().unwrap().configure(1, 56, 56);
                    state.configured = true;
                } else if let Some(buffer) = state.buffer.take() {
                    let (file, offset) = buffer.data::<(Arc<File>, u64)>().unwrap();
                    let mut pixel = [0; 4];
                    // Top of the badge's colored rim in a 56px-wide ARGB buffer.
                    file.read_exact_at(&mut pixel, offset + (3 * 56 + 28) * 4)
                        .unwrap();
                    buffer.release();
                    if let Some(frame) = state.frame.take() {
                        frame.done(1);
                    }
                    state
                        .reports
                        .send((state.anchor, state.input_area, u32::from_ne_bytes(pixel)))
                        .unwrap();
                }
            }
            _ => {}
        }
    }
}
impl Dispatch<WlCallback, ()> for State {
    fn request(
        _: &mut Self,
        _: &Client,
        _: &WlCallback,
        _: wl_callback::Request,
        _: &(),
        _: &DisplayHandle,
        _: &mut DataInit<'_, Self>,
    ) {
    }
}
impl Dispatch<WlShm, ()> for State {
    fn request(
        _: &mut Self,
        _: &Client,
        _: &WlShm,
        request: wl_shm::Request,
        _: &(),
        _: &DisplayHandle,
        init: &mut DataInit<'_, Self>,
    ) {
        if let wl_shm::Request::CreatePool { id, fd, .. } = request {
            init.init(id, Arc::new(File::from(fd)));
        }
    }
}
impl Dispatch<WlShmPool, Arc<File>> for State {
    fn request(
        _: &mut Self,
        _: &Client,
        _: &WlShmPool,
        request: wl_shm_pool::Request,
        file: &Arc<File>,
        _: &DisplayHandle,
        init: &mut DataInit<'_, Self>,
    ) {
        if let wl_shm_pool::Request::CreateBuffer {
            id,
            offset,
            width,
            height,
            stride,
            ..
        } = request
        {
            assert_eq!((width, height, stride), (56, 56, 224));
            init.init(id, (file.clone(), offset as u64));
        }
    }
}
impl Dispatch<WlBuffer, (Arc<File>, u64)> for State {
    fn request(
        _: &mut Self,
        _: &Client,
        _: &WlBuffer,
        _: wl_buffer::Request,
        _: &(Arc<File>, u64),
        _: &DisplayHandle,
        _: &mut DataInit<'_, Self>,
    ) {
    }
}

#[test]
fn badge_maps_above_fullscreen_without_keyboard_focus_updates_and_exits_on_parent_eof() {
    let mut display = Display::<State>::new().unwrap();
    let mut handle = display.handle();
    handle.create_global::<State, WlCompositor, _>(6, ());
    handle.create_global::<State, WlShm, _>(1, ());
    handle.create_global::<State, Shell, _>(4, ());
    let (client, server) = UnixStream::pair().unwrap();
    handle.insert_client(server, Arc::new(ClientState)).unwrap();
    let (mut stop, wake) = UnixStream::pair().unwrap();
    let (tx, rx) = mpsc::channel();
    let mut state = State {
        layer: None,
        configured: false,
        frame: None,
        buffer: None,
        anchor: 0,
        input_area: 0,
        reports: tx,
    };
    let compositor = thread::spawn(move || {
        loop {
            display.dispatch_clients(&mut state).unwrap();
            display.flush_clients().unwrap();
            let mut fds = [
                rustix::event::PollFd::new(&display, rustix::event::PollFlags::IN),
                rustix::event::PollFd::new(&wake, rustix::event::PollFlags::IN),
            ];
            rustix::event::poll(&mut fds, None).unwrap();
            if !fds[1].revents().is_empty() {
                break;
            }
        }
    });
    let (mut commands, input) = UnixStream::pair().unwrap();
    let badge = thread::spawn(move || {
        crate::gaming_badge::run_on(
            wayland_client::Connection::from_socket(client).unwrap(),
            input,
        )
        .map_err(|e| e.to_string())
    });
    // Deadline only bounds a broken transport test; every assertion follows a compositor receipt.
    let initial = rx.recv_timeout(Duration::from_secs(3)).unwrap();
    assert_eq!(
        initial.0,
        (layer::Anchor::Top | layer::Anchor::Right).bits()
    );
    assert!(initial.1 > 0 && initial.1 < 56 * 56);
    assert_eq!(initial.2, 0xffb59a63);
    commands.write_all(b"{\"command\":\"update\",\"status\":{\"attention\":1,\"unread\":2,\"working\":0,\"offline\":0,\"corner\":\"bottom-left\",\"pulse\":false}}\n").unwrap();
    let updated = rx.recv_timeout(Duration::from_secs(3)).unwrap();
    assert_eq!(
        updated.0,
        (layer::Anchor::Bottom | layer::Anchor::Left).bits()
    );
    assert_eq!(updated.2, 0xffedc46c);
    drop(commands);
    badge.join().unwrap().unwrap();
    stop.write_all(&[1]).unwrap();
    compositor.join().unwrap();
    assert!(rx.try_recv().is_err());
}
