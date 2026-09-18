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
/// Accent for the ring and count pill; attention outranks everything else.
fn color(status: &Status) -> u32 {
    if status.attention > 0 {
        0xffe9b949
    } else if status.unread > 0 {
        0xff5fd48f
    } else if status.working > 0 {
        0xff5aa9e6
    } else if status.offline > 0 {
        0xff8a8f96
    } else {
        0xff3a4048
    }
}

// The T3 wordmark from assets/prod/logo.svg as 8-bit coverage, 26x16 logical px.
const MARK_W: i32 = 26;
const MARK_H: i32 = 16;
const MARK: [u8; 416] = [
    253,255,255,255,255,255,255,255,255,255,255,255,254,111,130,254,255,255,255,255,255,255,255,255,254,72,
    253,255,255,255,255,255,255,255,255,255,255,255,254,111,130,254,255,255,255,255,255,255,255,255,254,71,
    242,243,243,243,244,255,255,255,249,243,243,243,243,106,120,233,233,233,233,233,233,254,255,255,243,38,
    0,0,0,0,17,254,255,254,129,0,0,0,0,0,0,0,0,0,0,1,116,254,255,251,77,0,
    0,0,0,0,17,254,255,254,129,0,0,0,0,0,0,0,0,0,0,83,252,255,254,111,0,0,
    0,0,0,0,17,254,255,254,129,0,0,0,0,0,0,0,0,0,57,245,255,254,145,0,0,0,
    0,0,0,0,17,254,255,254,129,0,0,0,0,0,0,0,0,16,232,255,255,247,121,32,0,0,
    0,0,0,0,17,254,255,254,129,0,0,0,0,0,0,0,0,31,254,255,255,255,254,248,111,0,
    0,0,0,0,17,254,255,254,129,0,0,0,0,0,0,0,0,31,246,247,253,254,255,255,254,78,
    0,0,0,0,17,254,255,254,129,0,0,0,0,0,0,0,0,0,1,2,18,101,249,255,255,195,
    0,0,0,0,17,254,255,254,129,0,0,0,0,0,0,0,0,0,0,0,0,0,171,255,255,244,
    0,0,0,0,17,254,255,254,129,0,0,0,0,0,4,55,0,0,0,0,0,1,184,255,255,239,
    0,0,0,0,17,254,255,254,129,0,0,0,0,0,97,252,178,89,36,19,43,142,254,255,255,179,
    0,0,0,0,17,254,255,254,129,0,0,0,0,4,219,255,255,254,254,254,254,254,255,255,251,59,
    0,0,0,0,17,254,255,254,129,0,0,0,0,24,199,254,254,255,255,255,255,255,254,245,93,0,
    0,0,0,0,12,180,180,180,91,0,0,0,0,0,1,65,147,204,239,251,243,209,141,34,0,0,
];
// Liberation Sans Bold digits, 8x11.
const DIGITS: [[u8; 11]; 10] = [
    [0b00111100, 0b01111110, 0b01100110, 0b01100111, 0b01100011, 0b01100011, 0b01100011, 0b01100111, 0b01100110, 0b01111110, 0b00111100],
    [0b00011100, 0b01111100, 0b01011100, 0b00011100, 0b00011100, 0b00011100, 0b00011100, 0b00011100, 0b00011100, 0b01111111, 0b01111111],
    [0b00111100, 0b01111110, 0b01100111, 0b00000111, 0b00000110, 0b00001110, 0b00011100, 0b00111000, 0b01110000, 0b01111111, 0b11111111],
    [0b00111100, 0b01111110, 0b01100111, 0b00000110, 0b00011110, 0b00011100, 0b00000111, 0b00000011, 0b11100111, 0b01111110, 0b00111100],
    [0b00001100, 0b00011100, 0b00011100, 0b00111100, 0b00101100, 0b01101100, 0b11001100, 0b11111110, 0b01111110, 0b00001100, 0b00001100],
    [0b01111110, 0b01111110, 0b01100000, 0b01100000, 0b01111110, 0b01111110, 0b00000111, 0b00000011, 0b01100111, 0b01111110, 0b00111100],
    [0b00111100, 0b01111110, 0b01100110, 0b01100000, 0b01111110, 0b11111110, 0b01100111, 0b01100011, 0b01100111, 0b01111110, 0b00111100],
    [0b01111111, 0b01111111, 0b00000110, 0b00000110, 0b00001100, 0b00011100, 0b00011000, 0b00011000, 0b00111000, 0b00110000, 0b00110000],
    [0b00111100, 0b01111110, 0b01100111, 0b01100110, 0b00111100, 0b01111110, 0b01100011, 0b11100011, 0b01100011, 0b01111110, 0b00111100],
    [0b00111100, 0b01111110, 0b01100110, 0b11100011, 0b01100111, 0b01111111, 0b00111111, 0b00000011, 0b01100110, 0b01111110, 0b00111100],
];

