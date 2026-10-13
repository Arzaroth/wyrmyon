use std::fmt::Write as _;

use base64::Engine as _;
use qrcode::{Color, EcLevel, QrCode, Version};

mod logo;

const QUIET: usize = 1;
const INK: &str = "\x1b[38;5;255m\x1b[48;5;232m";
const RESET: &str = "\x1b[0m";
const DARK: u8 = 232;
const LIGHT: u8 = 255;
const LOGO_BUDGET_PERCENT: usize = 12;
const SMALLEST: i16 = 6;
const CHUNK: usize = 4096;

pub fn uri(code: &str) -> String {
    format!("wormhole-transfer:{code}")
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Look {
    Ascii,
    Plain,
    Coloured,
    Picture,
}

impl Look {
    pub fn choose(ascii: bool, terminal: bool, env: impl Fn(&str) -> Option<String>) -> Self {
        let set = |name: &str| env(name).is_some_and(|v| !v.is_empty());
        let kitty = env("TERM").is_some_and(|t| t.contains("kitty"))
            || env("TERM_PROGRAM").is_some_and(|p| matches!(p.as_str(), "ghostty" | "WezTerm"))
            || set("KITTY_WINDOW_ID");
        match (ascii, terminal) {
            (true, _) => Self::Ascii,
            (false, false) => Self::Plain,
            (false, true) if kitty && !set("TMUX") && !set("STY") => Self::Picture,
            (false, true) => Self::Coloured,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Module {
    Dark,
    Light,
    Logo(u8),
}

impl Module {
    fn lit(self) -> bool {
        match self {
            Module::Dark => false,
            Module::Light => true,
            Module::Logo(pixel) => bright(pixel),
        }
    }

    fn colour(self) -> u8 {
        match self {
            Module::Dark => DARK,
            Module::Light => LIGHT,
            Module::Logo(pixel) => pixel,
        }
    }
}

struct Grid {
    width: usize,
    modules: Vec<Module>,
    logo: Option<(usize, usize)>,
}

impl Grid {
    fn at(&self, x: usize, y: usize) -> Module {
        self.modules[y * self.width + x]
    }
}

fn encode(data: &str) -> QrCode {
    match QrCode::with_error_correction_level(data, EcLevel::H) {
        Ok(qr) if matches!(qr.version(), Version::Normal(v) if v < SMALLEST) => {
            QrCode::with_version(data, Version::Normal(SMALLEST), EcLevel::H)
                .expect("a short code fits the smallest version")
        }
        Ok(qr) => qr,
        Err(_) => QrCode::new(data).expect("a code fits in a QR code"),
    }
}

fn grid(code: &str) -> Grid {
    let qr = encode(&uri(code));
    let width = qr.width();
    let mut modules: Vec<Module> = qr
        .to_colors()
        .into_iter()
        .map(|c| match c {
            Color::Light => Module::Light,
            Color::Dark => Module::Dark,
        })
        .collect();
    let logo = match qr.error_correction_level() {
        EcLevel::H => logo_spot(&qr),
        _ => None,
    };
    if let Some((left, top)) = logo {
        for (y, row) in logo::PIXELS.iter().enumerate() {
            for (x, &pixel) in row.iter().enumerate().filter(|&(_, &p)| p != 0) {
                modules[(top + y) * width + left + x] = Module::Logo(pixel);
            }
        }
    }
    Grid {
        width,
        modules,
        logo,
    }
}

fn logo_spot(qr: &QrCode) -> Option<(usize, usize)> {
    let width = qr.width();
    let (w, h) = (logo::COLS, logo::ROWS * 2);
    if w * h * 100 > width * width * LOGO_BUDGET_PERCENT {
        return None;
    }
    let (cx, cy) = ((width - w) / 2, (width - h) / 2);
    let mut spots: Vec<_> = (0..=width - w)
        .flat_map(|x| (0..=width - h).map(move |y| (x, y)))
        .filter(|&(_, y)| (y + QUIET).is_multiple_of(2))
        .collect();
    spots.sort_by_key(|&(x, y)| x.abs_diff(cx).pow(2) + y.abs_diff(cy).pow(2));
    spots
        .into_iter()
        .find(|&(x, y)| (y..y + h).all(|j| (x..x + w).all(|i| !reserved(qr, i, j))))
}

fn reserved(qr: &QrCode, x: usize, y: usize) -> bool {
    let far = qr.width() - 8;
    let edge = qr.width() - 11;
    let version_info = matches!(qr.version(), Version::Normal(v) if v >= 7)
        && ((x >= edge && y < 6) || (y >= edge && x < 6));
    let finder = (y < 9 && (x < 9 || x >= far)) || (x < 9 && y >= far);
    version_info || finder || x == 6 || y == 6
}

fn picture() -> String {
    let data = base64::engine::general_purpose::STANDARD.encode(logo::PNG);
    let chunks: Vec<&[u8]> = data.as_bytes().chunks(CHUNK).collect();
    let mut out = String::new();
    for (i, chunk) in chunks.iter().enumerate() {
        let keys = if i == 0 {
            format!("a=T,f=100,t=d,q=2,C=1,c={},r={},", logo::COLS, logo::ROWS)
        } else {
            String::new()
        };
        let more = u8::from(i + 1 < chunks.len());
        let chunk = std::str::from_utf8(chunk).expect("base64 is ASCII");
        let _ = write!(out, "\x1b_G{keys}m={more};{chunk}\x1b\\");
    }
    out
}

fn bright(pixel: u8) -> bool {
    let level = |i: u8| [0u32, 95, 135, 175, 215, 255][usize::from(i)];
    let cube = pixel.saturating_sub(16);
    let (r, g, b) = (level(cube / 36), level(cube / 6 % 6), level(cube % 6));
    299 * r + 587 * g + 114 * b > 128_000
}

pub fn render(code: &str, look: Look) -> String {
    let grid = grid(code);
    let side = grid.width + 2 * QUIET;
    let at = |x: usize, y: usize| {
        let inside = |v: usize| (QUIET..grid.width + QUIET).contains(&v);
        match (inside(x) && inside(y), y < side) {
            (true, _) => grid.at(x - QUIET, y - QUIET),
            (false, true) => Module::Light,
            (false, false) => Module::Dark,
        }
    };
    let corner = grid.logo.map(|(left, top)| (left + QUIET, top + QUIET));
    let ascii = look == Look::Ascii;
    let coloured = matches!(look, Look::Coloured | Look::Picture);
    let mut out = String::new();
    for y in (0..side).step_by(if ascii { 1 } else { 2 }) {
        if coloured {
            out.push_str(INK);
        }
        for x in 0..side {
            let (top, bottom) = (at(x, y), at(x, y + 1));
            if look == Look::Picture && corner == Some((x, y)) {
                out.push_str(&picture());
            }
            let logo = matches!(top, Module::Logo(_)) || matches!(bottom, Module::Logo(_));
            if look == Look::Coloured && logo {
                let _ = write!(
                    out,
                    "\x1b[38;5;{}m\x1b[48;5;{}m▀{INK}",
                    top.colour(),
                    bottom.colour()
                );
                continue;
            }
            out.push_str(match (ascii, top.lit(), bottom.lit()) {
                (true, true, _) => "##",
                (true, false, _) => "  ",
                (false, true, true) => "█",
                (false, true, false) => "▀",
                (false, false, true) => "▄",
                (false, false, false) => " ",
            });
        }
        if coloured {
            out.push_str(RESET);
        }
        out.push('\n');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use wyrmyon_wormhole::code::{choose_words, completions};

    const SCALE: usize = 4;
    const MARGIN: usize = 4;

    fn in_ink(grid: &Grid, x: usize, y: usize) -> u8 {
        if grid.at(x, y) == Module::Light {
            255
        } else {
            0
        }
    }

    fn with_silhouette(grid: &Grid, x: usize, y: usize) -> u8 {
        if grid.at(x, y).lit() { 238 } else { 8 }
    }

    fn scanned(grid: &Grid, shade: fn(&Grid, usize, usize) -> u8) -> Vec<String> {
        let side = (grid.width + 2 * MARGIN) * SCALE;
        let inside = |v: usize| (MARGIN..grid.width + MARGIN).contains(&v);
        let mut image = rqrr::PreparedImage::prepare_from_greyscale(side, side, |x, y| {
            let (x, y) = (x / SCALE, y / SCALE);
            if inside(x) && inside(y) {
                shade(grid, x - MARGIN, y - MARGIN)
            } else {
                255
            }
        });
        image
            .detect_grids()
            .into_iter()
            .map(|found| found.decode().expect("the QR code decodes").1)
            .collect()
    }

    fn assert_scans(code: &str) {
        let grid = grid(code);
        for shade in [in_ink, with_silhouette] {
            assert_eq!(scanned(&grid, shade), [uri(code)], "{code}");
        }
    }

    fn longest_word(prefix: &str, words: usize) -> String {
        completions(prefix, words)
            .into_iter()
            .map(|c| c[prefix.len()..].to_owned())
            .max_by_key(String::len)
            .unwrap()
    }

    fn env(pairs: &'static [(&'static str, &'static str)]) -> impl Fn(&str) -> Option<String> {
        move |name| {
            pairs
                .iter()
                .find(|(key, _)| *key == name)
                .map(|(_, value)| (*value).to_owned())
        }
    }

    #[test]
    fn the_code_becomes_a_square_drawn_in_light_modules() {
        let ascii = render("7-guitarist-revenge", Look::Ascii);
        let lines: Vec<&str> = ascii.lines().collect();
        let side = lines.len();
        assert_eq!(side, 41 + 2 * QUIET);
        assert!(lines.iter().all(|l| l.len() == 2 * side));
        assert_eq!(lines[0], "##".repeat(side));
        assert_eq!(lines[side - 1], "##".repeat(side));
        assert!(lines[1].starts_with("##              ##"));

        let unicode = render("7-guitarist-revenge", Look::Plain);
        let rows: Vec<&str> = unicode.lines().collect();
        assert_eq!(rows.len(), side.div_ceil(2));
        assert!(rows[0].starts_with("█▀▀▀▀▀▀▀█"));
        assert!(rows.iter().all(|r| r.chars().count() == side));
        assert!(rows.concat().chars().all(|c| " █▀▄".contains(c)));
        assert_ne!(
            render("7-guitarist-revenge", Look::Ascii),
            render("8-other", Look::Ascii)
        );
    }

    #[test]
    fn it_encodes_the_uri_the_python_client_does_and_colours_on_request() {
        assert_eq!(
            uri("7-guitarist-revenge"),
            "wormhole-transfer:7-guitarist-revenge"
        );
        let plain = render("7-a-b", Look::Plain);
        let coloured = render("7-a-b", Look::Coloured);
        assert_eq!(coloured.lines().count(), plain.lines().count());
        let unchanged = coloured
            .lines()
            .zip(plain.lines())
            .filter(|(c, p)| *c == format!("{INK}{p}{RESET}"))
            .count();
        assert!(unchanged > 0 && unchanged < plain.lines().count());
        let (top, bottom) = (logo::PIXELS[6][4], logo::PIXELS[7][4]);
        assert!(coloured.contains(&format!("\x1b[38;5;{top}m\x1b[48;5;{bottom}m▀{INK}")));
        assert!(!plain.contains('\x1b'));
    }

    #[test]
    fn the_picture_rides_in_kitty_graphics_chunks_over_the_centre() {
        let sent = picture();
        let chunks: Vec<&str> = sent.split("\x1b\\").filter(|c| !c.is_empty()).collect();
        assert!(chunks[0].starts_with("\x1b_Ga=T,f=100,t=d,q=2,C=1,c=13,r=7,m=1;"));
        assert!(
            chunks[1..chunks.len() - 1]
                .iter()
                .all(|c| c.starts_with("\x1b_Gm=1;"))
        );
        assert!(chunks.last().unwrap().starts_with("\x1b_Gm=0;"));
        let data: String = chunks
            .iter()
            .map(|c| c.split_once(';').unwrap().1)
            .collect();
        assert!(
            chunks
                .iter()
                .all(|c| c.split_once(';').unwrap().1.len() <= CHUNK)
        );
        let png = base64::engine::general_purpose::STANDARD
            .decode(data)
            .unwrap();
        assert_eq!(png, logo::PNG);
        assert!(png.starts_with(b"\x89PNG"));

        let shown = render("7-guitarist-revenge", Look::Picture);
        assert_eq!(shown.matches("\x1b_Ga=T").count(), 1);
        let coloured_cells = |text: &str| text.matches("\x1b[48;5;").count();
        let coloured = render("7-guitarist-revenge", Look::Coloured);
        assert!(coloured_cells(&shown) < coloured_cells(&coloured));
        assert_eq!(
            shown.replace(&sent, "").lines().count(),
            coloured.lines().count()
        );
    }

    #[test]
    fn the_picture_needs_a_kitty_graphics_terminal_outside_a_multiplexer() {
        assert_eq!(
            Look::choose(true, true, env(&[("TERM", "xterm-kitty")])),
            Look::Ascii
        );
        assert_eq!(
            Look::choose(false, false, env(&[("TERM", "xterm-kitty")])),
            Look::Plain
        );
        for kitty in [
            &[("TERM", "xterm-kitty")][..],
            &[("TERM_PROGRAM", "ghostty")],
            &[("TERM_PROGRAM", "WezTerm")],
            &[("KITTY_WINDOW_ID", "1")],
        ] {
            assert_eq!(Look::choose(false, true, env(kitty)), Look::Picture);
        }
        for other in [
            &[("TERM", "xterm-256color")][..],
            &[("TERM_PROGRAM", "ghostty"), ("TMUX", "/tmp/tmux")],
            &[("TERM", "xterm-kitty"), ("STY", "1.pts")],
            &[("KITTY_WINDOW_ID", "")],
        ] {
            assert_eq!(Look::choose(false, true, env(other)), Look::Coloured);
        }
    }

    #[test]
    fn codes_of_every_length_scan_through_the_wyrm() {
        let (odd, even) = (longest_word("1-", 1), longest_word("1-a-", 2));
        for words in 1..=8 {
            let longest: Vec<&str> = (0..words)
                .map(|i| {
                    if i % 2 == 0 {
                        odd.as_str()
                    } else {
                        even.as_str()
                    }
                })
                .collect();
            let mut codes = vec![format!("4095-{}", longest.join("-"))];
            codes.extend((0..4).map(|n| format!("{}-{}", n * 37 + 1, choose_words(words))));
            for code in codes {
                assert!(grid(&code).logo.is_some(), "{code}");
                assert_scans(&code);
            }
        }
    }

    #[test]
    fn long_custom_codes_scan_and_the_longest_go_without_the_wyrm() {
        for len in [120, 300] {
            let code = format!("7-{}", "z".repeat(len));
            assert!(grid(&code).logo.is_some());
            assert_scans(&code);
        }
        let code = format!("7-{}", "z".repeat(1300));
        assert_eq!(grid(&code).logo, None);
        assert_scans(&code);
        assert!(!render(&code, Look::Picture).contains("\x1b_G"));
    }

    #[test]
    fn the_wyrm_spares_the_function_patterns_fits_the_budget_and_whole_text_rows() {
        let mut versions = Vec::new();
        for len in [1, 8, 17, 30, 41, 50, 70, 90, 120, 160, 300, 1000, 1250] {
            let code = "z".repeat(len);
            let qr = encode(&uri(&code));
            let width = qr.width();
            let grid = grid(&code);
            let (left, top) = grid.logo.unwrap();
            let (w, h) = (logo::COLS, logo::ROWS * 2);
            assert!(w * h * 100 <= width * width * LOGO_BUDGET_PERCENT);
            assert!((top + QUIET).is_multiple_of(2));
            let colors = qr.to_colors();
            for y in 0..width {
                for x in 0..width {
                    let cleared = (left..left + w).contains(&x) && (top..top + h).contains(&y);
                    let original = match colors[y * width + x] {
                        Color::Light => Module::Light,
                        Color::Dark => Module::Dark,
                    };
                    let pixel = if cleared {
                        assert!(!reserved(&qr, x, y));
                        logo::PIXELS[y - top][x - left]
                    } else {
                        0
                    };
                    let expected = if pixel == 0 {
                        original
                    } else {
                        Module::Logo(pixel)
                    };
                    assert_eq!(grid.at(x, y), expected);
                }
            }
            let (Version::Normal(version) | Version::Micro(version)) = qr.version();
            versions.push(version);
        }
        assert_eq!(versions, [6, 6, 6, 6, 7, 8, 9, 10, 12, 14, 19, 36, 40]);
    }
}
