use qrcode::{Color, QrCode};

const QUIET: usize = 1;
const INK: &str = "\x1b[38;5;255m\x1b[48;5;232m";
const RESET: &str = "\x1b[0m";

pub fn uri(code: &str) -> String {
    format!("wormhole-transfer:{code}")
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Module {
    Dark,
    Light,
}

impl Module {
    fn lit(self) -> bool {
        self != Module::Dark
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
    let qr = QrCode::new(uri(code)).expect("a code fits in a QR code");
    let modules = qr
        .to_colors()
        .into_iter()
        .map(|c| match c {
            Color::Light => Module::Light,
            Color::Dark => Module::Dark,
        })
        .collect();
    Grid {
        width: qr.width(),
        modules,
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
            out.push_str(match (ascii, at(x, y).lit(), at(x, y + 1).lit()) {
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
        for (c, p) in coloured.lines().zip(plain.lines()) {
            assert_eq!(c, format!("{INK}{p}{RESET}"));
        }
    }
}
