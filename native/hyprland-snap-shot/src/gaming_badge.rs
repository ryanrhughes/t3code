//! A small, keyboard-inert layer surface. Sleeps on Wayland/stdin unless a finite alert is flashing.
use crate::{Result, emit};
use serde::Deserialize;
use smithay_client_toolkit::{
    compositor::{CompositorHandler, CompositorState},
    delegate_compositor, delegate_layer, delegate_output, delegate_pointer, delegate_registry,
    delegate_seat, delegate_shm,
    output::{OutputHandler, OutputState},
    registry::{ProvidesRegistryState, RegistryState},
    seat::{
        Capability, SeatHandler, SeatState,
        pointer::{PointerEvent, PointerEventKind, PointerHandler},
    },
    shell::{
        WaylandSurface,
        wlr_layer::{
            Anchor, KeyboardInteractivity, Layer, LayerShell, LayerShellHandler, LayerSurface,
            LayerSurfaceConfigure,
        },
    },
    shm::{Shm, ShmHandler, slot::SlotPool},
};
use std::{
    io::Read,
    os::fd::AsFd,
    time::{Duration, Instant},
};
use wayland_client::{
    Connection, QueueHandle,
    globals::registry_queue_init,
    protocol::{wl_output, wl_pointer, wl_seat, wl_shm, wl_surface},
};

const SIZE: i32 = 56;

#[derive(Clone, Copy, Default, Deserialize)]
#[serde(rename_all = "kebab-case")]
enum Corner {
    TopLeft,
    #[default]
    TopRight,
    BottomLeft,
    BottomRight,
}
impl Corner {
    fn anchor(self) -> Anchor {
        match self {
            Self::TopLeft => Anchor::TOP | Anchor::LEFT,
            Self::TopRight => Anchor::TOP | Anchor::RIGHT,
            Self::BottomLeft => Anchor::BOTTOM | Anchor::LEFT,
            Self::BottomRight => Anchor::BOTTOM | Anchor::RIGHT,
        }
    }
}
#[derive(Default, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Status {
    attention: u32,
    unread: u32,
    working: u32,
    offline: u32,
    corner: Corner,
    pulse: bool,
}
#[derive(Deserialize)]
#[serde(tag = "command", rename_all = "lowercase")]
enum Command {
    Close,
    Update { status: Status },
}
struct Badge {
    registry: RegistryState,
    output: OutputState,
    seat: SeatState,
    shm: Shm,
    pool: SlotPool,
    layer: LayerSurface,
    pointers: Vec<(wl_seat::WlSeat, wl_pointer::WlPointer)>,
    status: Status,
    configured: bool,
    ready: bool,
    done: bool,
    pressed: bool,
    scale: i32,
    flash: u8,
    next_flash: Option<Instant>,
}

