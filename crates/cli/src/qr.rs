use qrcode::{Color, EcLevel, QrCode, Version};

const QUIET: usize = 1;
const INK: &str = "\x1b[38;5;255m\x1b[48;5;232m";
const RESET: &str = "\x1b[0m";
const LIGHT: u8 = 255;
const DARK: u8 = 232;
const DRAKE_BUDGET_PERCENT: usize = 9;
const AURORA: [[u8; 3]; 4] = [
    [70, 40, 150],
    [60, 120, 230],
    [60, 210, 210],
    [190, 255, 220],
];
#[rustfmt::skip]
const DRAKES: [&[&str]; 3] = [
    &[
        ".......#.####..",
        "#......########",
        "##.....####....",
        "###...##.#####.",
        "####.###.......",
        ".#########.....",
        "..#######......",
        "#...##..##.....",
        ".###.#...#.....",
    ],
    &[
        ".......#.##",
        "#.....#####",
        "##....###..",
        "###..#####.",
        ".#######...",
        "..#####....",
        "#.#..#.#...",
    ],
    &[
        "....#.#",
        "#..####",
        "##.##..",
        ".####..",
        "..#.#..",
    ],
];

pub fn uri(code: &str) -> String {
    format!("wormhole-transfer:{code}")
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Module {
    Dark,
    Light,
    Drake(u8),
}

impl Module {
    fn lit(self) -> bool {
        self != Module::Dark
    }

    fn colour(self) -> u8 {
        match self {
            Module::Dark => DARK,
            Module::Light => LIGHT,
            Module::Drake(colour) => colour,
        }
    }
}

struct Grid {
    width: usize,
    modules: Vec<Module>,
}

impl Grid {
    fn at(&self, x: usize, y: usize) -> Module {
        self.modules[y * self.width + x]
    }
}

fn grid(code: &str) -> Grid {
    let data = uri(code);
    let qr = QrCode::with_error_correction_level(&data, EcLevel::H)
        .or_else(|_| QrCode::new(&data))
        .expect("a code fits in a QR code");
    let width = qr.width();
    let mut modules: Vec<Module> = qr
        .to_colors()
        .into_iter()
        .map(|c| match c {
            Color::Light => Module::Light,
            Color::Dark => Module::Dark,
        })
        .collect();
    let spot = match qr.error_correction_level() {
        EcLevel::H => drake_spot(&qr),
        _ => None,
    };
    if let Some((left, top, drake)) = spot {
        let (w, h) = (drake[0].len(), drake.len());
        for y in top..top + h + 2 {
            modules[y * width + left..y * width + left + w + 2].fill(Module::Dark);
        }
        for (y, row) in drake.iter().enumerate() {
            for (x, _) in row.bytes().enumerate().filter(|&(_, c)| c == b'#') {
                modules[(top + 1 + y) * width + left + 1 + x] = Module::Drake(aurora(x, w));
            }
        }
    }
    Grid { width, modules }
}

fn drake_spot(qr: &QrCode) -> Option<(usize, usize, &'static [&'static str])> {
    let width = qr.width();
    DRAKES.into_iter().find_map(|drake| {
        let (w, h) = (drake[0].len() + 2, drake.len() + 2);
        if w * h * 100 > width * width * DRAKE_BUDGET_PERCENT {
            return None;
        }
        let (cx, cy) = ((width - w) / 2, (width - h) / 2);
        let mut spots: Vec<_> = (0..=width - w)
            .flat_map(|x| (0..=width - h).map(move |y| (x, y)))
            .collect();
        spots.sort_by_key(|&(x, y)| x.abs_diff(cx).pow(2) + y.abs_diff(cy).pow(2));
        spots
            .into_iter()
            .find(|&(x, y)| (y..y + h).all(|j| (x..x + w).all(|i| !reserved(qr, i, j))))
            .map(|(x, y)| (x, y, drake))
    })
}

fn reserved(qr: &QrCode, x: usize, y: usize) -> bool {
    let edge = qr.width() - 11;
    let version_info = matches!(qr.version(), Version::Normal(v) if v >= 7)
        && ((x >= edge && y < 6) || (y >= edge && x < 6));
    version_info || qr.is_functional(x, y)
}

fn aurora(x: usize, width: usize) -> u8 {
    let segments = AURORA.len() - 1;
    let span = width - 1;
    let along = x * segments;
    let i = (along / span).min(segments - 1);
    let into = along - i * span;
    let channel = |c: usize| {
        let (from, to) = (usize::from(AURORA[i][c]), usize::from(AURORA[i + 1][c]));
        cube((from * (span - into) + to * into) / span)
    };
    16 + 36 * channel(0) + 6 * channel(1) + channel(2)
}

fn cube(value: usize) -> u8 {
    (0u8..)
        .zip([0usize, 95, 135, 175, 215, 255])
        .min_by_key(|&(_, level)| level.abs_diff(value))
        .map_or(0, |(i, _)| i)
}

fn tinted(top: Module, bottom: Module, ascii: bool) -> Option<String> {
    match (ascii, top, bottom) {
        (true, Module::Drake(colour), _) => Some(format!("\x1b[38;5;{colour}m##{INK}")),
        (true, ..) | (false, Module::Dark | Module::Light, Module::Dark | Module::Light) => None,
        (false, ..) => Some(format!(
            "\x1b[38;5;{}m\x1b[48;5;{}m▀{INK}",
            top.colour(),
            bottom.colour()
        )),
    }
}

pub fn render(code: &str, ascii: bool, coloured: bool) -> String {
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
    let mut out = String::new();
    let step = if ascii { 1 } else { 2 };
    for y in (0..side).step_by(step) {
        if coloured {
            out.push_str(INK);
        }
        for x in 0..side {
            let (top, bottom) = (at(x, y), at(x, y + 1));
            if coloured && let Some(cell) = tinted(top, bottom, ascii) {
                out.push_str(&cell);
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

    #[test]
    fn the_code_becomes_a_square_drawn_in_light_modules() {
        let ascii = render("7-guitarist-revenge", true, false);
        let lines: Vec<&str> = ascii.lines().collect();
        let side = lines.len();
        assert!(lines.iter().all(|l| l.len() == 2 * side));
        assert_eq!(lines[0], "##".repeat(side));
        assert_eq!(lines[side - 1], "##".repeat(side));
        assert!(lines[1].starts_with("##              ##"));

        let unicode = render("7-guitarist-revenge", false, false);
        let rows: Vec<&str> = unicode.lines().collect();
        assert_eq!(rows.len(), side.div_ceil(2));
        assert!(rows[0].starts_with("█▀▀▀▀▀▀▀█"));
        assert!(rows.iter().all(|r| r.chars().count() == side));
        assert!(rows.concat().chars().all(|c| " █▀▄".contains(c)));
        assert_ne!(
            render("7-guitarist-revenge", true, false),
            render("8-other", true, false)
        );
    }

    #[test]
    fn it_encodes_the_uri_the_python_client_does_and_colours_on_request() {
        assert_eq!(
            uri("7-guitarist-revenge"),
            "wormhole-transfer:7-guitarist-revenge"
        );
        let plain = render("7-a-b", false, false);
        let coloured = render("7-a-b", false, true);
        assert_eq!(coloured.lines().count(), plain.lines().count());
        let unchanged = coloured
            .lines()
            .zip(plain.lines())
            .filter(|(c, p)| *c == format!("{INK}{p}{RESET}"))
            .count();
        assert!(unchanged > 0 && unchanged < plain.lines().count());
    }
}