/// Coverage (0..=1) of the mark at a logical point, bilinearly sampled so any
/// output scale stays smooth.
fn mark_coverage(fx: f32, fy: f32) -> f32 {
    if fx < 0. || fy < 0. || fx >= MARK_W as f32 || fy >= MARK_H as f32 {
        return 0.;
    }
    let sample = |x: i32, y: i32| -> f32 {
        if x < 0 || y < 0 || x >= MARK_W || y >= MARK_H {
            0.
        } else {
            MARK[(y * MARK_W + x) as usize] as f32 / 255.
        }
    };
    let x0 = (fx - 0.5).floor();
    let y0 = (fy - 0.5).floor();
    let tx = fx - 0.5 - x0;
    let ty = fy - 0.5 - y0;
    let (x0, y0) = (x0 as i32, y0 as i32);
    let top = sample(x0, y0) * (1. - tx) + sample(x0 + 1, y0) * tx;
    let bottom = sample(x0, y0 + 1) * (1. - tx) + sample(x0 + 1, y0 + 1) * tx;
    top * (1. - ty) + bottom * ty
}

fn digit_on(digit: u32, x: i32, y: i32) -> bool {
    x >= 0 && y >= 0 && x < 8 && y < 11 && DIGITS[digit as usize][y as usize] & (0x80 >> x) != 0
}

fn mix(under: u32, over: u32, coverage: f32) -> u32 {
    let c = coverage.clamp(0., 1.);
    let channel = |shift: u32| {
        let a = ((under >> shift) & 0xff) as f32;
        let b = ((over >> shift) & 0xff) as f32;
        ((a + (b - a) * c).round() as u32) << shift
    };
    channel(24) | channel(16) | channel(8) | channel(0)
}