/// The entire transparent square is never an input target: only the round badge is clickable.
fn inside(x: f64, y: f64) -> bool {
    (x - 28.).powi(2) + (y - 28.).powi(2) <= 26. * 26.
}
fn color(status: &Status) -> u32 {
    if status.attention > 0 {
        0xffedc46c
    } else if status.unread > 0 {
        0xff94c7a3
    } else if status.working > 0 {
        0xff8dc5e6
    } else if status.offline > 0 {
        0xffaaa69c
    } else {
        0xffb59a63
    }
}
const DIGITS: [[u8; 5]; 10] = [
    [7, 5, 5, 5, 7],
    [2, 6, 2, 2, 7],
    [7, 1, 7, 4, 7],
    [7, 1, 7, 1, 7],
    [5, 5, 7, 1, 1],
    [7, 4, 7, 1, 7],
    [7, 4, 7, 5, 7],
    [7, 1, 1, 1, 1],
    [7, 5, 7, 5, 7],
    [7, 5, 7, 1, 7],
];
fn glyph(rows: &[u8; 5], x: i32, y: i32, left: i32, top: i32, scale: i32) -> bool {
    let (x, y) = (x - left, y - top);
    x >= 0
        && y >= 0
        && x < 3 * scale
        && y < 5 * scale
        && rows[(y / scale) as usize] & (1 << (2 - x / scale)) != 0
}
fn pixel(status: &Status, flash: bool, x: i32, y: i32) -> u32 {
    let distance = (x - 28).pow(2) + (y - 28).pow(2);
    if distance > 26 * 26 {
        return 0;
    }
    let accent = color(status);
    if distance >= 23 * 23 {
        return if flash { 0xfff6ecd3 } else { accent };
    }
    // A legible T3 rune with a status marker; no font dependency in the native helper.
    if glyph(&[7, 2, 2, 2, 2], x, y, 15, 15, 3) || glyph(&DIGITS[3], x, y, 29, 15, 3) {
        return 0xffece4cf;
    }
    let count = status.attention.saturating_add(status.unread).min(99);
    if count > 0 {
        let start = if count > 9 { 21 } else { 25 };
        let first = if count > 9 { count / 10 } else { count };
        if glyph(&DIGITS[first as usize], x, y, start, 36, 2)
            || (count > 9 && glyph(&DIGITS[(count % 10) as usize], x, y, 29, 36, 2))
        {
            return accent;
        }
    } else if (x - 28).pow(2) + (y - 39).pow(2) < 9 {
        return accent;
    }
    0xff181d21
}

impl Badge {
    fn draw(&mut self, qh: &QueueHandle<Self>) -> Result<()> {
        if !self.configured {
            return Ok(());
        }
        let size = SIZE * self.scale;
        let (buffer, canvas) =
            self.pool
                .create_buffer(size, size, size * 4, wl_shm::Format::Argb8888)?;
        for (i, chunk) in canvas.chunks_exact_mut(4).enumerate() {
            let x = (i as i32 % size) / self.scale;
            let y = (i as i32 / size) / self.scale;
            chunk.copy_from_slice(&pixel(&self.status, self.flash % 2 == 1, x, y).to_ne_bytes());
        }
        let surface = self.layer.wl_surface();
        surface.set_buffer_scale(self.scale);
        buffer.attach_to(surface)?;
        surface.damage_buffer(0, 0, size, size);
        if !self.ready {
            surface.frame(qh, surface.clone());
        }
        surface.commit();
        Ok(())
    }
    fn redraw(&mut self, qh: &QueueHandle<Self>) {
        if self.draw(qh).is_err() {
            self.done = true;
        }
    }
    fn update(&mut self, status: Status, qh: &QueueHandle<Self>) {
        if status.pulse && (status.attention > 0 || status.unread > 0) {
            self.flash = 5;
            self.next_flash = Some(Instant::now() + Duration::from_millis(320));
        }
        self.layer.set_anchor(status.corner.anchor());
        self.status = status;
        self.redraw(qh);
    }
}

pub fn run() -> Result<()> {
    run_on(Connection::connect_to_env()?, std::io::stdin().lock())
}

