#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss
)]

use std::fmt::Write as _;
use std::io::IsTerminal;
use std::time::Instant;

use indicatif::{ProgressState, style::ProgressTracker};

const WIDTH: usize = 32;
const ROWS: usize = 4;
const PX: usize = WIDTH * 2;
const MID: f64 = 1.5;
const MIB: f64 = 1_048_576.0;
const CRAWL: f64 = 65_536.0;
const BLAST: f64 = 1.1;

const OPEN: [&[u8; 8]; ROWS] = [b"#.####..", b"########", b"####....", b"..#####."];
const SHUT: [&[u8; 8]; ROWS] = [b"#.####..", b"########", b"#######.", b"..#....."];
const BITS: [[u32; 2]; ROWS] = [[0x01, 0x08], [0x02, 0x10], [0x04, 0x20], [0x40, 0x80]];
const VORTEX: [char; 4] = ['◐', '◓', '◑', '◒'];
const SCALES: [char; ROWS] = ['\'', '~', '-', '_'];

type Rgb = (f64, f64, f64);

const AURORA: [Rgb; 4] = [
    (70.0, 40.0, 150.0),
    (60.0, 120.0, 230.0),
    (60.0, 210.0, 210.0),
    (190.0, 255.0, 220.0),
];
const FIRE: [Rgb; 5] = [
    (255.0, 250.0, 210.0),
    (255.0, 210.0, 80.0),
    (255.0, 120.0, 30.0),
    (200.0, 40.0, 30.0),
    (110.0, 20.0, 30.0),
];
const HEAD: Rgb = (120.0, 255.0, 220.0);
const TUNNEL: Rgb = (150.0, 90.0, 255.0);
const TRACK: Rgb = (80.0, 80.0, 95.0);
const TUNNEL_TRACK: Rgb = (90.0, 70.0, 130.0);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Paint {
    Plain,
    Ansi256,
    TrueColor,
}

impl Paint {
    pub fn detect() -> Self {
        let set = |name: &str| std::env::var_os(name).is_some_and(|v| !v.is_empty());
        let colorterm = std::env::var("COLORTERM").unwrap_or_default();
        Self::choose(
            std::io::stderr().is_terminal(),
            set("NO_COLOR") || std::env::var("TERM").is_ok_and(|t| t == "dumb"),
            matches!(colorterm.as_str(), "truecolor" | "24bit") || set("WT_SESSION"),
        )
    }

    fn choose(terminal: bool, refused: bool, deep: bool) -> Self {
        match (terminal && !refused, deep) {
            (false, _) => Self::Plain,
            (true, false) => Self::Ansi256,
            (true, true) => Self::TrueColor,
        }
    }