/// Premultiplied ARGB for one device pixel. `x`,`y` are device coordinates and
/// `scale` the buffer scale, so the shapes are analytic and stay crisp at any DPI.
fn pixel(status: &Status, flash: bool, x: i32, y: i32, scale: i32) -> u32 {
    let s = scale as f32;
    let px = (x as f32 + 0.5) / s;
    let py = (y as f32 + 0.5) / s;
    let center = SIZE as f32 / 2.;
    let radius = center - 2.;
    let d = ((px - center).powi(2) + (py - center).powi(2)).sqrt();
    // Antialiased disc edge one logical pixel wide.
    let disc = (radius - d + 0.5).clamp(0., 1.);
    if disc <= 0. {
        return 0;
    }
    let accent = if flash { 0xfff4ecd8 } else { color(status) };
    let ring = 2.0;
    let mut rgb = if d > radius - ring {
        // Ring blends into the fill across one logical pixel.
        let t = (radius - ring - d + 0.5).clamp(0., 1.);
        mix(accent, 0xff14171b, t)
    } else {
        0xff14171b
    };
    // Wordmark, vertically centred slightly above the middle to leave room for the count.
    let count = status.attention.saturating_add(status.unread).min(99);
    let mark_y = if count > 0 { 13. } else { center - MARK_H as f32 / 2. };
    let mark_x = center - MARK_W as f32 / 2.;
    let coverage = mark_coverage(px - mark_x, py - mark_y);
    if coverage > 0. {
        rgb = mix(rgb, 0xfff2f4f7, coverage);
    }
    if count > 0 {
        // Count pill: accent capsule with dark digits.
        let digits: Vec<u32> = if count > 9 { vec![count / 10, count % 10] } else { vec![count] };
        let text_w = digits.len() as f32 * 8. + (digits.len() as f32 - 1.) * 1.;
        let pill_w = text_w + 8.;
        let pill_h = 15.;
        let pill_x = center - pill_w / 2.;
        let pill_y = 32.;
        let r = pill_h / 2.;
        let cx = px.clamp(pill_x + r, pill_x + pill_w - r);
        let cy = pill_y + r;
        let pd = ((px - cx).powi(2) + (py - cy).powi(2)).sqrt();
        let pill = (r - pd + 0.5).clamp(0., 1.);
        if pill > 0. {
            rgb = mix(rgb, accent, pill);
            let tx = px - (center - text_w / 2.);
            let ty = py - (pill_y + 2.);
            let slot = (tx / 9.).floor() as i32;
            if slot >= 0 && (slot as usize) < digits.len() {
                let lx = (tx - slot as f32 * 9.).floor() as i32;
                let ly = ty.floor() as i32;
                if digit_on(digits[slot as usize], lx, ly) {
                    rgb = mix(rgb, 0xff14171b, pill);
                }
            }
        }
    }
    // Premultiply the disc edge.
    let a = (disc * 255.).round() as u32;
    let pm = |shift: u32| ((((rgb >> shift) & 0xff) * a) / 255) << shift;
    (a << 24) | pm(16) | pm(8) | pm(0)
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
            let x = i as i32 % size;
            let y = i as i32 / size;
            chunk.copy_from_slice(
                &pixel(&self.status, self.flash % 2 == 1, x, y, self.scale).to_ne_bytes(),
            );
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
            // Readiness can be consumed by the Wayland backend while dispatching.
            // An empty nonblocking read is not a compositor disconnect.
            match read.read() {
                Ok(_) => {}
                Err(wayland_client::backend::WaylandError::Io(error))
                    if error.kind() == std::io::ErrorKind::WouldBlock => {}
                Err(error) => return Err(error.into()),
            }
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
        assert_eq!(color(&status), 0xffe9b949);
        for scale in [1, 2] {
            for y in 0..SIZE * scale {
                for x in 0..SIZE * scale {
                    pixel(&status, false, x, y, scale);
                }
            }
        }
    }
    #[test]
    fn corners_do_not_intercept_game_clicks() {
        assert!(!inside(0., 0.));
        assert!(inside(28., 28.));
        assert_eq!(pixel(&Status::default(), false, 0, 0, 1), 0);
        assert_eq!(pixel(&Status::default(), false, 0, 0, 2), 0);
    }
    #[test]
    fn scaled_buffers_stay_premultiplied_and_centered() {
        let status = Status { unread: 12, ..Status::default() };
        // Fully opaque interior at both scales.
        assert_eq!(pixel(&status, false, 28, 28, 1) >> 24, 0xff);
        assert_eq!(pixel(&status, false, 56, 56, 2) >> 24, 0xff);
        // Edge pixels never carry colour beyond their alpha.
        for scale in [1, 2] {
            for y in 0..SIZE * scale {
                for x in 0..SIZE * scale {
                    let p = pixel(&status, false, x, y, scale);
                    let a = p >> 24;
                    assert!((p >> 16) & 0xff <= a && (p >> 8) & 0xff <= a && p & 0xff <= a);
                }
            }
        }
        // The ring takes the unread accent.
        assert_eq!(pixel(&status, false, 28, 3, 1), 0xff5fd48f);
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