pub(crate) fn run_on(connection: Connection, mut stdin: impl Read + AsFd) -> Result<()> {
    let (globals, mut queue) = registry_queue_init(&connection)?;
    let qh = queue.handle();
    let compositor = CompositorState::bind(&globals, &qh)?;
    let shell = LayerShell::bind(&globals, &qh)?;
    let shm = Shm::bind(&globals, &qh)?;
    let surface = compositor.create_surface(&qh);
    let region = compositor.wl_compositor().create_region(&qh, ());
    for y in 0..SIZE {
        let half = ((26 * 26 - (y - 28).pow(2)).max(0) as f64).sqrt() as i32;
        if half > 0 {
            region.add(28 - half, y, half * 2, 1);
        }
    }
    surface.set_input_region(Some(&region));
    region.destroy();
    let layer =
        shell.create_layer_surface(&qh, surface, Layer::Overlay, Some("t3-gaming-badge"), None);
    layer.set_keyboard_interactivity(KeyboardInteractivity::None);
    layer.set_exclusive_zone(-1);
    layer.set_anchor(Corner::default().anchor());
    layer.set_margin(96, 24, 24, 24);
    layer.set_size(SIZE as u32, SIZE as u32);
    layer.commit();
    let mut state = Badge {
        registry: RegistryState::new(&globals),
        output: OutputState::new(&globals, &qh),
        seat: SeatState::new(&globals, &qh),
        pool: SlotPool::new((SIZE * SIZE * 4) as usize, &shm)?,
        shm,
        layer,
        pointers: Vec::new(),
        status: Status::default(),
        configured: false,
        ready: false,
        done: false,
        pressed: false,
        scale: 1,
        flash: 0,
        next_flash: None,
    };
    let mut commands = Vec::new();
    while !state.done {
        queue.dispatch_pending(&mut state)?;
        if state.done {
            break;
        }
        if state.next_flash.is_some_and(|time| time <= Instant::now()) {
            state.flash = state.flash.saturating_sub(1);
            state.next_flash =
                (state.flash > 0).then(|| Instant::now() + Duration::from_millis(320));
            state.draw(&qh)?;
        }
        connection.flush()?;
        let Some(read) = queue.prepare_read() else {
            continue;
        };
        let timeout = state
            .next_flash
            .map(|time| {
                rustix::event::Timespec::try_from(time.saturating_duration_since(Instant::now()))
            })
            .transpose()?;
        let mut fds = [
            rustix::event::PollFd::new(&connection, rustix::event::PollFlags::IN),
            rustix::event::PollFd::new(&stdin, rustix::event::PollFlags::IN),
        ];
        rustix::event::poll(&mut fds, timeout.as_ref())?;
        if !fds[0].revents().is_empty() {
            read.read()?;
        } else {
            drop(read);
        }
        let input_ready = !fds[1].revents().is_empty();
        drop(fds);
        if input_ready {
            let mut bytes = [0; 2048];
            let count = stdin.read(&mut bytes)?;
            if count == 0 {
                break;
            }
            commands.extend_from_slice(&bytes[..count]);
            if commands.len() > 8192 {
                return Err("Oversized gaming badge command.".into());
            }
            while let Some(end) = commands.iter().position(|byte| *byte == b'\n') {
                let line: Vec<_> = commands.drain(..=end).collect();
                match serde_json::from_slice(&line)? {
                    Command::Close => {
                        state.done = true;
                        break;
                    }
                    Command::Update { status } => state.update(status, &qh),
                }
            }
        }
    }
    emit(serde_json::json!({"event":"done"}))
}

