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
    use wyrmyon_wormhole::code::{choose_words, completions};

    const SCALE: usize = 4;
    const MARGIN: usize = 4;

    fn level(index: u8) -> u32 {
        [0, 95, 135, 175, 215, 255][usize::from(index)]
    }

    fn on_screen(module: Module) -> u8 {
        let rgb = match module.colour() {
            DARK => [8, 8, 8],
            LIGHT => [238, 238, 238],
            cube => {
                let i = cube - 16;
                [level(i / 36), level(i / 6 % 6), level(i % 6)]
            }
        };
        u8::try_from((299 * rgb[0] + 587 * rgb[1] + 114 * rgb[2]) / 1000).unwrap()
    }

    fn in_ink(module: Module) -> u8 {
        if module.lit() { 255 } else { 0 }
    }

    fn scanned(grid: &Grid, shade: fn(Module) -> u8) -> Vec<String> {
        let side = (grid.width + 2 * MARGIN) * SCALE;
        let inside = |v: usize| (MARGIN..grid.width + MARGIN).contains(&v);
        let mut image = rqrr::PreparedImage::prepare_from_greyscale(side, side, |x, y| {
            let (x, y) = (x / SCALE, y / SCALE);
            if inside(x) && inside(y) {
                shade(grid.at(x - MARGIN, y - MARGIN))
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

    fn drake_cells(grid: &Grid) -> usize {
        grid.modules
            .iter()
            .filter(|m| matches!(m, Module::Drake(_)))
            .count()
    }

    fn assert_scans(code: &str) {
        let grid = grid(code);
        for shade in [on_screen, in_ink] {
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

    #[test]
    fn codes_of_every_length_scan_through_the_drake() {
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
                assert!(drake_cells(&grid(&code)) > 0, "{code}");
                assert_scans(&code);
            }
        }
    }

    #[test]
    fn long_custom_codes_scan_and_the_longest_go_without_a_drake() {
        for len in [120, 300] {
            let code = format!("7-{}", "z".repeat(len));
            assert!(drake_cells(&grid(&code)) > 0);
            assert_scans(&code);
        }
        let code = format!("7-{}", "z".repeat(1300));
        assert_eq!(drake_cells(&grid(&code)), 0);
        assert_scans(&code);
    }

    #[test]
    fn the_drake_spares_the_function_patterns_and_fits_the_budget() {
        let mut sizes = Vec::new();
        for len in [1, 8, 17, 30, 41, 50, 70, 90, 120, 160, 300, 1000, 1250] {
            let data = uri(&"z".repeat(len));
            let qr = QrCode::with_error_correction_level(&data, EcLevel::H).unwrap();
            let width = qr.width();
            let (left, top, drake) = drake_spot(&qr).unwrap();
            let (w, h) = (drake[0].len() + 2, drake.len() + 2);
            assert!(w * h * 100 <= width * width * DRAKE_BUDGET_PERCENT);
            let grid = grid(&"z".repeat(len));
            let colors = qr.to_colors();
            for y in 0..width {
                for x in 0..width {
                    let cleared = (left..left + w).contains(&x) && (top..top + h).contains(&y);
                    let original = match colors[y * width + x] {
                        Color::Light => Module::Light,
                        Color::Dark => Module::Dark,
                    };
                    if cleared {
                        assert!(!reserved(&qr, x, y) && !qr.is_functional(x, y));
                        assert_ne!(grid.at(x, y), Module::Light);
                    } else {
                        assert_eq!(grid.at(x, y), original);
                    }
                }
            }
            let (Version::Normal(version) | Version::Micro(version)) = qr.version();
            sizes.push((version, drake[0].len()));
        }
        assert_eq!(
            sizes,
            [
                (3, 7),
                (4, 7),
                (5, 11),
                (6, 11),
                (7, 11),
                (8, 15),
                (9, 15),
                (10, 15),
                (12, 15),
                (14, 15),
                (19, 15),
                (36, 15),
                (40, 15)
            ]
        );
    }

    #[test]
    fn the_drake_is_drawn_in_an_aurora_from_tail_to_head() {
        let violet = 16 + 36 * cube(70) + 6 * cube(40) + cube(150);
        let mint = 16 + 36 * cube(190) + 6 * cube(255) + cube(220);
        assert_eq!((aurora(0, 15), aurora(14, 15)), (violet, mint));
        assert_eq!(
            aurora(7, 15),
            16 + 36 * cube(60) + 6 * cube(165) + cube(220)
        );

        let code = "7-guitarist-revenge";
        let grid = grid(code);
        let colours: Vec<u8> = grid.modules.iter().map(|m| m.colour()).collect();
        assert!(colours.contains(&violet) && colours.contains(&mint));

        let half = render(code, false, true);
        assert!(half.contains(&format!("\x1b[48;5;{mint}m▀{INK}")));
        assert!(half.contains(&format!("\x1b[38;5;{DARK}m\x1b[48;5;{violet}m▀{INK}")));
        let ascii = render(code, true, true);
        assert!(ascii.contains(&format!("\x1b[38;5;{violet}m##{INK}")));

        let plain = render(code, true, false);
        let lit = plain.matches("##").count();
        let light = (0..grid.width)
            .flat_map(|y| (0..grid.width).map(move |x| (x, y)))
            .filter(|&(x, y)| grid.at(x, y).lit())
            .count();
        assert_eq!(lit, light + 4 * (grid.width + 1));
        assert!(!plain.contains('\x1b') && !render(code, false, false).contains('\x1b'));
    }
}