    fn ink(self, out: &mut String, colour: Rgb, bold: bool, glyph: char) {
        let level = |v: f64| v.clamp(0.0, 255.0).round() as u8;
        let (r, g, b) = (level(colour.0), level(colour.1), level(colour.2));
        let bold = if bold { "\x1b[1m" } else { "" };
        let _ = match self {
            Self::Plain => write!(out, "{glyph}"),
            Self::Ansi256 => {
                let six = |v: u8| (u16::from(v) * 5 + 127) / 255;
                let index = 16 + 36 * six(r) + 6 * six(g) + six(b);
                write!(out, "{bold}\x1b[38;5;{index}m{glyph}\x1b[0m")
            }
            Self::TrueColor => write!(out, "{bold}\x1b[38;2;{r};{g};{b}m{glyph}\x1b[0m"),
        };
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Look {
    pub ascii: bool,
    pub paint: Paint,
    pub wormhole: bool,
}

#[derive(Clone, Debug)]
pub struct Wyrm {
    look: Look,
    now: f64,
    frac: f64,
    phase: f64,
    amp: f64,
    chomp: f64,
    burst: Option<f64>,
    rate: f64,
    earlier: f64,
    earlier_at: f64,
    idle: f64,
    pos: u64,
    seen: bool,
    finished: bool,
}

struct Sketch {
    dots: [[bool; PX]; ROWS],
    fire: [[Option<f64>; PX]; ROWS],
}

impl Sketch {
    fn put(&mut self, x: isize, y: isize) {
        if let (Ok(x), Ok(y)) = (usize::try_from(x), usize::try_from(y))
            && x < PX
            && y < ROWS
        {
            self.dots[y][x] = true;
        }
    }
}

impl Wyrm {
    pub fn new(look: Look) -> Self {
        Self {
            look,
            now: 0.0,
            frac: 0.0,
            phase: 0.0,
            amp: 1.0,
            chomp: 0.0,
            burst: None,
            rate: 0.0,
            earlier: 0.0,
            earlier_at: 0.0,
            idle: 0.0,
            pos: 0,
            seen: false,
            finished: false,
        }
    }

    pub fn step(&mut self, now: f64, pos: u64, len: Option<u64>) {
        let dt = if self.seen {
            (now - self.now).max(0.0)
        } else {
            0.0
        };
        let moved = pos.saturating_sub(self.pos) as f64;
        let ease = (dt * 3.0).min(1.0);
        self.rate += if dt > 0.0 {
            (moved / dt - self.rate) * ease
        } else {
            moved * 3.0
        };
        self.idle = if moved > 0.0 { 0.0 } else { self.idle + dt };
        let finished = len.is_some_and(|len| pos >= len);
        if finished && !self.finished {
            self.burst = Some(now);
        }
        let quiet = self.burst.is_none_or(|at| now - at > 3.0);
        if !finished && quiet && self.rate > MIB && self.rate > 2.2 * self.earlier.max(CRAWL) {
            self.burst = Some(now);
        }
        if now - self.earlier_at >= 0.5 {
            self.earlier = self.rate;
            self.earlier_at = now;
        }
        let pace = ((self.rate / CRAWL).max(1.0).log2() / 8.0).min(1.0);
        let resting = finished || self.idle > 1.2;
        let target = if resting {
            0.8 + 0.15 * (now * 1.8).sin()
        } else {
            1.0
        };
        self.amp += (target - self.amp) * ease;
        self.phase += dt * if resting { 0.35 } else { 2.5 + 11.0 * pace };
        if !resting {
            self.chomp += dt * (0.8 + 3.0 * pace);
        }
        self.frac = match len {
            Some(0) | None => 0.0,
            Some(len) => pos.min(len) as f64 / len as f64,
        };
        self.now = now;
        self.pos = pos;
        self.seen = true;
        self.finished = finished;
    }

    fn breath(&self) -> f64 {
        match self.burst.map(|at| self.now - at) {
            Some(t) if (0.0..=BLAST).contains(&t) => {
                (t / 0.25).min(1.0) * ((BLAST - t) / 0.35).min(1.0)
            }
            _ => 0.0,
        }
    }

    fn mouths(&self) -> Option<(usize, usize)> {
        self.look.wormhole.then_some((WIDTH / 4, WIDTH * 3 / 4))
    }

    fn tunnelled(&self, x: usize) -> bool {
        self.mouths()
            .is_some_and(|(into, out)| into * 2 + 1 < x && x < out * 2)
    }

    fn head(&self) -> f64 {
        4.0 + self.frac * (PX as f64 - 10.0)
    }

    fn centre(&self, x: f64) -> f64 {
        let head = self.head();
        let near = ((head - 8.0 - x) / 6.0).clamp(0.0, 1.0);
        let swing = self.amp * MID * (0.55 + 0.45 * (x / 8.0).min(1.0)) * near;
        MID + swing * ((x - head) * 0.5 + self.phase).sin()
    }

    fn open(&self) -> bool {
        self.breath() > 0.0 || !(self.chomp as u64).is_multiple_of(3)
    }

    fn tick(&self) -> u64 {
        (self.now * 24.0) as u64
    }

    fn sketch(&self) -> Sketch {
        let mut sketch = Sketch {
            dots: [[false; PX]; ROWS],
            fire: [[None; PX]; ROWS],
        };
        let head = self.head();
        let h = head as usize;
        for x in 0..h.saturating_sub(6) {
            let at = x as f64;
            let c = self.centre(at);
            let half = 0.3 + 0.5 * (at / (head * 0.6).max(1.0)).min(1.0);
            if self.tunnelled(x) {
                if x % 2 == 0 {
                    sketch.put(x as isize, (c - half).round() as isize);
                    sketch.put(x as isize, (c + half).round() as isize);
                }
                continue;
            }
            for y in 0..ROWS {
                if (y as f64 - c).abs() <= half + 0.01 {
                    sketch.put(x as isize, y as isize);
                }
            }
        }
        let sprite = if self.open() { OPEN } else { SHUT };
        let left = h as isize - 7;
        for (y, line) in sprite.iter().enumerate() {
            for (i, &dot) in line.iter().enumerate() {
                let x = left + i as isize;
                let shown = usize::try_from(x).is_ok_and(|x| !self.tunnelled(x) || x % 2 == 0);
                if dot == b'#' && shown {
                    sketch.put(x, y as isize);
                }
            }
        }
        let power = self.breath();
        if power > 0.0 {
            let reach = 3.0 + 11.0 * power;
            for d in 1..=reach as usize {
                let x = h + d;
                let spread = 0.4 + 0.28 * d as f64;
                for (y, row) in sketch.fire.iter_mut().enumerate() {
                    let off = (y as f64 - 2.2).abs();
                    let chance = 0.95 - 0.55 * d as f64 / reach - 0.25 * off / spread;
                    if x < PX && off <= spread && noise(x, y, self.tick()) < chance {
                        row[x] = Some(d as f64 / reach);
                    }
                }
            }
        }
        sketch
    }

    pub fn draw(&self) -> String {
        let sketch = self.sketch();
        let head = self.head() as usize / 2;
        let mouths = self.mouths();
        let paint = if self.look.ascii {
            Paint::Plain
        } else {
            self.look.paint
        };
        let shimmer = (self.now * 12.0) % (head as f64 + 10.0);
        let mut out = String::new();
        for col in 0..WIDTH {
            let mut bits = 0;
            let mut fire = 0;
            let mut warmths = Vec::new();
            for (y, row) in BITS.iter().enumerate() {
                for (k, &bit) in row.iter().enumerate() {
                    let x = col * 2 + k;
                    if sketch.dots[y][x] {
                        bits |= bit;
                    }
                    if let Some(warmth) = sketch.fire[y][x] {
                        fire |= bit;
                        warmths.push(warmth);
                    }
                }
            }
            let inside = mouths.is_some_and(|(into, out)| into < col && col < out);
            if mouths.is_some_and(|(into, out)| col == into || col == out) {
                let swirl = f64::midpoint((self.now * 5.0 + col as f64).sin(), 1.0);
                let colour = lerp((140.0, 70.0, 255.0), (255.0, 110.0, 220.0), swirl);
                let glyph = if self.look.ascii {
                    'O'
                } else {
                    VORTEX[((self.now * 8.0) as usize + col) % 4]
                };
                paint.ink(&mut out, colour, true, glyph);
            } else if !warmths.is_empty() && col > head {
                let warmth = warmths.iter().sum::<f64>() / warmths.len() as f64;
                let flicker = noise(col, 0, self.tick());
                let colour = grad(&FIRE, warmth * 0.9 + 0.1 * flicker);
                let glyph = if self.look.ascii {
                    ['*', '+', ':'][((warmth * 3.0) as usize).min(2)]
                } else {
                    braille(bits | fire)
                };
                paint.ink(&mut out, colour, false, glyph);
            } else if col > head + 2 || (bits == 0 && col > head) {
                let colour = if inside { TUNNEL_TRACK } else { TRACK };
                let crumb = if self.look.ascii { '.' } else { '·' };
                paint.ink(
                    &mut out,
                    colour,
                    false,
                    if col % 2 == 0 { crumb } else { ' ' },
                );
            } else if bits == 0 {
                out.push(' ');
            } else {
                let mut colour = grad(&AURORA, 0.1 + 0.85 * col as f64 / head.max(1) as f64);
                let gap = (head as f64 - col as f64) - shimmer + 5.0;
                colour = brighten(colour, 0.5 * (-(gap * gap) / 3.0).exp());
                if col + 3 >= head {
                    colour = brighten(HEAD, 0.35 + 0.25 * (self.now * 6.0).sin());
                }
                if inside {
                    colour = dim(lerp(colour, TUNNEL, 0.5), 0.75);
                }
                let glyph = if !self.look.ascii {
                    braille(bits)
                } else if col == head {
                    if self.open() { '<' } else { '-' }
                } else if col + 1 == head {
                    '@'
                } else if inside {
                    ':'
                } else {
                    let row = self.centre(col as f64 * 2.0).round().clamp(0.0, 3.0);
                    SCALES[row as usize]
                };
                paint.ink(&mut out, colour, false, glyph);
            }
        }
        out
    }
}

impl ProgressTracker for Wyrm {
    fn clone_box(&self) -> Box<dyn ProgressTracker> {
        Box::new(self.clone())
    }

    fn tick(&mut self, state: &ProgressState, _: Instant) {
        self.step(state.elapsed().as_secs_f64(), state.pos(), state.len());
    }

    fn reset(&mut self, _: &ProgressState, _: Instant) {
        *self = Self::new(self.look);
    }

    fn write(&self, _: &ProgressState, w: &mut dyn std::fmt::Write) {
        let _ = w.write_str(&self.draw());
    }
}

fn braille(bits: u32) -> char {
    char::from_u32(0x2800 + bits).unwrap_or(' ')
}

fn noise(x: usize, y: usize, tick: u64) -> f64 {
    let mut n = (x as u64)
        .wrapping_mul(374_761_393)
        .wrapping_add((y as u64).wrapping_mul(668_265_263))
        .wrapping_add(tick.wrapping_mul(2_147_483_647))
        & 0xFFFF_FFFF;
    n = ((n ^ (n >> 13)).wrapping_mul(1_274_126_177)) & 0xFFFF_FFFF;
    (n & 0xFFFF) as f64 / 65_535.0
}

fn lerp(a: Rgb, b: Rgb, t: f64) -> Rgb {
    (
        a.0 + (b.0 - a.0) * t,
        a.1 + (b.1 - a.1) * t,
        a.2 + (b.2 - a.2) * t,
    )
}

fn grad(stops: &[Rgb], t: f64) -> Rgb {
    let t = t.clamp(0.0, 1.0);
    let n = stops.len() - 1;
    let i = ((t * n as f64) as usize).min(n - 1);
    lerp(stops[i], stops[i + 1], t * n as f64 - i as f64)
}

fn brighten(c: Rgb, k: f64) -> Rgb {
    lerp(c, (255.0, 255.0, 255.0), k)
}

fn dim(c: Rgb, k: f64) -> Rgb {
    (c.0 * k, c.1 * k, c.2 * k)
}