impl CompositorHandler for Badge {
    fn scale_factor_changed(
        &mut self,
        _: &Connection,
        qh: &QueueHandle<Self>,
        _: &wl_surface::WlSurface,
        scale: i32,
    ) {
        self.scale = scale.clamp(1, 8);
        self.redraw(qh);
    }
    fn transform_changed(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_surface::WlSurface,
        _: wl_output::Transform,
    ) {
    }
    fn surface_enter(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_surface::WlSurface,
        _: &wl_output::WlOutput,
    ) {
    }
    fn surface_leave(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_surface::WlSurface,
        _: &wl_output::WlOutput,
    ) {
    }
    fn frame(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_surface::WlSurface, _: u32) {
        if !self.ready {
            self.ready = true;
            if emit(serde_json::json!({"event":"ready"})).is_err() {
                self.done = true;
            }
        }
    }
}
impl LayerShellHandler for Badge {
    fn closed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &LayerSurface) {
        self.done = true;
    }
    fn configure(
        &mut self,
        _: &Connection,
        qh: &QueueHandle<Self>,
        _: &LayerSurface,
        config: LayerSurfaceConfigure,
        _: u32,
    ) {
        if (config.new_size.0 != 0 && config.new_size.0 != SIZE as u32)
            || (config.new_size.1 != 0 && config.new_size.1 != SIZE as u32)
        {
            self.done = true;
            return;
        }
        self.configured = true;
        self.redraw(qh);
    }
}
impl SeatHandler for Badge {
    fn seat_state(&mut self) -> &mut SeatState {
        &mut self.seat
    }
    fn new_seat(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_seat::WlSeat) {}
    fn new_capability(
        &mut self,
        _: &Connection,
        qh: &QueueHandle<Self>,
        seat: wl_seat::WlSeat,
        capability: Capability,
    ) {
        if capability == Capability::Pointer && !self.pointers.iter().any(|(s, _)| s == &seat) {
            if let Ok(pointer) = self.seat.get_pointer(qh, &seat) {
                self.pointers.push((seat, pointer));
            }
        }
    }
    fn remove_capability(
        &mut self,
        conn: &Connection,
        qh: &QueueHandle<Self>,
        seat: wl_seat::WlSeat,
        capability: Capability,
    ) {
        if capability == Capability::Pointer {
            self.remove_seat(conn, qh, seat);
        }
    }
    fn remove_seat(&mut self, _: &Connection, _: &QueueHandle<Self>, seat: wl_seat::WlSeat) {
        self.pointers.retain(|(s, p)| {
            if s == &seat {
                p.release();
                false
            } else {
                true
            }
        });
        self.pressed = false;
    }
}
impl PointerHandler for Badge {
    fn pointer_frame(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_pointer::WlPointer,
        events: &[PointerEvent],
    ) {
        for event in events {
            if &event.surface != self.layer.wl_surface() {
                continue;
            }
            match event.kind {
                PointerEventKind::Press { button: 0x110, .. } => {
                    self.pressed = inside(event.position.0, event.position.1)
                }
                PointerEventKind::Release { button: 0x110, .. } => {
                    if self.pressed
                        && inside(event.position.0, event.position.1)
                        && emit(serde_json::json!({"event":"activate"})).is_err()
                    {
                        self.done = true;
                    }
                    self.pressed = false;
                }
                PointerEventKind::Leave { .. } => self.pressed = false,
                _ => {}
            }
        }
    }
}
impl OutputHandler for Badge {
    fn output_state(&mut self) -> &mut OutputState {
        &mut self.output
    }
    fn new_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
    fn update_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
    fn output_destroyed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
}
impl ShmHandler for Badge {
    fn shm_state(&mut self) -> &mut Shm {
        &mut self.shm
    }
}
impl ProvidesRegistryState for Badge {
    fn registry(&mut self) -> &mut RegistryState {
        &mut self.registry
    }
    smithay_client_toolkit::registry_handlers!(OutputState, SeatState);
}
delegate_compositor!(Badge);
delegate_layer!(Badge);
delegate_output!(Badge);
delegate_pointer!(Badge);
delegate_seat!(Badge);
delegate_shm!(Badge);
delegate_registry!(Badge);
wayland_client::delegate_noop!(Badge: ignore wayland_client::protocol::wl_region::WlRegion);

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn attention_wins_and_counts_cannot_overflow() {
        let status = Status {
            attention: u32::MAX,
            unread: 10,
            working: 2,
            ..Status::default()
        };
        assert_eq!(color(&status), 0xffedc46c);
        for y in 0..SIZE {
            for x in 0..SIZE {
                pixel(&status, false, x, y);
            }
        }
    }
    #[test]
    fn corners_do_not_intercept_game_clicks() {
        assert!(!inside(0., 0.));
        assert!(inside(28., 28.));
        assert_eq!(pixel(&Status::default(), false, 0, 0), 0);
    }
    #[test]
    fn update_protocol_rejects_invalid_data() {
        assert!(
            serde_json::from_str::<Command>(r#"{"command":"update","status":{"attention":-1}}"#)
                .is_err()
        );
        assert!(serde_json::from_str::<Command>(r#"{"command":"update","status":{"attention":0,"unread":1,"working":0,"offline":0,"corner":"top-right","pulse":true}}"#).is_ok());
    }
}
